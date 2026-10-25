//! File Parser — 带魔法数字嗅探的智能解析路由
//!
//! 策略:
//!   1. 读取文件前 4096 字节做 magic number 嗅探
//!   2. 嗅探命中明确格式 → 直接路由
//!   3. 无后缀 + 嗅探失败 → 轮询兜底（PDF→DOCX→XLSX→TXT）
//!   4. 图片 → base64 → 视觉模型 OCR
//!   5. PDF → OCR 优先（PDF→图片→视觉API），失败退 pdf-extract

use std::io::Read;
use crate::database;
use base64::Engine;
use regex::Regex;
use serde_json::{json, Value};
use once_cell::sync::Lazy;

/// 共享视觉模型 HTTP 客户端（复用连接池，避免逐页重建 reqwest::Client）
static VISION_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .expect("视觉模型 HTTP 客户端创建失败")
});

/// 知识库视觉请求全局闸：跨文件封顶在途请求数（文件内并发 3 × 最多 5 文件 = 15 路需求，
/// 由此闸背压到 10），防供应商 429 限流。覆盖 PDF 逐页 OCR、整图 OCR、chat 文档附件解析。
static VISION_SEMAPHORE: Lazy<tokio::sync::Semaphore> = Lazy::new(|| tokio::sync::Semaphore::new(10));

/// 聊天图片描述独立通道：不与知识库批量解析排队，KB 高峰时发图仍秒级响应
static CHAT_VISION_SEMAPHORE: Lazy<tokio::sync::Semaphore> = Lazy::new(|| tokio::sync::Semaphore::new(2));

#[derive(Default)]
pub struct ParseResult { pub text: String, pub success: bool, pub error: String, pub inferred_ext: Option<String> }

// ═══════════════════════════════════════════════════════════════
// OCR via Vision API
// ═══════════════════════════════════════════════════════════════

/// 统一默认提示词
const VISION_DEFAULT_PROMPT: &str = r#"你是一个精确的文档提取助手。从图片提取内容，输出紧凑、完整、便于检索的 Markdown。

## 硬性约束
1. 只输出提取内容本身，禁止任何开场白、结束语、解释、评论或与文档无关的文字。
2. 同一信息在文档中重复出现时（如每页重复的公司名、口号、免责声明）只保留第一次。

## 输出规则
3. 标题：保留原文层级（# ## ###），文字尽量精炼。
4. 表格：一律标准 Markdown 表格。首行必须是表头（列名=型号或维度），第二行必须是对齐分隔行；型号作表头列，不在数据中重复标注；禁止合并/嵌套单元格；相同值逐格填全，禁止「同上/同XXX」；单元格多值用英文逗号分隔。
5. 数值与单位：保留所有数值、型号、编号与参数，必须携带原始单位（GB、Gbps、℃），数字与单位间空格（256 GB、2U）。
6. 噪声：跳过页码、水印、页眉页脚、装饰图标。
7. 模糊：看不清用 [?]，不猜测。
8. 双栏：先完整提取第一栏再第二栏，禁止混排。
9. 压缩：输出短于原文，删冗余修饰语与空行，但不得省略任何数值、型号、编号与技术参数。"#;

/// 视觉模型配置
pub struct VisionConfig {
    base_url: String,
    api_key: String,
    model: String,
    api_format: String, // "openai" or "dashscope"
}

/// 视觉模型适配器：每个 api_format + model_name 定义自己的请求构建、响应解析和文本后处理
trait VisionAdapter: Send + Sync {
    fn build_request(&self, cfg: &VisionConfig, base64_data_uri: &str, prompt: &str) -> Value;
    fn parse_response(&self, body: &Value) -> String;
    /// 模型专属输出文本归一化（在 parse_response 之后调用）
    fn post_process(&self, text: &str) -> String {
        text.to_string() // default: no-op
    }
}

struct OpenaiAdapter;
impl VisionAdapter for OpenaiAdapter {
    fn build_request(&self, cfg: &VisionConfig, base64_data_uri: &str, prompt: &str) -> Value {
        json!({
            "model": cfg.model,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "image_url", "image_url": {"url": base64_data_uri}},
                    {"type": "text", "text": prompt}
                ]
            }],
            "max_tokens": 4096,
            "temperature": 0.0
        })
    }
    fn parse_response(&self, body: &Value) -> String {
        body["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string()
    }
}

struct DashscopeAdapter;
impl VisionAdapter for DashscopeAdapter {
    fn build_request(&self, cfg: &VisionConfig, base64_data_uri: &str, prompt: &str) -> Value {
        json!({
            "model": cfg.model,
            "input": {
                "messages": [{
                    "role": "user",
                    "content": [
                        {"image": base64_data_uri},
                        {"text": prompt}
                    ]
                }]
            },
            "parameters": {"max_tokens": 4096}
        })
    }
    fn parse_response(&self, body: &Value) -> String {
        body["output"]["choices"][0]["message"]["content"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|v| v["text"].as_str())
            .unwrap_or("")
            .to_string()
    }
}

fn is_qwen3(model_name: &str) -> bool {
    let m = model_name.to_lowercase();
    m.contains("qwen3-vl") || m.contains("qwenvl3")
}

/// Qwen3-VL 适配器：与 Dashscope 同格式，但输出文本含混合行需拆分
struct Qwen3VlAdapter;
/// Split inline heading chains like "## A ### B #### C" → ["## A", "### B", "#### C"]
fn split_heading_chain(line: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let mut start = 0;
    for i in 1..chars.len() {
        if chars[i-1] == ' ' && chars[i] == '#' {
            let prev = line[start..i].trim();
            if !prev.is_empty() {
                parts.push(prev.to_string());
            }
            start = i;
        }
    }
    let remaining = line[start..].trim();
    if !remaining.is_empty() {
        parts.push(remaining.to_string());
    }
    if parts.is_empty() { parts.push(line.to_string()); }
    parts
}

impl VisionAdapter for Qwen3VlAdapter {
    fn build_request(&self, cfg: &VisionConfig, base64_data_uri: &str, prompt: &str) -> Value {
        // 与 Dashscope 相同
        json!({
            "model": cfg.model,
            "input": {
                "messages": [{
                    "role": "user",
                    "content": [
                        {"image": base64_data_uri},
                        {"text": prompt}
                    ]
                }]
            },
            "parameters": {"max_tokens": 4096}
        })
    }
    fn parse_response(&self, body: &Value) -> String {
        // 与 Dashscope 相同
        body["output"]["choices"][0]["message"]["content"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|v| v["text"].as_str())
            .unwrap_or("")
            .to_string()
    }

    fn post_process(&self, text: &str) -> String {
        // Step 1: strip wrapping ```markdown / ``` fences
        let mut lines: Vec<&str> = text.lines().collect();
        if !lines.is_empty() && lines[0].trim().starts_with("```") {
            lines.remove(0);
        }
        if !lines.is_empty() && lines.last().map(|l| l.trim().starts_with("```")).unwrap_or(false) {
            lines.pop();
        }

        let mut out: Vec<String> = Vec::new();
        for raw_line in &lines {
            let line = raw_line.trim();
            if line.is_empty() { out.push(String::new()); continue; }

            // Split inline heading chains first
            let fragments = split_heading_chain(line);

            for frag in &fragments {
                // Check for heading + <table / | mix
                if (frag.starts_with('#') || frag.starts_with("##"))
                    && (frag.contains("<table") || frag.contains('|'))
                {
                    let cut = frag.find("<table").or_else(|| frag.find('|')).unwrap_or(frag.len());
                    let head = frag[..cut].trim();
                    let rest = frag[cut..].trim();
                    if head.starts_with('#') || head.starts_with("##") {
                        out.push(head.to_string());
                        out.push(rest.to_string());
                        continue;
                    }
                }

                // Heading + dash content: "### 全可编程架构 - CloudEngine..."
                if (frag.starts_with('#') || frag.starts_with("##") || frag.starts_with("###"))
                    && frag.contains(" - ") && !frag.contains("<table") && !frag.contains('|')
                {
                    let dash_pos = frag.find(" - ").unwrap();
                    let heading_part = frag[..dash_pos].trim().to_string();
                    let content_part = frag[dash_pos..].trim().to_string();
                    if heading_part.chars().count() <= 40 && !content_part.contains('#') {
                        out.push(heading_part);
                        out.push(content_part);
                        continue;
                    }
                }

                out.push(frag.to_string());
            }
        }
        out.join("\n")
    }
}

fn get_adapter(api_format: &str, model_name: &str) -> Box<dyn VisionAdapter + Send + Sync> {
    match api_format {
        "dashscope" if is_qwen3(model_name) => Box::new(Qwen3VlAdapter),
        "dashscope" => Box::new(DashscopeAdapter),
        _ => Box::new(OpenaiAdapter),
    }
}

/// 从 ai_config 读取视觉模型配置（visionProviders + activeVisionId）
pub fn get_vision_config() -> Option<VisionConfig> {
    let row = database::get(
        "SELECT ai_config FROM system_config WHERE tenant_id='local-user'",
        &[],
    ).ok().flatten()?;
    let s = row["ai_config"].as_str()?;
    let cfg: Value = serde_json::from_str(s).ok()?;
    let machine = crate::crypto::get_machine_hash();
    let cfg = crate::crypto::decrypt_config_api_keys(&cfg, &machine);
    let providers = cfg["visionProviders"].as_array()?;
    let active_id = cfg["activeVisionId"].as_str()?;
    let provider = providers.iter().find(|p| p["id"].as_str() == Some(active_id))?;
    let base_url = provider["baseUrl"].as_str()?.to_string();
    let api_key = provider["apiKey"].as_str()?.to_string();
    let model = provider["modelName"].as_str()?.to_string();
    let api_format = provider["apiFormat"].as_str().unwrap_or("openai").to_string();
    Some(VisionConfig { base_url, api_key, model, api_format })
}

/// 读取文件为 base64 编码的 data URI
pub(crate) fn file_to_data_uri(path: &str, mime: &str) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&buf);
    Some(format!("data:{};base64,{}", mime, b64))
}

/// 调用视觉模型 API 进行 OCR（异步）
async fn ocr_with_vision_api(base64_data_uri: &str) -> Result<String, String> {
    let cfg = get_vision_config().ok_or("未配置视觉模型，请在设置页配置视觉模型")?;
    // 全局闸：限制知识库路径跨文件在途视觉请求总数（重试间不持 permit，退避 sleep 在调用方）
    let _permit = VISION_SEMAPHORE.acquire().await.ok();
    let adapter = get_adapter(&cfg.api_format, &cfg.model);

    // 复用共享客户端（避免每页重建 reqwest::Client 的开销）
    let client = &*VISION_CLIENT;

    let body = adapter.build_request(&cfg, base64_data_uri, VISION_DEFAULT_PROMPT);

    // 自动拼接 /chat/completions（与 LLM 请求和测试连接保持一致）
    let url = if cfg.base_url.contains("/v1/") || cfg.base_url.ends_with("/chat/completions") {
        cfg.base_url.clone()
    } else {
        format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'))
    };
    let mut req = client.post(&url).json(&body);
    if !cfg.api_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", cfg.api_key));
    }

    let t_send = std::time::Instant::now();
    let resp = req.send().await.map_err(|e| format!("请求失败: {}", e))?;
    let status = resp.status();
    let send_ms = t_send.elapsed().as_millis();
    let t_body = std::time::Instant::now();
    let resp_text = resp.text().await.map_err(|e| format!("读取响应失败: {}", e))?;
    let body_ms = t_body.elapsed().as_millis();
    let t_parse = std::time::Instant::now();
    let resp_body: Value = serde_json::from_str(&resp_text).map_err(|_| {
        format!("响应不是JSON: {} (HTTP {})", crate::preview(&resp_text, 500), status)
    })?;

    if !status.is_success() {
        let err_detail = resp_body["error"]["message"].as_str().map(|s| s.to_string())
            .or_else(|| resp_body["error"].as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| resp_text.chars().take(500).collect());
        return Err(format!("API 错误 (HTTP {}): {}", status, err_detail));
    }

    let text = adapter.parse_response(&resp_body);
    // ★ 模型专属后处理（格式归一化）
    let text = adapter.post_process(&text);
    if text.is_empty() {
        return Err("OCR 返回空内容".into());
    }
    // 后处理：去除版面分析的标记符号和坐标格式
    // 匹配 pattern: any_text[[x, y, w, h]] 或 <|ref|>xxx<|/ref|>
    let cleaned = Regex::new(r"<\|/?[a-z]+\|>").unwrap()
        .replace_all(&text, "")
        .to_string();
    let cleaned = Regex::new(r"\b[a-z_]+\s*\[\[\s*\d+\s*,\s*\d+\s*,\s*\d+\s*,\s*\d+\s*\]\]")
        .unwrap()
        .replace_all(&cleaned, "")
        .to_string();
    // 检测清洗后是否还有实际文字内容（中文或英文字母）
    let has_real_content = cleaned.chars().any(|c| c.is_ascii_alphanumeric() || c as u32 > 127);
    if !has_real_content {
        return Err("OCR 模型仅返回版面坐标，未提取到文字内容，请检查视觉模型是否正确".into());
    }
    let cleaned = cleaned.lines()
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if cleaned.trim().is_empty() {
        return Err("OCR 返回的内容清洗后为空".into());
    }
    log::info!(
        "[文件解析] 视觉API: send={}ms body={}ms process={}ms 状态={} 文本={}字符",
        send_ms, body_ms, t_parse.elapsed().as_millis(), status, cleaned.len()
    );
    Ok(cleaned)
}

/// Chat 场景图片描述提示词（比 OCR 提示词更通用，支持截图、照片、图表等）
const CHAT_DESCRIBE_PROMPT: &str = r#"请详细描述这张图片的内容。如果图片中包含文字、表格、图表或数据，请完整提取；如果是物体、场景或界面截图，请描述关键信息。输出简洁的中文 Markdown。"#;

/// Chat 场景图片描述（调用视觉模型，用通用提示词而非文档提取提示词）
pub async fn describe_image_for_chat(base64_data_uri: &str) -> Result<String, String> {
    let cfg = get_vision_config().ok_or("未配置视觉模型，请在设置页配置视觉模型")?;
    // 独立小通道：不排在知识库批量 OCR 后面（KB 全局闸 10 满时也只需等本通道 2 permits）
    let _permit = CHAT_VISION_SEMAPHORE.acquire().await.ok();
    let adapter = get_adapter(&cfg.api_format, &cfg.model);
    let client = &*VISION_CLIENT;
    let body = adapter.build_request(&cfg, base64_data_uri, CHAT_DESCRIBE_PROMPT);
    let url = if cfg.base_url.contains("/v1/") || cfg.base_url.ends_with("/chat/completions") {
        cfg.base_url.clone()
    } else {
        format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'))
    };
    let mut req = client.post(&url).json(&body);
    if !cfg.api_key.is_empty() { req = req.header("Authorization", format!("Bearer {}", cfg.api_key)); }
    let resp = req.send().await.map_err(|e| format!("请求失败: {}", e))?;
    let status = resp.status();
    let resp_text = resp.text().await.map_err(|e| format!("读取响应失败: {}", e))?;
    let resp_body: Value = serde_json::from_str(&resp_text).map_err(|_| {
        format!("响应不是JSON: {} (HTTP {})", crate::preview(&resp_text, 500), status)
    })?;
    if !status.is_success() {
        let err_detail = resp_body["error"]["message"].as_str().map(|s| s.to_string())
            .or_else(|| resp_body["error"].as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| resp_text.chars().take(500).collect());
        return Err(format!("API 错误 (HTTP {}): {}", status, err_detail));
    }
    let text = adapter.parse_response(&resp_body);
    let text = adapter.post_process(&text);
    if text.trim().is_empty() { return Err("图片描述模型返回空内容".into()); }
    Ok(text.trim().to_string())
}

/// 判断扩展名是否为图片格式
fn is_image_ext(ext: &str) -> bool {
    matches!(ext.to_lowercase().as_str(), "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif")
}

/// 单个附件解析后文本长度上限
pub const ATTACH_TEXT_CAP: usize = 16000;

/// 解析单个附件文件：文档走 parse_file_smart，图片走视觉模型描述
/// 返回 (kind, text)，kind 为 "image" 或 "doc"
/// 文档附件默认不截断（完整返回），图片描述保持 ATTACH_TEXT_CAP 上限
pub async fn parse_chat_attachment(path: &str, ext: &str) -> Result<(String, String), String> {
    let ext_lower = ext.to_lowercase();
    if is_image_ext(&ext_lower) {
        let mime = match ext_lower.as_str() {
            "png" => "image/png", "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp", "bmp" => "image/bmp", "gif" => "image/gif",
            _ => "image/png",
        };
        let data_uri = file_to_data_uri(path, mime).ok_or_else(|| format!("无法读取图片文件: {}", path))?;
        let text = describe_image_for_chat(&data_uri).await?;
        let truncated = truncate_text(&text, ATTACH_TEXT_CAP);
        Ok(("image".to_string(), truncated))
    } else {
        // 文档：用 spawn_blocking 包装同步解析，完整返回不截断
        let path_owned = path.to_string();
        let result = tokio::task::spawn_blocking(move || {
            let _guard = BlockingThreadEnter::new();
            parse_file_smart(&path_owned, &ext_lower)
        })
            .await
            .map_err(|e| format!("解析任务失败: {}", e))?;
        if !result.success && result.error.is_empty() {
            return Err(format!("文件解析失败或内容为空: {}", path));
        }
        let text = if result.success { result.text } else { result.error };
        Ok(("doc".to_string(), text))
    }
}

/// 结构感知截断：优先在表格行（</tr>）边界截断，其次段落边界，最后字符边界
/// 保留表头，截断后提示剩余内容可追问
pub fn truncate_at_boundary(text: &str, cap: usize) -> String {
    if text.len() <= cap { return text.to_string(); }
    // UTF-8 安全：cap 可能落在多字节字符中间，向前找安全边界
    let safe_cap = {
        let mut c = cap;
        while c > 0 && !text.is_char_boundary(c) { c -= 1; }
        c
    };
    let head = &text[..safe_cap];
    // 1. 优先在表格行结束标签边界截断
    if let Some(pos) = head.rfind("</tr>") {
        let cut = pos + "</tr>".len();
        let kept_rows = text[..cut].matches("<tr>").count();
        let total_rows = text.matches("<tr>").count();
        return format!(
            "{}\n\n（表格内容较长，已展示前 {} / {} 行）",
            &text[..cut], kept_rows, total_rows
        );
    }
    // 2. 其次在段落/换行边界截断
    if let Some(pos) = head.rfind('\n') {
        return format!(
            "{}\n\n（内容较长，已截断）",
            &text[..pos]
        );
    }
    // 3. 最后按字符边界截断
    format!(
        "{}\n\n（内容较长，已截断，可追问剩余内容）",
        &text[..safe_cap]
    )
}

/// 截断文本，超出时附注截断提示
fn truncate_text(text: &str, cap: usize) -> String {
    if text.len() <= cap { return text.to_string(); }
    let mut end = cap;
    // 在字符边界截断（UTF-8 安全）
    while end < text.len() && !text.is_char_boundary(end) { end += 1; }
    let total = text.len();
    format!("{}\n\n（内容已截断，原文共 {} 字）", &text[..end], total)
}

/// PDF 每页渲染为 PNG 图片 → Vec<base64 data URI>（200 DPI, 全部页面）
/// 渲染 PDF 全部页面为 base64 图片
///   pdfium → pdftoppm
fn pdf_to_images_base64(path: &str) -> Option<Vec<String>> {
    // 方法1：pdfium（跨平台，DLL/dylib 已捆绑）
    let result = try_pdfium_render(path);
    if result.is_some() { return result; }

    // 方法2：macOS 使用系统自带的 sips（无需安装）
    #[cfg(target_os = "macos")]
    {
        let result = try_macos_sips(path);
        if result.is_some() { return result; }
    }

    // 方法3：尝试系统 pdftoppm 命令（poppler 工具集）
    let result = try_pdftoppm(path);
    if result.is_some() { return result; }

    log::info!("[文件解析] 所有 PDF 渲染方法均不可用");
    None
}

fn try_load_pdfium() -> Option<pdfium_render::prelude::Pdfium> {
    use pdfium_render::prelude::*;
    // 静态链接：Pdfium::default() 自动使用编译时链接的二进制
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Pdfium::default())) {
        Ok(p) => return Some(p),
        Err(_) => log::info!("[文件解析] PDFium 静态链接加载失败"),
    }
    None
}

/// 获取 PDF 总页数
pub fn get_pdf_page_count(path: &str) -> Option<u16> {
    let pdfium = try_load_pdfium()?;
    let doc = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pdfium.load_pdf_from_file(path, None)))
        .ok()?.ok()?;
    Some(doc.pages().len())
}

/// 单页预渲染结果
pub enum PageRender {
    /// base64 data URI（96 DPI JPEG q55，与原逐页渲染参数一致）
    Image(String),
    /// 近纯白页（本地像素预检命中，无需发视觉模型）
    Blank,
    /// 页面缺失 / 渲染失败 / 渲染 panic
    Failed,
}

/// 近空白位图判定：RGB 步长采样（≤3000 点），白色像素（R/G/B 均 ≥250）占比 ≥99.5% 视为空白。
/// 阈值刻意保守：96 DPI 正文页墨量通常 3~8% 不会误杀；灰底/彩色设计页照常送 VLM。
pub fn is_near_blank_rgb(rgb: &[u8]) -> bool {
    let total_px = rgb.len() / 3;
    if total_px == 0 { return true; }
    let stride = (total_px / 3000).max(1);
    let mut samples = 0usize;
    let mut white = 0usize;
    let mut i = 0;
    while i < total_px {
        let px = &rgb[i * 3..i * 3 + 3];
        samples += 1;
        if px[0] >= 250 && px[1] >= 250 && px[2] >= 250 { white += 1; }
        i += stride;
    }
    samples > 0 && white * 200 >= samples * 199 // ≥ 99.5%
}

/// 批量渲染 PDF 指定页为 data URI（文档一次 load，消除逐页重复加载全文档的 IO）。
/// 返回 None = 文档整体加载失败；Some(vec) 按入参页序，页级失败以 PageRender::Failed 标记。
pub fn render_pdf_pages(path: &str, pages: &[u16]) -> Option<Vec<(u16, PageRender)>> {
    let t0 = std::time::Instant::now();
    let pdfium = try_load_pdfium()?;
    let doc = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pdfium.load_pdf_from_file(path, None)))
        .ok()?.ok()?;
    let t_load = t0.elapsed().as_millis();
    let doc_pages = doc.pages();
    let mut results = Vec::with_capacity(pages.len());
    for &p in pages {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            render_one_page(&doc_pages, p)
        })).unwrap_or(PageRender::Failed);
        results.push((p, outcome));
    }
    log::info!("[文件解析] 批量渲染 {} 页: load={}ms 总耗时={}ms", pages.len(), t_load, t0.elapsed().as_millis());
    Some(results)
}

fn render_one_page(doc_pages: &pdfium_render::prelude::PdfPages, page_num: u16) -> PageRender {
    let page = match doc_pages.get(page_num) { Ok(p) => p, Err(_) => return PageRender::Failed };
    let scale = 96.0 / 72.0;
    let width = (page.width().value * scale) as i32;
    let height = (page.height().value * scale) as i32;
    let bitmap = match page.render(width, height, None) { Ok(b) => b, Err(_) => return PageRender::Failed };

    // pdfium 输出 BGRA 格式，需要转换为 RGB
    let bgra = bitmap.as_raw_bytes();
    let rgb: Vec<u8> = bgra.chunks(4)
        .flat_map(|c| [c[2], c[1], c[0]])  // BGRA → RGB
        .collect();
    if is_near_blank_rgb(&rgb) { return PageRender::Blank; }
    let img = match image::RgbImage::from_raw(width as u32, height as u32, rgb) { Some(i) => i, None => return PageRender::Failed };

    let mut jpeg_buf = std::io::Cursor::new(Vec::new());
    // JPEG 质量降到 55，减小发给视觉模型的 payload（对 OCR 精度影响小）
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_buf, 55);
    if encoder.encode_image(&image::DynamicImage::ImageRgb8(img)).is_err() { return PageRender::Failed; }
    let b64 = base64::engine::general_purpose::STANDARD.encode(jpeg_buf.into_inner());
    PageRender::Image(format!("data:image/jpeg;base64,{}", b64))
}

/// 通过 pdfium-render 提取每页纯文本
pub fn try_pdfium_text_per_page(path: &str) -> Option<Vec<String>> {
    let pdfium = try_load_pdfium()?;
    let doc = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pdfium.load_pdf_from_file(path, None)))
        .ok()?.ok()?;
    let pages = doc.pages();
    let count = pages.len();
    if count == 0 { return None; }
    let mut result: Vec<String> = Vec::with_capacity(count as usize);
    for i in 0..count {
        if let Ok(page) = pages.get(i) {
            if let Ok(text) = page.text() {
                result.push(text.to_string().trim().to_string());
                continue;
            }
        }
        result.push(String::new());
    }
    Some(result)
}

/// 通过 pdfium-render 渲染 PDF 全部页面为 base64 图片（150 DPI，JPEG 格式）
fn try_pdfium_render(path: &str) -> Option<Vec<String>> {
    let pdfium = try_load_pdfium()?;

    let doc = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pdfium.load_pdf_from_file(path, None)))
        .ok()?.ok()?;
    let pages = doc.pages();
    let count = pages.len();
    if count == 0 { return None; }

    let scale = 150.0 / 72.0;
    // OCR 页数上限：从渲染源头截断（150DPI 全量内存渲染的峰值封顶在 ~90MB/300 页）
    let render_count = count.min(MAX_OCR_PAGES);
    let mut results = Vec::with_capacity(render_count as usize);

    for i in 0..render_count {
        let page = match pages.get(i) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let width = (page.width().value * scale) as i32;
        let height = (page.height().value * scale) as i32;
        let bitmap = match page.render(width, height, None) {
            Ok(b) => b,
            Err(_) => continue,
        };
        // pdfium 输出 BGRA 格式，需要转换为 RGB
        let bgra = bitmap.as_raw_bytes();
        let rgb: Vec<u8> = bgra.chunks(4)
            .flat_map(|c| [c[2], c[1], c[0]])  // BGRA → RGB
            .collect();
        let img = match image::RgbImage::from_raw(width as u32, height as u32, rgb) {
            Some(i) => i,
            None => continue,
        };
        let mut jpeg_buf = std::io::Cursor::new(Vec::new());
        if img.write_to(&mut jpeg_buf, image::ImageFormat::Jpeg).is_err() { continue; }
        let b64 = base64::engine::general_purpose::STANDARD.encode(jpeg_buf.into_inner());
        results.push(format!("data:image/jpeg;base64,{}", b64));
    }
    if results.is_empty() { None } else { Some(results) }
}

/// 通过系统 pdftoppm 命令渲染 PDF（poppler 工具集，跨平台）
fn try_pdftoppm(path: &str) -> Option<Vec<String>> {
    let cmd = if cfg!(target_os = "windows") { "pdftoppm.exe" } else { "pdftoppm" };
    let tmp_dir = std::env::temp_dir().join(format!("arkmind_ppm_{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()));
    std::fs::create_dir_all(&tmp_dir).ok()?;

    let output_prefix = tmp_dir.join("page").to_string_lossy().to_string();
    let mut cmd_obj = std::process::Command::new(cmd);
    // -l 为闭区间：仅渲染前 MAX_OCR_PAGES 页（与 pdfium 路径同上限）
    cmd_obj.args(["-jpeg", "-r", "150", "-f", "1", "-l", &MAX_OCR_PAGES.to_string(), path, &output_prefix])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd_obj.creation_flags(0x08000000);
    }
    let status = cmd_obj.status().ok()?;
    if !status.success() { let _ = std::fs::remove_dir_all(&tmp_dir); return None; }

    let mut results = Vec::new();
    let mut page_num = 1;
    loop {
        let page_path = tmp_dir.join(format!("page-{}.jpg", page_num));
        if !page_path.exists() { break; }
        if let Some(uri) = file_to_data_uri(&page_path.to_string_lossy(), "image/jpeg") {
            results.push(uri);
        }
        page_num += 1;
    }
    let _ = std::fs::remove_dir_all(&tmp_dir);
    if results.is_empty() { None } else { Some(results) }
}

/// macOS 专用：使用系统自带的 sips（Scriptable Image Processing System）
#[cfg(target_os = "macos")]
fn try_macos_sips(path: &str) -> Option<Vec<String>> {
    use std::process::Command;

    // 先获取 PDF 页数
    let output = Command::new("sips")
        .args(["-g", "pageCount", path])
        .output().ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let num_pages: usize = stdout.lines()
        .find_map(|l| {
            let l = l.trim();
            if l.starts_with("pageCount:") {
                l.split(':').nth(1)?.trim().parse().ok()
            } else { None }
        })?;
    if num_pages == 0 { return None; }

    let mut results = Vec::new();
    for page in 0..num_pages.min(MAX_OCR_PAGES as usize) {
        let tmp_file = std::env::temp_dir().join(format!("arkmind_page_{}.png", page));
        let status = Command::new("sips")
            .args(["-s", "format", "png", "--pdfPage", &page.to_string(),
                   path, "--out", &tmp_file.to_string_lossy()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status().ok()?;
        if status.success() && tmp_file.exists() {
            if let Some(uri) = file_to_data_uri(&tmp_file.to_string_lossy(), "image/png") {
                results.push(uri);
            }
            let _ = std::fs::remove_file(&tmp_file);
        }
    }

    if results.is_empty() { None } else { Some(results) }
}

/// OCR 结果：区分文本 / 空白页 / 失败
pub enum OcrOutcome {
    Ok(String),
    Blank,
    Failed(String),
}

/// 将视觉模型返回的错误分类，返回 (是否可重试, 是否空白页)
fn classify_ocr_error(err: &str) -> (bool, bool) {
    // 空白页类错误：不重试、不判定失败
    if err.contains("返回空内容")
        || err.contains("仅返回版面坐标")
        || err.contains("清洗后为空") {
        return (false, true);
    }
    // 永久配置类错误：不重试、判定失败
    if err.contains("未配置视觉模型")
        || err.contains("HTTP 客户端创建失败") {
        return (false, false);
    }
    // API 错误：按 HTTP 状态码判断（4xx 永久失败；5xx / 429 可重试）
    if let Some(st) = err.find("API 错误 (HTTP ") {
        let rest = &err[st + "API 错误 (HTTP ".len()..];
        if let Some(code) = rest.split(')').next().and_then(|s| s.trim().parse::<u16>().ok()) {
            if code == 429 || code >= 500 {
                return (true, false);
            }
            return (false, false);
        }
    }
    // 其余（请求失败 / 读取响应失败 / 响应不是JSON）→ 可重试
    (true, false)
}

use super::tokio_ctx::{in_blocking_thread, BlockingThreadEnter};

/// 调用视觉模型 API（async），最多重试 3 次（仅重试可恢复错误；空白页直接返回 Blank）。
/// 全局闸在 ocr_with_vision_api 内部逐次 acquire，退避 sleep 期间不占 permit；
/// 429/5xx 退避加 jitter，防并发页同时重试踩限流。
pub async fn ocr_image_async(base64_uri: &str) -> OcrOutcome {
    use rand::Rng;
    const MAX_ATTEMPTS: u32 = 3;
    let mut last_err = String::from("未知错误");
    for attempt in 0..MAX_ATTEMPTS {
        let t0 = std::time::Instant::now();
        let result = ocr_with_vision_api(base64_uri).await;
        let attempt_ms = t0.elapsed().as_millis();

        match result {
            Ok(text) => {
                log::info!("[文件解析] OCR 成功 第 {} 次, 耗时 {}ms, 文本 {} 字符", attempt + 1, attempt_ms, text.len());
                return OcrOutcome::Ok(text);
            }
            Err(e) => {
                let (retryable, blank) = classify_ocr_error(&e);
                if blank {
                    log::info!("[文件解析] OCR 空白页 (耗时 {}ms): {}", attempt_ms, e);
                    return OcrOutcome::Blank;
                }
                last_err = e.clone();
                if !retryable {
                    log::info!("[文件解析] OCR 失败(不可重试, 耗时 {}ms): {}", attempt_ms, e);
                    return OcrOutcome::Failed(e);
                }
                log::info!("[文件解析] OCR 失败 第 {}/{} 次 (耗时 {}ms): {}", attempt + 1, MAX_ATTEMPTS, attempt_ms, e);
                if attempt + 1 < MAX_ATTEMPTS {
                    // 退避 2s/4s + 0~500ms jitter
                    let jitter_ms: u64 = rand::thread_rng().gen_range(0..500);
                    tokio::time::sleep(std::time::Duration::from_millis(2000 * (attempt as u64 + 1) + jitter_ms)).await;
                }
            }
        }
    }
    OcrOutcome::Failed(last_err)
}

/// 同步调用视觉模型 API（兼容 parse_file_smart / 图片附件等旧路径）
pub fn ocr_image_sync_result(base64_uri: &str) -> OcrOutcome {
    // 区分调用场景：worker 线程可用 block_in_place；spawn_blocking 线程 block_in_place 必 panic，
    // 须改跑独立 current-thread runtime（见 BlockingThreadEnter 标记）
    if tokio::runtime::Handle::try_current().is_ok() && !in_blocking_thread() {
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| handle.block_on(ocr_image_async(base64_uri)))
    } else {
        // runtime 每次调用建一个（重试循环在 async 内部，不再逐次重建）
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(r) => r,
            Err(e) => {
                log::info!("[文件解析] tokio 运行时创建失败: {}", e);
                return OcrOutcome::Failed(format!("tokio 运行时创建失败: {}", e));
            }
        };
        rt.block_on(ocr_image_async(base64_uri))
    }
}

/// 同步调用 OCR API 的阻塞封装（兼容旧调用，失败返回空字符串）
#[allow(dead_code)]
pub fn ocr_image_sync(base64_uri: &str) -> String {
    match ocr_image_sync_result(base64_uri) {
        OcrOutcome::Ok(text) => text,
        OcrOutcome::Blank => String::new(),
        OcrOutcome::Failed(e) => {
            log::info!("[文件解析] OCR 失败: {}", e);
            String::new()
        }
    }
}

/// 对图片文件执行 OCR（带重试），返回 Result 以便区分失败原因
fn try_ocr_image_result(path: &str, mime: &str) -> Result<String, String> {
    match file_to_data_uri(path, mime) {
        Some(uri) => match ocr_image_sync_result(&uri) {
            OcrOutcome::Ok(text) => Ok(text),
            OcrOutcome::Blank => Err("图片为空内容或空白页".to_string()),
            OcrOutcome::Failed(e) => Err(e),
        },
        None => {
            log::info!("[文件解析] 图片读取失败: {}", path);
            Err(format!("图片读取失败: {}", path))
        }
    }
}

/// 检查文本是否包含有效字符
pub fn has_real_text(text: &str) -> bool {
    text.chars().any(|c| c.is_ascii_alphanumeric() || c as u32 > 127)
}

fn try_ocr_pdf(path: &str) -> String {
    if get_vision_config().is_none() {
        log::info!("[文件解析] 未配置视觉模型，跳过 OCR");
        return String::new();
    }
    try_pdfium_with_ocr(path, true)
}

fn try_pdfium_extract(path: &str) -> String {
    try_pdfium_with_ocr(path, false)
}

fn try_pdfium_with_ocr(path: &str, enable_ocr: bool) -> String {
    // OCR 模式：跳过 pdfium 文本提取，直接全量 OCR
    if enable_ocr {
        if get_vision_config().is_none() {
            log::info!("[文件解析] 未配置视觉模型，跳过 OCR");
            return String::new();
        }
        let uris = match pdf_to_images_base64(path) {
            Some(u) => u,
            None => {
                log::info!("[文件解析] PDF 渲染失败: {}", path);
                // 渲染失败时回退到 pdfium 文本提取
                return match try_pdfium_text_per_page(path) {
                    Some(pages) => pages.iter().enumerate().fold(String::new(), |mut acc, (i, text)| {
                        if !text.is_empty() { if !acc.is_empty() { acc.push_str(&format!("\n\n--- 第 {} 页 ---\n\n", i + 1)); } acc.push_str(text); }
                        acc
                    }),
                    None => String::new(),
                };
            }
        };
        // 截断尾注：渲染端已按 MAX_OCR_PAGES 封顶（pdfium break / pdftoppm -l / sips min），此处补文案
        let note = ocr_truncation_note(get_pdf_page_count(path).unwrap_or(0), MAX_OCR_PAGES).unwrap_or_default();
        log::info!("[文件解析] 全量 OCR: {} 页", uris.len());
        let mut full_text = String::new();
        for (i, uri) in uris.iter().enumerate() {
            if !std::path::Path::new(path).exists() {
                log::info!("[文件解析] 文件已被删除，终止 OCR（已处理 {} / {} 页）", i, uris.len());
                break;
            }
            match ocr_image_sync_result(uri) {
                OcrOutcome::Ok(text) => {
                    if has_real_text(&text) {
                        if !full_text.is_empty() { full_text.push_str(&format!("\n\n--- 第 {} 页 ---\n\n", i + 1)); }
                        full_text.push_str(&text);
                    }
                }
            OcrOutcome::Blank => {} // 空白页跳过
            OcrOutcome::Failed(e) => log::info!("[文件解析] 第 {} 页 OCR 失败: {}", i + 1, e),
        }
        }
        if !note.is_empty() {
            if !full_text.is_empty() { full_text.push('\n'); }
            full_text.push_str(&note);
        }
        return full_text;
    }

    // 非 OCR 模式：使用 pdfium 文本提取
    let pdfium_pages = match try_pdfium_text_per_page(path) {
        Some(pages) => pages,
        None => {
            log::info!("[文件解析] pdfium 不可用");
            return String::new();
        }
    };
    log::info!("[文件解析] pdfium 提取完成（{} 页）", pdfium_pages.len());
    pdfium_pages.iter().enumerate().fold(String::new(), |mut acc, (i, text)| {
        if !text.is_empty() { if !acc.is_empty() { acc.push_str(&format!("\n\n--- 第 {} 页 ---\n\n", i + 1)); } acc.push_str(text); }
        acc
    })
}
/// zip 容器细分：OOXML（docx/xlsx/pptx）本质是 zip，infer 对 PK 魔数一律给 "zip"——
/// 不细分会导致 file_ext 自愈把 docx 降级成 zip（预览打开资源管理器、下载变 .zip）。
/// 按容器内目录判定：word/ → docx，xl/ → xlsx，ppt/ → pptx；纯 zip/损坏 → None。
pub fn refine_zip_container_ext(bytes: &[u8]) -> Option<&'static str> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor).ok()?;
    let mut has_word = false;
    let mut has_xl = false;
    let mut has_ppt = false;
    for i in 0..archive.len() {
        let name = match archive.by_index(i) { Ok(f) => f.name().to_string(), Err(_) => continue };
        if name.starts_with("word/") { has_word = true; }
        else if name.starts_with("xl/") { has_xl = true; }
        else if name.starts_with("ppt/") { has_ppt = true; }
        if has_word || has_xl || has_ppt { break; }
    }
    if has_word { Some("docx") } else if has_xl { Some("xlsx") } else if has_ppt { Some("pptx") } else { None }
}

// ═══ 有界读取与尺寸闸（Windows 闪退/整机死机根因修复）════════════════════
// 历史：嗅探/zip 细分把整文件读进内存，叠加摄入 MAX_CONCURRENT=5 并发——
// 大文件阶段数 GB 分配打满 Windows 换页 → 整机假死，随后 OOM 杀进程（闪退）。
// 现约定：任何解析入口的单次文件读取必须有界；超尺寸直接 unsupported 终态。

/// 单文件解析尺寸上限（与目录遍历同口径 200MB）：超限不解析、不入知识索引（文件本体保留可用）
pub const MAX_PARSE_BYTES: u64 = 200 * 1024 * 1024;

/// 尺寸闸判定：返回 Some(拒绝理由) 则不得继续解析
fn parse_size_gate(file_len: u64) -> Option<String> {
    if file_len > MAX_PARSE_BYTES {
        Some(format!(
            "{}文件过大（{}MB > 200MB 上限），暂不解析、不入知识索引",
            crate::services::file_ingestion::UNSUPPORTED_PREFIX,
            file_len / 1_048_576
        ))
    } else {
        None
    }
}

/// OCR 页数上限：超限仅识别前 N 页并追加尾注——渲染内存与视觉模型调用数双重封顶
pub const MAX_OCR_PAGES: u16 = 300;

/// OCR 截断尾注（纯函数）：未达上限返回 None
pub(crate) fn ocr_truncation_note(raw_total: u16, cap: u16) -> Option<String> {
    if raw_total > cap {
        Some(format!("（OCR 已截断：仅识别前 {} 页，全文共 {} 页）", cap, raw_total))
    } else {
        None
    }
}

/// 有界窗口读取（绝不整文件入内存）。off=None 从头读 cap 字节；否则从指定偏移读
fn read_window(path: &str, off: Option<u64>, cap: usize) -> Vec<u8> {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return Vec::new(); };
    if let Some(o) = off {
        if f.seek(SeekFrom::Start(o)).is_err() {
            return Vec::new();
        }
    }
    let mut buf = vec![0u8; cap];
    let n = f.read(&mut buf).unwrap_or(0);
    buf.truncate(n);
    buf
}

const ZIP_REFINE_FULL_CAP: u64 = 64 * 1024 * 1024;
const ZIP_TAIL_WINDOW: u64 = 1024 * 1024;
const ZIP_HEAD_WINDOW: usize = 256 * 1024;

/// OOXML 必备条目名（central directory 含全部条目名，作为窗口扫描特征）
fn scan_ooxml_markers(bytes: &[u8]) -> Option<&'static str> {
    let has = |s: &str| bytes.windows(s.len()).any(|w| w == s.as_bytes());
    if has("word/document.xml") { Some("docx") }
    else if has("xl/workbook.xml") { Some("xlsx") }
    else if has("ppt/presentation.xml") { Some("pptx") }
    else { None }
}

/// zip 容器细分（有界版，替代调用方的全文件读取）：
/// ≤64MB 全读走 zip crate 精确解析（内存封顶 64MB）；>64MB 不整读——
/// central directory 在文件尾且携带全部条目名，尾 1MB + 头 256KB 双窗扫描特征条目即可判型
pub fn refine_zip_bounded(path: &str) -> Option<&'static str> {
    let size = std::fs::metadata(path).ok()?.len();
    if size == 0 { return None; }
    if size <= ZIP_REFINE_FULL_CAP {
        let all = std::fs::read(path).ok()?;
        return refine_zip_container_ext(&all);
    }
    let tail_off = size.saturating_sub(ZIP_TAIL_WINDOW);
    if let Some(r) = scan_ooxml_markers(&read_window(path, Some(tail_off), ZIP_TAIL_WINDOW as usize)) {
        return Some(r);
    }
    scan_ooxml_markers(&read_window(path, None, ZIP_HEAD_WINDOW))
}

/// pdfium(C++) / OCR 光栅化串行锁：进程内最重内存消耗者，5 并发叠峰会打满交换。
/// 持锁点仅在 parse_file_smart 的各解析 arm（这些重型函数之间无互调，无重入死锁）；
/// 峰值从「并发数 × 单 PDF」压到「单 PDF」
static HEAVY_PARSE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn heavy_parse_guard() -> std::sync::MutexGuard<'static, ()> {
    HEAVY_PARSE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// ★ 内容权威嗅探：读文件头 4KB 用 infer 识别真实类型。
/// 命中返回真实扩展名；未命中回退文件名扩展（自动剥离 Outlook CID 尾巴如 `image.jpg@hex.hash`）；再无则空。
pub fn sniff_file_ext(path: &str, file_name: &str) -> String {
    if let Ok(bytes) = std::fs::File::open(path).and_then(|mut f| {
        use std::io::Read;
        let mut buf = vec![0u8; 4096];
        let n = f.read(&mut buf)?;
        buf.truncate(n);
        Ok(buf)
    }) {
        if let Some(kind) = infer::get(&bytes) {
            let ext = kind.extension().to_string();
            // ★ zip 容器细分：OOXML 不得降级为 zip（central directory 在文件尾部，需读全文件）
            if ext == "zip" {
                if let Some(r) = refine_zip_bounded(path) {
                    return r.to_string();
                }
            }
            return ext;
        }
    }
    // 回退：文件名扩展；若扩展呈十六进制长尾（疑似 CID 垃圾后缀），取 '@' 前主名的扩展
    let raw = std::path::Path::new(file_name)
        .extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let looks_hex_junk = raw.len() >= 6 && raw.chars().all(|c| c.is_ascii_hexdigit());
    if !raw.is_empty() && !looks_hex_junk {
        return raw;
    }
    let base = file_name.split('@').next().unwrap_or(file_name);
    std::path::Path::new(base)
        .extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase()
}

pub fn parse_file_smart(path: &str, hinted_ext: &str) -> ParseResult {
    if !std::path::Path::new(path).exists() {
        return ParseResult { text: String::new(), success: false, ..Default::default() };
    }

    // ── 尺寸闸（200MB，与目录遍历同口径）：超限直接 unsupported 终态，杜绝大文件入解析管线 ──
    if let Ok(md) = std::fs::metadata(path) {
        if let Some(reason) = parse_size_gate(md.len()) {
            return ParseResult { text: String::new(), success: false, error: reason, inferred_ext: None };
        }
    }

    // ── 读文件头做嗅探（有界 8KB：此前是全量 std::fs::read，大文件闪退/死机根因）──
    let head = read_window(path, None, 8 * 1024);
    let probe = &head[..head.len().min(4096)];
    let inferred = infer::get(probe);
    // ★ 内容权威：魔数命中即记录真实类型扩展名（文件名后缀不可信）
    let mut magic_ext: Option<String> = inferred.map(|k| k.extension().to_string());
    // ★ zip 容器细分：OOXML 的 PK 魔数被 infer 判为 zip——不细分会让 file_ext 自愈降级 docx → zip
    if magic_ext.as_deref() == Some("zip") {
        if let Some(r) = refine_zip_bounded(path) {
            magic_ext = Some(r.to_string());
        }
    }
    let mk = |text: String, error: String| { let success = !text.is_empty(); ParseResult { text, success, error, inferred_ext: magic_ext.clone() } };

    // ── 嗅探命中：直接路由 ──
    if let Some(kind) = inferred {
        match kind.mime_type() {
            "application/pdf" => {
                // PDF: OCR（pdfium + 视觉模型）优先 → pdfium 纯文本 → pdf-extract（重型解析串行）
                let _heavy = heavy_parse_guard();
                let ocr_result = try_ocr_pdf(path);
                if !ocr_result.is_empty() {
                    return mk(ocr_result, String::new());
                }
                // 视觉模型未配置 → pdfium 纯文本
                let pdfium_text = try_pdfium_extract(path);
                if !pdfium_text.is_empty() {
                    return mk(pdfium_text, String::new());
                }
                log::info!("[文件解析] PDF 解析失败，回退到 pdf-extract");
                let text = read_pdf(path);
                return mk(text, String::new());
            }
            "image/png" | "image/jpeg" | "image/webp" | "image/bmp" => {
                let _heavy = heavy_parse_guard();
                return match try_ocr_image_result(path, kind.mime_type()) {
                    Ok(text) => mk(text, String::new()),
                    Err(e) => mk(String::new(), format!("{}视觉模型解析失败: {}", crate::services::file_ingestion::NON_RETRYABLE_PREFIX, e)),
                };
            }
            "application/zip" | "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
                // ZIP 基类：可能是 docx / xlsx / 纯 zip
                // 优先尝试 DOCX（内部 XML），失败回退 XLSX
                // ★ 仅接受非空文本命中：纯 zip 会被 read_pptx 开出 Ok("")——空结果不是命中，
                //   否则走 mk() 保住 zip 魔数，ingestion 把"已知容器无解析器"误判成真解析失败(→failed×3)
                if let Ok(t) = read_docx(path) { if !t.is_empty() { return mk(t, String::new()); } }
                if let Ok(t) = read_xlsx(path) { if !t.is_empty() { return mk(t, String::new()); } }
                if let Ok(t) = read_pptx(path) { if !t.is_empty() { return mk(t, String::new()); } }
            }
            "text/plain" | "text/csv" | "application/json" => {
                let text = std::fs::read_to_string(path).unwrap_or_default();
                return mk(text, String::new());
            }
            "application/msword" | "application/x-ole-storage" => {
                // OLE2 格式（.doc）：使用 rwml 解析
                if let Ok(t) = read_doc_legacy(path) {
                    return mk(t, String::new());
                }
            }
            _ => {} // 未知二进制 — 转入后缀路由
        }
    }

    // ── 后缀路由 ──
    let ext = hinted_ext.to_lowercase();
    let text = match ext.as_str() {
        "txt" | "md" | "csv" | "json" => std::fs::read_to_string(path).unwrap_or_default(),
        "docx" => read_docx(path).unwrap_or_default(),
        "doc" => read_doc_legacy(path).unwrap_or_default(),
        "xlsx" | "xls" => read_xlsx(path).unwrap_or_default(),
        "pptx" | "ppt" => read_pptx(path).unwrap_or_default(),
        "pdf" => {
            // 后缀 PDF：OCR（pdfium + 视觉模型）优先 → pdfium 纯文本 → pdf-extract（重型解析串行）
            let _heavy = heavy_parse_guard();
            let ocr = try_ocr_pdf(path);
            if !ocr.is_empty() { ocr } else { try_pdfium_extract(path) }
        }
        "png" | "jpg" | "jpeg" | "webp" | "bmp" => {
            let mime = match ext.as_str() {
                "png" => "image/png", "jpg" | "jpeg" => "image/jpeg",
                "webp" => "image/webp", "bmp" => "image/bmp",
                _ => "image/png",
            };
            let _heavy = heavy_parse_guard();
            return match try_ocr_image_result(path, mime) {
                Ok(t) if !t.is_empty() => ParseResult { success: true, text: t, ..Default::default() },
                Ok(_) => ParseResult { success: false, text: String::new(), error: format!("{}视觉模型解析失败: OCR 未返回内容", crate::services::file_ingestion::NON_RETRYABLE_PREFIX), inferred_ext: None },
                Err(e) => ParseResult { success: false, text: String::new(), error: format!("{}视觉模型解析失败: {}", crate::services::file_ingestion::NON_RETRYABLE_PREFIX, e), inferred_ext: None },
            };
        }
        _ => String::new(),
    };
    if !text.is_empty() {
        return ParseResult { success: true, text, ..Default::default() };
    }

    // ── 轮询兜底：依次硬啃 PDF → DOCX → XLSX → TXT（含 read_pdf，重型解析串行）──
    let _heavy = heavy_parse_guard();
    let fallbacks: &[fn(&str) -> String] = &[read_pdf, |p| read_docx(p).unwrap_or_default(), |p| read_xlsx(p).unwrap_or_default(), |p| read_pptx(p).unwrap_or_default(), |p| std::fs::read_to_string(p).unwrap_or_default()];
    for try_parse in fallbacks {
        let t = try_parse(path);
        if !t.is_empty() && t.len() > 20 {
            return ParseResult { success: true, text: t, ..Default::default() };
        }
    }

    // ★ 失败出口保留魔数结论：改名压缩包（rar→docx 等）靠它走 unsupported 终态，
    //   而非误入"后缀受支持的真解析失败"×3 重试；成功出口维持原样（自愈仅针对明确命中解析器）
    ParseResult { text: String::new(), success: false, inferred_ext: magic_ext, ..Default::default() }
}

/// 兼容旧调用
pub fn parse_file_with_hint(path: &str, ext: &str) -> ParseResult {
    parse_file_smart(path, ext)
}

// ═══════════════════════════════════════════════════════════════
// 两级清洗：基础消毒（分格式） vs 向量化清洗
// ═══════════════════════════════════════════════════════════════

/// 使用 html2md 库将 HTML 转换为 Markdown（支持表格、列表等复杂结构）
/// - 删除 <style> 标签及其内容
/// - 忽略 <img src="cid:..."> 引用
pub fn html_to_markdown_v2(html: &str) -> String {
    // 1. 删除 <style>...</style> 标签及其内容
    let re_style = regex::Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();
    let cleaned = re_style.replace_all(html, "");
    
    // 2. 删除 <img src="cid:..."> 标签（内嵌图片已在邮件正文中渲染）
    let re_cid_img = regex::Regex::new(r#"(?i)<img[^>]*src\s*=\s*"cid:[^"]*"[^>]*/?>"#).unwrap();
    let cleaned = re_cid_img.replace_all(&cleaned, "");
    
    // 3. 使用 html2md 库转换
    let result = html2md::parse_html(&cleaned);
    
    // 4. 清理多余的空行
    let re_empty_lines = regex::Regex::new(r"\n{3,}").unwrap();
    let result = re_empty_lines.replace_all(&result, "\n\n");
    
    result.trim().to_string()
}

/// 邮件正文专用 HTML→Markdown：与 v2 的区别是 cid 图片不删除而是留 [图片:{cid}] 占位，
/// 供 extract_email 就地注入 OCR 文字；顺手剔除 data:URI 残留（存量未转附件的老邮件）。
pub fn html_to_markdown_email(html: &str) -> String {
    let re_style = regex::Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();
    let cleaned = re_style.replace_all(html, "");

    // data:URI 内联图（存量老邮件）：整标签删除，base64 绝不进文本
    let re_data_img = regex::Regex::new(r#"(?i)<img[^>]*src\s*=\s*["']?data:[^"'>\s]+["']?[^>]*?/?>"#).unwrap();
    let cleaned = re_data_img.replace_all(&cleaned, "").to_string();

    // cid 图片 → 纯文本占位（html2md 会原样保留文本节点），extract_email 负责替换为 OCR 文字或剔除
    let re_cid_img = regex::Regex::new(r#"(?i)<img[^>]*src\s*=\s*"cid:([^"]*)"[^>]*/?>"#).unwrap();
    let cleaned = re_cid_img.replace_all(&cleaned, "[图片:$1]").to_string();

    let result = html2md::parse_html(&cleaned);
    let re_empty_lines = regex::Regex::new(r"\n{3,}").unwrap();
    let result = re_empty_lines.replace_all(&result, "\n\n");
    // html2md 对未知结构可能产出的 base64 data URI 兜底剥离（![x](data:...) / [x](data:...)）
    let re_md_data = regex::Regex::new(r"!?\[[^\]]*\]\(data:[^)]*\)").unwrap();
    let result = re_md_data.replace_all(&result, "").to_string();

    result.trim().to_string()
}

/// 将 TipTap HTML 转换为 Markdown。支持 p/h1-h6/ul/ol/li/strong/em/u/s/a/pre/code/blockquote/hr/table/br。
pub fn html_to_markdown(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let chars: Vec<char> = html.chars().collect();
    let len = chars.len();
    let mut i = 0;
    // 追踪是否在表格单元格内（br 正确处理）
    let mut in_table_cell = false;

    while i < len {
        if chars[i] == '<' {
            let tag_end = chars[i..].iter().position(|&c| c == '>').map(|p| i + p + 1).unwrap_or(len);
            let tag_str: String = chars[i+1..tag_end-1].iter().collect();
            let tag_lower = tag_str.to_lowercase();
            let is_close = tag_lower.starts_with('/');
            let tag_name = if is_close { &tag_lower[1..] } else { &tag_lower };
            let tag_name = tag_name.split_whitespace().next().unwrap_or(tag_name); // strip attributes

            match tag_name {
                "p" | "div" | "span" | "section" => {
                    if !is_close && i > 0 && out.chars().last() != Some('\n') { out.push('\n'); }
                }
                "h1" => { if !is_close { out.push_str("# "); } else { out.push('\n'); } }
                "h2" => { if !is_close { out.push_str("## "); } else { out.push('\n'); } }
                "h3" => { if !is_close { out.push_str("### "); } else { out.push('\n'); } }
                "h4" => { if !is_close { out.push_str("#### "); } else { out.push('\n'); } }
                "h5" => { if !is_close { out.push_str("##### "); } else { out.push('\n'); } }
                "h6" => { if !is_close { out.push_str("###### "); } else { out.push('\n'); } }
                "strong" | "b" => { out.push_str(if is_close { "**" } else { "**" }); }
                "em" | "i" => { out.push_str(if is_close { "*" } else { "*" }); }
                "u" | "ins" => { out.push_str(if is_close { "__" } else { "__" }); }
                "s" | "del" | "strike" => { out.push_str(if is_close { "~~" } else { "~~" }); }
                "br" | "br/" => {
                    if in_table_cell { out.push(' '); } else { out.push('\n'); }
                }
                "hr" | "hr/" => { out.push_str("\n---\n"); }
                "ul" => { if is_close { out.push('\n'); } }
                "ol" => { if is_close { out.push('\n'); } }
                "li" => {
                    if !is_close {
                        // 检测是 ul 还是 ol
                        let start_char = i.saturating_sub(50);
                        let parent_list: String = chars[start_char..i].iter().collect::<String>().to_lowercase();
                        if parent_list.contains("<ol") {
                            out.push_str("1. ");
                        } else {
                            out.push_str("- ");
                        }
                    } else { out.push('\n'); }
                }
                "a" => {
                    if !is_close {
                        out.push('[');
                    } else { out.push_str("]()"); }
                }
                "code" => { out.push_str(if is_close { "`" } else { "`" }); }
                "pre" => {
                    if !is_close { out.push_str("\n```\n"); } else { out.push_str("\n```\n"); }
                }
                "blockquote" => {
                    if !is_close { out.push_str("> "); } else { out.push('\n'); }
                }
                "table" => { if is_close { out.push('\n'); } }
                "tr" => {
                    if is_close { out.push('\n'); }
                }
                "th" | "td" => {
                    if !is_close { in_table_cell = true; out.push('|'); } else { in_table_cell = false; }
                }
                "img" => {
                    let src_start = tag_str.find("src=\"");
                    let alt_start = tag_str.find("alt=\"");
                    let src = src_start.map(|s| tag_str[s+5..].chars().take_while(|&c| c != '"').collect::<String>()).unwrap_or_default();
                    let alt = alt_start.map(|s| tag_str[s+5..].chars().take_while(|&c| c != '"').collect::<String>()).unwrap_or_default();
                    out.push_str(&format!("![{}]({})", alt, src));
                }
                _ => {}
            }
            i = tag_end;
        } else if chars[i] == '&' {
            // HTML entity decoding
            let semi = chars[i..].iter().position(|&c| c == ';').map(|p| i + p + 1).unwrap_or(len);
            let entity: String = chars[i+1..semi-1].iter().collect();
            let decoded = match entity.as_str() {
                "amp" => "&",
                "lt" => "<",
                "gt" => ">",
                "quot" => "\"",
                "#39" | "apos" => "'",
                "nbsp" => " ",
                _ => { i = semi; continue; }
            };
            out.push_str(decoded);
            i = semi;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }

    out
}

/// 统一换行符，去掉首尾空白行。不改变段落结构。
pub fn sanitize_chars(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '\n' | '\r' | '\t' => out.push(c),
            c if c.is_ascii_control() => {} // 跳过 BEL、ESC 等
            c if c == '\u{FEFF}' || c == '\u{FFFE}' => {} // BOM / 非法 Unicode
            c if c.is_ascii_whitespace() => out.push(' '), // 其他 ASCII 空白归一为空格
            _ => out.push(c),
        }
    }
    let unified = out.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = unified.lines().collect();
    let start = lines.iter().position(|l| !l.trim().is_empty()).unwrap_or(0);
    let end = lines.iter().rposition(|l| !l.trim().is_empty()).map(|i| i + 1).unwrap_or(lines.len());
    let trimmed = lines[start..end].join("\n");
    let no_pages = strip_page_numbers(&trimmed);
    strip_garbled_lines(&no_pages)
}

/// 去除 PDF OCR 等工具留下的页码标记，如 "--- 第 1 页 ---" / "— Page 2 —" 等。
fn strip_page_numbers(text: &str) -> String {
    text.lines()
        .filter(|line| {
            let t = line.trim();
            if t.len() > 30 { return true; }
            let wrapped = (t.starts_with("---") && t.ends_with("---"))
                || (t.starts_with("—") && t.ends_with("—"))
                || (t.starts_with("--") && t.ends_with("--"));
            if !wrapped { return true; }
            let has_page_keyword = t.contains('页') || t.contains("Page")
                || t.contains("page") || t.contains("PAGE");
            !has_page_keyword
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 去除解析噪音行：同一字符重复 ≥10 次（如 "目目目目目目目目目目"、"............"）。
fn strip_garbled_lines(text: &str) -> String {
    text.lines()
        .filter(|line| {
            let t = line.trim();
            if t.chars().count() < 10 { return true; }
            let first = t.chars().next().unwrap();
            !t.chars().all(|c| c == first)
        })
        .collect::<Vec<_>>()
        .join("\n")
}
/// PDF 断行修复：合并非段落结尾的碎片行。
/// 规则：不以句尾标点结尾的行，与下一行合并（加空格），除非下一行是标题。
#[allow(dead_code)]
fn merge_pdf_broken_lines(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 2 { return text.to_string(); }

    // 短行（≤20字符）且不是句尾 → 视为独立章节标题，不合并到上一段落
    let is_short_title = |s: &str| -> bool {
        let t = s.trim();
        if t.is_empty() { return false; }
        t.chars().count() <= 20
            && !t.ends_with('.') && !t.ends_with('。') && !t.ends_with('！')
            && !t.ends_with('？') && !t.ends_with('：') && !t.ends_with(':')
    };

    let mut result = Vec::with_capacity(lines.len());
    let mut i = 0usize;
    while i < lines.len() {
        let mut current = lines[i].trim_end().to_string();
        // 空行直接保留（段落边界）
        if current.trim().is_empty() {
            result.push(String::new());
            i += 1;
            continue;
        }
        // 向后合并：当前行不是句尾 → 尝试与下一行合并
        while i + 1 < lines.len() {
            let next = lines[i + 1].trim();
            if next.is_empty() { break; } // 空行是段落边界
            // 句尾结束 → 停止合并
            let ends_with_sentence = current.ends_with('.') || current.ends_with('?') || current.ends_with('!')
                || current.ends_with('。') || current.ends_with('？') || current.ends_with('！')
                || current.ends_with('：') || current.ends_with(':')
                || current.ends_with('」') || current.ends_with('"');
            if ends_with_sentence { break; }
            // 当前行是标题 → 停止合并（标题后必须换行）
            if current.trim_start().starts_with('#') { break; }
            // HTML 表格标签 → 停止合并（wrap_orphan_table_html 已拆到单独行）
            if next.starts_with("<table") || next.starts_with("</table>") { break; }
            // 下一行是短章节标题 → 停止合并
            if is_short_title(next) { break; }
            current.push(' ');
            current.push_str(next);
            i += 1;
        }
        result.push(current);
        i += 1;
    }
    result.join("\n")
}

/// 带格式提示的消毒版本，供摄入中台调用。
///   - pdf  → 基础消毒（OCR 输出格式已完整，无需断行修复）
///   - docx → 基础消毒
///   - xlsx/xls → 基础消毒（已输出 Markdown 表格）
///   - 其他 → 基础消毒 + 游离表格 HTML 补全（OCR 输出经常缺 <table> 标签）
pub fn clean_extracted_text_with_hint(raw: &str, ext: &str) -> String {
    let base = sanitize_chars(raw);
    let base = wrap_orphan_table_html(&base);
    match ext.to_lowercase().as_str() {
        "pdf" => base,
        "docx" => base,
        "xlsx" | "xls" => clean_xlsx(&base),
        "txt" | "md" | "csv" | "json" => base,
        _ => base,
    }
}

/// 补全游离的表格 HTML 片段，保证 chunker 的 html_table_to_kv 能正确识别。
/// 两个功能：
///   1）<table 出现在行中（前面有其他文本）→ 拆到单独一行
///   2）存在 <thead>/<tbody> 但缺 <table> 包裹 → 自动包裹
fn wrap_orphan_table_html(text: &str) -> String {
    // 先处理行中的 <table
    let mut split = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(pos) = trimmed.find("<table") {
            if pos > 0 {
                let before = &trimmed[..pos].trim_end();
                if !before.is_empty() {
                    split.push_str(before);
                    split.push('\n');
                }
                split.push_str(&trimmed[pos..]);
                split.push('\n');
                continue;
            }
        }
        split.push_str(line);
        split.push('\n');
    }

    let has_table_tag = split.contains("<table") || split.contains("</table>");
    let has_table_section = split.contains("<thead") || split.contains("<tbody");
    if !has_table_tag && has_table_section {
        format!("<table>\n{}\n</table>", split)
    } else {
        split
    }
}

/// XLSX 清洗：read_xlsx 已输出 Markdown 表格格式，只需基础消毒
fn clean_xlsx(text: &str) -> String {
    text.to_string()
}

// ── 底层解析器 ──

/// PPTX 解析：提取所有幻灯片中的文本（<a:t> 节点），按页输出 markdown
fn read_pptx(p: &str) -> Result<String, Box<dyn std::error::Error>> {
    use quick_xml::events::Event;
    use quick_xml::Reader;
    use std::io::BufReader;

    let data = std::fs::read(p)?;
    let cursor = std::io::Cursor::new(data);
    let mut zip = zip::ZipArchive::new(cursor)?;

    // 查找所有幻灯片文件
    let mut slide_files: Vec<String> = Vec::new();
    for i in 0..zip.len() {
        let name = zip.by_index(i)?.name().to_string();
        if name.starts_with("ppt/slides/slide") && name.ends_with(".xml") {
            slide_files.push(name);
        }
    }
    slide_files.sort();

    if slide_files.is_empty() { return Ok(String::new()); }

    let mut result = String::new();
    for (idx, slide_name) in slide_files.iter().enumerate() {
        let slide = zip.by_name(slide_name)?;
        let mut reader = Reader::from_reader(BufReader::new(slide));
        reader.config_mut().trim_text(true);
        let mut buf = Vec::new();
        let mut slide_text = String::new();
        let mut in_t = false; // <a:t> text element

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    let __name = e.name();
                    let n = std::str::from_utf8(__name.as_ref()).unwrap_or("");
                    if n == "a:t" { in_t = true; }
                }
                Ok(Event::End(ref e)) => {
                    let __name = e.name();
                    let n = std::str::from_utf8(__name.as_ref()).unwrap_or("");
                    if n == "a:t" { in_t = false; }
                    if n == "a:p" { slide_text.push('\n'); } // 段落结束
                }
                Ok(Event::Text(e)) => {
                    if in_t {
                        if let Ok(s) = e.unescape() { slide_text.push_str(&s); }
                    }
                }
                Ok(Event::Eof) => break,
                Err(_) => break,
                _ => {}
            }
            buf.clear();
        }

        let trimmed = slide_text.trim().to_string();
        if !trimmed.is_empty() {
            if !result.is_empty() {
                result.push_str("\n\n");
            }
            result.push_str(&format!("## 幻灯片 {}\n\n{}", idx + 1, trimmed));
        }
    }

    Ok(result.trim().to_string())
}

/// DOCX → Markdown：rwml 解析出文档模型后由 `word_markdown` 渲染 GFM
/// （标题、粗斜体、删除线、表格、列表、超链接等），图片以轻量注记标记（不做 OCR——
/// 内嵌图片多为截图/装饰，OCR 碎片价值低且污染检索）
fn read_docx_to_markdown(p: &str) -> Result<String, Box<dyn std::error::Error>> {
    let bytes = std::fs::read(p)?;
    word_bytes_to_markdown(&bytes)
}

/// DOCX 解析：保留旧接口名，实现同 `read_docx_to_markdown`
fn read_docx(p: &str) -> Result<String, Box<dyn std::error::Error>> {
    read_docx_to_markdown(p)
}

/// Word（.doc/.docx）字节流 → Markdown。rwml 建模 + word_markdown 渲染。
/// 图片占位符替换为轻量注记（含 alt 文本）：不调视觉模型 OCR——
/// Word 内嵌图片以截图/装饰为主，OCR 产出脱离上下文的碎片，
/// 写入知识库反而稀释检索质量；且每图一次视觉调用（含重试）成本高、拖慢解析。
/// 需要图片文字的场景：用户直接发图片（独立 OCR 路径）或 PDF 扫描件（整页 OCR）。
fn word_bytes_to_markdown(bytes: &[u8]) -> Result<String, Box<dyn std::error::Error>> {
    use crate::services::word_markdown;

    let doc = rwml::Document::open(bytes)?;
    let model = doc.model();
    let rendered = word_markdown::render(&model);
    let mut markdown = rendered.markdown;

    // 图片注记：保留"此处有图"的结构信息（alt 常含图题），零 API 成本
    for img in &rendered.images {
        markdown = markdown.replace(&img.placeholder, &word_markdown::image_note_fallback(img.alt.as_deref()));
    }

    Ok(markdown)
}

/// DOC 解析：使用 rwml 解析旧版 Word 二进制格式（OLE2），输出 Markdown
fn read_doc_legacy(p: &str) -> Result<String, Box<dyn std::error::Error>> {
    let bytes = std::fs::read(p)?;
    word_bytes_to_markdown(&bytes)
}

fn read_xlsx(p: &str) -> Result<String, Box<dyn std::error::Error>> {
    use calamine::{Reader, Data, Dimensions, Sheets};
    let mut wb = calamine::open_workbook_auto(p)?;
    let mut result = String::new();
    let names = wb.sheet_names();

    // Pre-load merged regions if XLSX (loads once for all sheets); XLS doesn't need preload
    if let Sheets::Xlsx(ref mut xlsx) = wb {
        xlsx.load_merged_regions().ok();
    }

    for sn in &names {
        // Get merged cells info (XLSX and XLS both supported)
        let merges: Vec<Dimensions> = match wb {
            Sheets::Xlsx(ref mut xlsx) => xlsx
                .worksheet_merge_cells(sn)
                .and_then(|r| r.ok())
                .unwrap_or_default(),
            Sheets::Xls(ref xls) => xls
                .worksheet_merge_cells(sn)
                .unwrap_or_default()
                .into_iter()
                .map(|r| Dimensions {
                    start: (r.start.0 as u32, r.start.1 as u32),
                    end: (r.end.0 as u32, r.end.1 as u32),
                })
                .collect(),
            _ => vec![],
        };

        if let Ok(ws) = wb.worksheet_range(sn) {
            result.push_str(&format!("## {}\n\n", sn));

            let base = ws.start().unwrap_or((0, 0));
            let base_row = base.0;
            let base_col = base.1;

            // Build merge map: (rel_row, rel_col) -> (rowspan, colspan)
            // and skip set for non-origin cells within merged ranges
            let mut merge_map: std::collections::HashMap<(usize, usize), (usize, usize)> =
                std::collections::HashMap::new();
            let mut skip_cells: std::collections::HashSet<(usize, usize)> =
                std::collections::HashSet::new();
            for m in &merges {
                let r0 = (m.start.0 - base_row) as usize;
                let c0 = (m.start.1 - base_col) as usize;
                let r1 = (m.end.0 - base_row) as usize;
                let c1 = (m.end.1 - base_col) as usize;
                let rs = r1 - r0 + 1;
                let cs = c1 - c0 + 1;
                merge_map.insert((r0, c0), (rs, cs));
                for r in r0..=r1 {
                    for c in c0..=c1 {
                        if r != r0 || c != c0 {
                            skip_cells.insert((r, c));
                        }
                    }
                }
            }

            let mut rows: Vec<(usize, Vec<String>)> = Vec::new();
            for (orig_ri, row) in ws.rows().enumerate() {
                let cells: Vec<String> = row
                    .iter()
                    .map(|c| match c {
                        Data::Empty => String::new(),
                        Data::Error(e) => format!("#ERR {}", e),
                        Data::String(s) => s.trim().replace('\r', "").replace('\n', " "),
                        Data::Float(f) => {
                            if *f == 0.0 {
                                "0".to_string()
                            } else {
                                format!("{}", f)
                            }
                        }
                        Data::Int(i) => format!("{}", i),
                        Data::Bool(b) => format!("{}", b),
                        _ => c.to_string(),
                    })
                    .collect();

                if cells.iter().all(|c| c.is_empty()) {
                    continue;
                }
                rows.push((orig_ri, cells));
            }

            if rows.is_empty() {
                continue;
            }

            let max_cols = rows.iter().map(|r| r.1.len()).max().unwrap_or(0);
            if max_cols == 0 {
                continue;
            }

            // Detect empty columns — 跳过每一行都空的列
            let mut non_empty_cols = vec![false; max_cols];
            for row in &rows {
                for (ci, cell) in row.1.iter().enumerate() {
                    if ci < max_cols && !cell.is_empty() {
                        non_empty_cols[ci] = true;
                    }
                }
            }
            let col_map: Vec<usize> = non_empty_cols
                .iter()
                .enumerate()
                .filter(|(_, &non_empty)| non_empty)
                .map(|(i, _)| i)
                .collect();

            result.push_str("<table>\n");
            for (ri, (orig_ri, row)) in rows.iter().enumerate() {
                result.push_str("<tr>");
                let tag = if ri == 0 { "th" } else { "td" };
                for &ci in &col_map {
                    // 跳过被合并覆盖的非起始单元格（用原始行号匹配）
                    if skip_cells.contains(&(*orig_ri, ci)) {
                        continue;
                    }
                    let cell = row.get(ci).map(|s| s.as_str()).unwrap_or("");
                    let cleaned = cell
                        .replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;");
                    let attrs = match merge_map.get(&(*orig_ri, ci)) {
                        Some(&(rs, cs)) => format!(" rowspan=\"{}\" colspan=\"{}\"", rs, cs),
                        None => String::new(),
                    };
                    result.push_str(&format!("<{}{}>{}</{}>", tag, attrs, cleaned, tag));
                }
                result.push_str("</tr>\n");
            }
            result.push_str("</table>\n\n");
        }
    }

    let trimmed = result.trim().to_string();
    if trimmed.is_empty() {
        for sn in wb.sheet_names() {
            if let Ok(ws) = wb.worksheet_range(&sn) {
                for row in ws.rows() {
                    for cell in row {
                        match cell {
                            Data::Empty => {}
                            Data::Error(e) => result.push_str(&format!("#ERR {} ", e)),
                            Data::String(s) => {
                                result.push_str(&format!("{} ", s.trim()));
                            }
                            Data::Float(f) => {
                                result.push_str(&format!("{} ", f));
                            }
                            Data::Int(i) => {
                                result.push_str(&format!("{} ", i));
                            }
                            _ => result.push_str(&format!("{} ", cell.to_string())),
                        }
                    }
                    result.push('\n');
                }
            }
        }
        return Ok(result.trim().to_string());
    }
    Ok(trimmed)
}

fn read_pdf(p: &str) -> String {
    pdf_extract::extract_text(p).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// zip 容器细分：OOXML（docx/xlsx/pptx）不得被当纯 zip——否则 file_ext 自愈降级 → 预览/下载变 .zip
    #[test]
    fn refine_zip_distinguishes_ooxml_from_plain_zip() {
        use std::io::Write as _;
        let make_zip = |entries: &[&str]| -> Vec<u8> {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            for name in entries {
                w.start_file(*name, zip::write::FileOptions::default()).unwrap();
                w.write_all(b"x").unwrap();
            }
            w.finish().unwrap().into_inner()
        };
        assert_eq!(refine_zip_container_ext(&make_zip(&["word/document.xml"])), Some("docx"));
        assert_eq!(refine_zip_container_ext(&make_zip(&["xl/workbook.xml", "xl/styles.xml"])), Some("xlsx"));
        assert_eq!(refine_zip_container_ext(&make_zip(&["ppt/presentation.xml"])), Some("pptx"));
        assert_eq!(refine_zip_container_ext(&make_zip(&["readme.txt"])), None, "纯 zip 不归类");
        assert_eq!(refine_zip_container_ext(b"not a zip at all"), None, "非 zip 字节 → None");
    }

    /// ★ 尺寸闸：200MB 内放行、超限 unsupported 终态且入口即拒不读文件——
    /// 大文件全量入内存 + 5 并发曾把 Windows 整机换页打挂（死机）后被 OOM 杀进程（闪退）
    #[test]
    fn parse_size_gate_boundary_and_oversize_sparse_file() {
        use std::io::Write as _;
        assert!(parse_size_gate(MAX_PARSE_BYTES).is_none(), "恰达上限放行");
        let err = parse_size_gate(MAX_PARSE_BYTES + 1).unwrap();
        assert!(err.starts_with(crate::services::file_ingestion::UNSUPPORTED_PREFIX), "{err}");
        assert!(err.contains("文件过大"), "{err}");

        // 真实调用路径：201MB 稀疏文件（set_len 不实际落数据）带 PDF 魔数头 → 拒因必须是尺寸
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big.pdf");
        {
            let mut f = std::fs::File::create(&p).unwrap();
            f.write_all(b"%PDF-1.7 fake header for sniff").unwrap();
            f.set_len(MAX_PARSE_BYTES + 1).unwrap();
        }
        let r = parse_file_smart(p.to_str().unwrap(), "pdf");
        assert!(!r.success);
        assert!(r.error.contains("文件过大"), "{}", r.error);
        assert!(r.error.starts_with(crate::services::file_ingestion::UNSUPPORTED_PREFIX), "{}", r.error);
    }

    /// OCR 截断尾注：仅超上限时产出，文案含上限与真实总页数
    #[test]
    fn ocr_truncation_note_only_when_over_cap() {
        assert!(ocr_truncation_note(300, 300).is_none(), "恰达上限不截断");
        assert!(ocr_truncation_note(50, 300).is_none(), "未达上限不加注");
        assert!(ocr_truncation_note(0, 300).is_none(), "0 页不加注");
        let note = ocr_truncation_note(1234, 300).unwrap();
        assert!(note.contains("仅识别前 300 页"), "{note}");
        assert!(note.contains("全文共 1234 页"), "{note}");
    }

    /// 有界读：头窗（含 cap 截断）/尾窗（含 EOF 收口），绝不整文件入内存
    #[test]
    fn read_window_reads_head_and_tail_windows() {
        use std::io::Write as _;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("w.bin");
        let mut data = b"HEAD".to_vec();
        data.extend(std::iter::repeat(b'x').take(100));
        data.extend_from_slice(b"TAIL");
        std::fs::write(&p, &data).unwrap();

        assert_eq!(read_window(p.to_str().unwrap(), None, 4), b"HEAD".to_vec(), "头窗");
        assert_eq!(read_window(p.to_str().unwrap(), None, 3), b"HEA".to_vec(), "cap 截断");
        let len = data.len() as u64;
        let tail = read_window(p.to_str().unwrap(), Some(len - 4), 4);
        assert_eq!(tail, b"TAIL".to_vec(), "尾窗（offset 后取 cap）");
        let clamped = read_window(p.to_str().unwrap(), Some(len - 4), 999);
        assert_eq!(clamped.len(), 4, "越过 EOF 由实际字节数收口");
        assert!(read_window("/nonexistent/nope", None, 4).is_empty());
    }

    /// zip 细分有界版：小文件精确全读；>64MB 稀疏大文件走尾窗 central-directory 扫描（不整读）
    #[test]
    fn refine_zip_bounded_full_and_tail_window() {
        use std::io::Write as _;
        let dir = tempfile::tempdir().unwrap();

        // 小 OOXML：≤64MB 全读路径（zip crate 精确解析）
        let p1 = dir.path().join("small.docx");
        {
            let mut w = zip::ZipWriter::new(std::fs::File::create(&p1).unwrap());
            w.start_file("word/document.xml", zip::write::FileOptions::default()).unwrap();
            w.write_all(b"<w:document/>").unwrap();
            w.finish().unwrap();
        }
        assert_eq!(refine_zip_bounded(p1.to_str().unwrap()), Some("docx"));

        // 小纯 zip：全读路径判 None
        let p2 = dir.path().join("plain.zip");
        {
            let mut w = zip::ZipWriter::new(std::fs::File::create(&p2).unwrap());
            w.start_file("readme.txt", zip::write::FileOptions::default()).unwrap();
            w.write_all(b"x").unwrap();
            w.finish().unwrap();
        }
        assert_eq!(refine_zip_bounded(p2.to_str().unwrap()), None);

        // >64MB 稀疏文件 + 尾部真实 xlsx zip：>cap 分支必须走尾窗扫描（CD 在尾，条目名可扫）
        let p3 = dir.path().join("huge.zip");
        let mut zip_bytes = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut zip_bytes));
            w.start_file("xl/workbook.xml", zip::write::FileOptions::default()).unwrap();
            w.write_all(b"<x/>").unwrap();
            w.finish().unwrap();
        }
        {
            let f = std::fs::File::create(&p3).unwrap();
            f.set_len(ZIP_REFINE_FULL_CAP + 1).unwrap();
        }
        {
            let mut f = std::fs::OpenOptions::new().append(true).open(&p3).unwrap();
            f.write_all(&zip_bytes).unwrap();
        }
        assert_eq!(
            refine_zip_bounded(p3.to_str().unwrap()),
            Some("xlsx"),
            "尾窗应扫到 central directory 里的 xl/workbook.xml"
        );
    }

    /// ★ 纯 zip 不得被 read_pptx 的空命中截胡：终态是"失败+保留 zip 魔数"，由 ingestion 判 unsupported
    /// （旧行为：Ok("") 当命中 → mk 保魔数 → 内容权威规则误判"已知类型真失败"×3 → 状态错误地=失败）
    #[test]
    fn plain_zip_fails_with_magic_kept_for_unsupported_classify() {
        use std::io::Write as _;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("pack.zip");
        {
            let mut w = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
            w.start_file("readme.txt", zip::write::FileOptions::default()).unwrap();
            w.write_all(b"hello archive, not a document").unwrap();
            w.finish().unwrap();
        }
        let r = parse_file_smart(p.to_str().unwrap(), "zip");
        assert!(!r.success, "纯 zip 不应算解析成功");
        assert!(r.text.is_empty());
        assert_eq!(r.inferred_ext.as_deref(), Some("zip"), "失败出口必须带出 zip 魔数供 unsupported 裁决");
    }

    /// 损坏 docx（OOXML 结构在、正文解析失败）：魔数被 refine 为 docx → 真解析失败（failed 可重试），不是 unsupported
    #[test]
    fn broken_docx_keeps_real_failure_class() {
        use std::io::Write as _;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("broken.docx");
        {
            let mut w = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
            w.start_file("word/document.xml", zip::write::FileOptions::default()).unwrap();
            w.write_all(b"<not-a-valid-document>").unwrap();
            w.finish().unwrap();
        }
        let r = parse_file_smart(p.to_str().unwrap(), "docx");
        assert!(!r.success);
        assert_eq!(r.inferred_ext.as_deref(), Some("docx"), "OOXML 细分保住真实类型 → ingestion 判真失败而非 unsupported");
    }

    #[test]
    fn test_has_real_text() {
        assert!(has_real_text("正常文本"));
        assert!(has_real_text("abc123"));
        assert!(has_real_text("中")); // 非 ASCII 字符
        assert!(!has_real_text(""));
        assert!(!has_real_text("   \n\t  "));
        assert!(!has_real_text("--- *** ---"));
    }

    #[test]
    fn is_near_blank_detects_white_and_text() {
        // 纯白页 → 空白
        let white = vec![255u8; 1200 * 3];
        assert!(is_near_blank_rgb(&white), "纯白应判空白");
        // 白底黑字（模拟 96 DPI 正文页 ~6% 墨量）→ 不应误杀
        let mut text = vec![255u8; 1200 * 3];
        for i in 0..72 { text[i * 3] = 0; text[i * 3 + 1] = 0; text[i * 3 + 2] = 0; }
        assert!(!is_near_blank_rgb(&text), "正文页不应判空白");
        // 灰底设计页（亮度 230 < 250）→ 不判空白，照常送 VLM
        let gray = vec![230u8; 1200 * 3];
        assert!(!is_near_blank_rgb(&gray), "灰底页不应判空白");
        // 零星标点（0.08% < 0.5% 容差）→ 仍判空白
        let mut speck = vec![255u8; 1200 * 3];
        speck[0] = 0; speck[1] = 0; speck[2] = 0;
        assert!(is_near_blank_rgb(&speck), "单点装饰应视为空白扉页");
        // 空缓冲
        assert!(is_near_blank_rgb(&[]));
    }

    #[test]
    fn test_sanitize_chars_removes_control_and_bom() {
        let raw = "\u{FEFF}正常\u{0007}文本\u{0001}";
        let out = sanitize_chars(raw);
        assert!(out.contains("正常"));
        assert!(out.contains("文本"));
        assert!(!out.contains('\u{FEFF}'), "BOM 应被去除");
        assert!(!out.contains('\u{0007}'), "控制字符应被去除");
    }

    #[test]
    fn test_sanitize_chars_strips_page_numbers() {
        let raw = "第一页内容\n--- 第 1 页 ---\n第二页内容";
        let out = sanitize_chars(raw);
        assert!(out.contains("第一页内容"));
        assert!(out.contains("第二页内容"));
        assert!(!out.contains("--- 第 1 页 ---"), "页码标记应被去除: {}", out);
    }

    #[test]
    fn test_sanitize_chars_strips_garbled_lines() {
        let raw = "正常行\n目目目目目目目目目目目目\n另一正常行";
        let out = sanitize_chars(raw);
        assert!(out.contains("正常行"));
        assert!(!out.contains("目目目"), "重复字符噪音行应被去除: {}", out);
    }

    #[test]
    fn test_sanitize_chars_unifies_line_endings() {
        let raw = "行一\r\n行二\r行三";
        let out = sanitize_chars(raw);
        assert!(!out.contains('\r'));
        assert_eq!(out.matches("行").count(), 3);
    }

    #[test]
    fn test_merge_pdf_broken_lines() {
        // 非句尾的行应与下一行合并
        let text = "这是一个被强行\n截断的句子。这是下一句。";
        let out = merge_pdf_broken_lines(text);
        assert!(out.contains("截断的句子。"), "断行应合并: {}", out);
        assert!(out.contains("这是一个被强行 截断的句子。"));

        // 句尾不合并
        let text2 = "完整句。\n下一句。";
        let out2 = merge_pdf_broken_lines(text2);
        assert!(out2.contains("完整句。\n下一句。") || out2.lines().count() == 2);
    }
}


