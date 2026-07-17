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

pub struct ParseResult { pub text: String, pub success: bool }

// ═══════════════════════════════════════════════════════════════
// OCR via Vision API
// ═══════════════════════════════════════════════════════════════

/// 视觉模型配置
struct VisionConfig {
    base_url: String,
    api_key: String,
    model: String,
    api_format: String, // "openai" or "dashscope"
    prompt: String,
}

impl VisionConfig {
    /// 统一默认提示词
    fn default_prompt(_api_format: &str) -> &'static str {
        "从文档图片中提取并组织关键信息，以结构化 Markdown 格式输出。规则：1. 保留所有标题层级（# ## ###）；2. 每个标题后面必须换行，标题和正文之间空一行；3. 表格内容提炼为简洁的文本段落，每条规格独立一行；4. 不要输出原始表格格式；5. 保留所有数值、型号、技术参数；6. 每个特性用一句话概括"
    }
}

/// 视觉模型适配器：每个 api_format + model_name 定义自己的请求构建、响应解析和文本后处理
trait VisionAdapter {
    fn build_request(&self, cfg: &VisionConfig, base64_data_uri: &str) -> Value;
    fn parse_response(&self, body: &Value) -> String;
    #[allow(dead_code)]
    fn default_prompt(&self) -> &'static str;
    /// 模型专属输出文本归一化（在 parse_response 之后调用）
    fn post_process(&self, text: &str) -> String {
        text.to_string() // default: no-op
    }
}

struct OpenaiAdapter;
impl VisionAdapter for OpenaiAdapter {
    fn build_request(&self, cfg: &VisionConfig, base64_data_uri: &str) -> Value {
        json!({
            "model": cfg.model,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "image_url", "image_url": {"url": base64_data_uri}},
                    {"type": "text", "text": &cfg.prompt}
                ]
            }],
            "max_tokens": 4096,
            "temperature": 0.0
        })
    }
    fn parse_response(&self, body: &Value) -> String {
        body["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string()
    }
    fn default_prompt(&self) -> &'static str {
        "从文档图片中提取并组织关键信息，以结构化 Markdown 格式输出。规则：1. 保留所有标题层级（# ## ###）；2. 每个标题后面必须换行，标题和正文之间空一行；3. 表格内容提炼为简洁的文本段落，每条规格独立一行；4. 不要输出原始表格格式；5. 保留所有数值、型号、技术参数；6. 每个特性用一句话概括"
    }
}

struct DashscopeAdapter;
impl VisionAdapter for DashscopeAdapter {
    fn build_request(&self, cfg: &VisionConfig, base64_data_uri: &str) -> Value {
        json!({
            "model": cfg.model,
            "input": {
                "messages": [{
                    "role": "user",
                    "content": [
                        {"image": base64_data_uri},
                        {"text": &cfg.prompt}
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
    fn default_prompt(&self) -> &'static str {
        "从文档图片中提取并组织关键信息，以结构化 Markdown 格式输出。规则：1. 保留所有标题层级（# ## ###）；2. 每个标题后面必须换行，标题和正文之间空一行；3. 表格内容提炼为简洁的文本段落，每条规格独立一行；4. 不要输出原始表格格式；5. 保留所有数值、型号、技术参数；6. 每个特性用一句话概括"
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
    fn build_request(&self, cfg: &VisionConfig, base64_data_uri: &str) -> Value {
        // 与 Dashscope 相同
        json!({
            "model": cfg.model,
            "input": {
                "messages": [{
                    "role": "user",
                    "content": [
                        {"image": base64_data_uri},
                        {"text": &cfg.prompt}
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
    fn default_prompt(&self) -> &'static str {
        "从文档图片中提取并组织关键信息，以结构化 Markdown 格式输出。规则：1. 保留所有标题层级（# ## ###）；2. 每个标题后面必须换行，标题和正文之间空一行；3. 表格内容提炼为简洁的文本段落，每条规格独立一行；4. 不要输出原始表格格式；5. 保留所有数值、型号、技术参数；6. 每个特性用一句话概括"
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

fn get_adapter(api_format: &str, model_name: &str) -> Box<dyn VisionAdapter> {
    match api_format {
        "dashscope" if is_qwen3(model_name) => Box::new(Qwen3VlAdapter),
        "dashscope" => Box::new(DashscopeAdapter),
        _ => Box::new(OpenaiAdapter),
    }
}

/// 从 ai_config 读取视觉模型配置（visionProviders + activeVisionId）
fn get_vision_config() -> Option<VisionConfig> {
    let row = database::get(
        "SELECT ai_config FROM system_config WHERE tenant_id='local-user'",
        &[],
    ).ok().flatten()?;
    let s = row["ai_config"].as_str()?;
    let cfg: Value = serde_json::from_str(s).ok()?;
    let machine = crate::license::get_machine_hash();
    let cfg = crate::crypto::decrypt_config_api_keys(&cfg, &machine);
    let providers = cfg["visionProviders"].as_array()?;
    let active_id = cfg["activeVisionId"].as_str()?;
    let provider = providers.iter().find(|p| p["id"].as_str() == Some(active_id))?;
    let base_url = provider["baseUrl"].as_str()?.to_string();
    let api_key = provider["apiKey"].as_str()?.to_string();
    let model = provider["modelName"].as_str()?.to_string();
    let api_format = provider["apiFormat"].as_str().unwrap_or("openai").to_string();
    let prompt = provider["prompt"].as_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| VisionConfig::default_prompt(&api_format).to_string());
    Some(VisionConfig { base_url, api_key, model, api_format, prompt })
}

/// 读取文件为 base64 编码的 data URI
fn file_to_data_uri(path: &str, mime: &str) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&buf);
    Some(format!("data:{};base64,{}", mime, b64))
}

/// 调用视觉模型 API 进行 OCR（异步）
async fn ocr_with_vision_api(base64_data_uri: &str) -> Result<String, String> {
    let cfg = get_vision_config().ok_or("未配置视觉模型，请在设置页配置视觉模型")?;
    let adapter = get_adapter(&cfg.api_format, &cfg.model);
    eprintln!("[文件解析] OCR 请求: model={}, format={}, prompt_preview={}", cfg.model, cfg.api_format, cfg.prompt.chars().take(60).collect::<String>());

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("HTTP 客户端创建失败: {}", e))?;

    let body = adapter.build_request(&cfg, base64_data_uri);

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

    let resp = req.send().await.map_err(|e| format!("请求失败: {}", e))?;
    let status = resp.status();
    let resp_text = resp.text().await.map_err(|e| format!("读取响应失败: {}", e))?;
    let resp_body: Value = serde_json::from_str(&resp_text).map_err(|_| {
        format!("响应不是JSON: {} (HTTP {})", &resp_text[..resp_text.len().min(500)], status)
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
    eprintln!("[文件解析] OCR 模型返回 (前200字): {}", text.chars().take(200).collect::<String>());
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
    Ok(cleaned)
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

    eprintln!("[文件解析] 所有 PDF 渲染方法均不可用");
    None
}

fn try_load_pdfium() -> Option<pdfium_render::prelude::Pdfium> {
    use pdfium_render::prelude::*;
    // 静态链接：Pdfium::default() 自动使用编译时链接的二进制
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Pdfium::default())) {
        Ok(p) => return Some(p),
        Err(_) => eprintln!("[文件解析] PDFium 静态链接加载失败"),
    }
    None
}

/// 通过 pdfium-render 提取每页纯文本
fn try_pdfium_text_per_page(path: &str) -> Option<Vec<String>> {
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

/// 通过 pdfium-render 渲染 PDF 全部页面为 base64 图片
fn try_pdfium_render(path: &str) -> Option<Vec<String>> {
    let pdfium = try_load_pdfium()?;

    let doc = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pdfium.load_pdf_from_file(path, None)))
        .ok()?.ok()?;
    let pages = doc.pages();
    let count = pages.len();
    if count == 0 { return None; }

    let scale = 200.0 / 72.0;
    let mut results = Vec::with_capacity(count as usize);

    for i in 0..count {
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
        let rgba = bitmap.as_raw_bytes();
        let rgb: Vec<u8> = rgba.chunks(4).flat_map(|c| [c[0], c[1], c[2]]).collect();
        let img = match image::RgbImage::from_raw(width as u32, height as u32, rgb) {
            Some(i) => i,
            None => continue,
        };
        let mut png_buf = std::io::Cursor::new(Vec::new());
        if img.write_to(&mut png_buf, image::ImageFormat::Png).is_err() { continue; }
        let b64 = base64::engine::general_purpose::STANDARD.encode(png_buf.into_inner());
        results.push(format!("data:image/png;base64,{}", b64));
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
    cmd_obj.args(["-png", "-r", "150", path, &output_prefix])
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
        let page_path = tmp_dir.join(format!("page-{}.png", page_num));
        if !page_path.exists() { break; }
        if let Some(uri) = file_to_data_uri(&page_path.to_string_lossy(), "image/png") {
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
    for page in 0..num_pages {
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

/// 同步调用 OCR API 的阻塞封装（供同步 parse_file_smart 调用）
fn ocr_image_sync(base64_uri: &str) -> String {
    // 检查是否已在 tokio runtime 中，避免 "Cannot start a runtime from within a runtime" panic
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        // 已在 runtime 中，直接 block_on
        match handle.block_on(ocr_with_vision_api(base64_uri)) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("[文件解析] OCR 失败: {}", e);
                String::new()
            }
        }
    } else {
        // 不在 runtime 中，创建新 runtime
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(r) => r,
            Err(e) => { eprintln!("[文件解析] tokio 运行时创建失败: {}", e); return String::new(); }
        };
        match rt.block_on(ocr_with_vision_api(base64_uri)) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("[文件解析] OCR 失败: {}", e);
                String::new()
            }
        }
    }
}

/// 对图片文件执行 OCR：读取 → base64 → 视觉 API
fn try_ocr_image(path: &str, mime: &str) -> String {
    match file_to_data_uri(path, mime) {
        Some(uri) => ocr_image_sync(&uri),
        None => {
            eprintln!("[文件解析] 图片读取失败: {}", path);
            String::new()
        }
    }
}

/// 对 base64 data URI 图片进行裁剪：去掉顶部和底部指定比例（默认 8%）
fn crop_image_uri(uri: &str, top_pct: f64, bottom_pct: f64) -> Option<String> {
    let header = "data:image/png;base64,";
    let b64 = uri.strip_prefix(header)?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    let mut img = image::load_from_memory(&bytes).ok()?;
    let (w, h) = (img.width(), img.height());
    let top = (h as f64 * top_pct) as u32;
    let bottom = (h as f64 * bottom_pct) as u32;
    if top + bottom >= h { return None; }
    let cropped = img.crop(0, top, w, h - top - bottom);
    let mut buf = std::io::Cursor::new(Vec::new());
    cropped.write_to(&mut buf, image::ImageFormat::Png).ok()?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(buf.into_inner());
    Some(format!("data:image/png;base64,{}", b64))
}

/// 对 PDF 执行 OCR：PDF 所有页渲染为图片 → 裁剪页眉页脚 → 逐页 OCR → 合并
fn has_real_text(text: &str) -> bool {
    text.chars().any(|c| c.is_ascii_alphanumeric() || c as u32 > 127)
}

fn try_ocr_pdf(path: &str) -> String {
    if get_vision_config().is_none() {
        eprintln!("[文件解析] 未配置视觉模型，跳过 OCR");
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
            eprintln!("[文件解析] 未配置视觉模型，跳过 OCR");
            return String::new();
        }
        let uris = match pdf_to_images_base64(path) {
            Some(u) => u,
            None => {
                eprintln!("[文件解析] PDF 渲染失败: {}", path);
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
        eprintln!("[文件解析] 全量 OCR: {} 页", uris.len());
        let mut full_text = String::new();
        for (i, uri) in uris.iter().enumerate() {
            if !std::path::Path::new(path).exists() {
                eprintln!("[文件解析] 文件已被删除，终止 OCR（已处理 {} / {} 页）", i, uris.len());
                break;
            }
            let cropped = crop_image_uri(uri, 0.04, 0.04).unwrap_or_else(|| uri.clone());
            let ocr_text = ocr_image_sync(&cropped);
            if has_real_text(&ocr_text) {
                if !full_text.is_empty() { full_text.push_str(&format!("\n\n--- 第 {} 页 ---\n\n", i + 1)); }
                full_text.push_str(&ocr_text);
            }
        }
        return full_text;
    }

    // 非 OCR 模式：使用 pdfium 文本提取
    let pdfium_pages = match try_pdfium_text_per_page(path) {
        Some(pages) => pages,
        None => {
            eprintln!("[文件解析] pdfium 不可用");
            return String::new();
        }
    };
    eprintln!("[文件解析] pdfium 提取完成（{} 页）", pdfium_pages.len());
    pdfium_pages.iter().enumerate().fold(String::new(), |mut acc, (i, text)| {
        if !text.is_empty() { if !acc.is_empty() { acc.push_str(&format!("\n\n--- 第 {} 页 ---\n\n", i + 1)); } acc.push_str(text); }
        acc
    })
}
pub fn parse_file_smart(path: &str, hinted_ext: &str) -> ParseResult {
    if !std::path::Path::new(path).exists() {
        return ParseResult { text: String::new(), success: false };
    }

    // ── 读前 4KB 做嗅探 ──
    let head = std::fs::read(path).unwrap_or_default();
    let probe = &head[..head.len().min(4096)];
    let inferred = infer::get(probe);

    // ── 嗅探命中：直接路由 ──
    if let Some(kind) = inferred {
        match kind.mime_type() {
            "application/pdf" => {
                // PDF: OCR（pdfium + 视觉模型）优先 → pdfium 纯文本 → pdf-extract
                let ocr_result = try_ocr_pdf(path);
                if !ocr_result.is_empty() {
                    return ParseResult { success: true, text: ocr_result };
                }
                // 视觉模型未配置 → pdfium 纯文本
                let pdfium_text = try_pdfium_extract(path);
                if !pdfium_text.is_empty() {
                    return ParseResult { success: true, text: pdfium_text };
                }
                eprintln!("[文件解析] PDF 解析失败，回退到 pdf-extract");
                let text = read_pdf(path);
                return ParseResult { success: !text.is_empty(), text };
            }
            "image/png" | "image/jpeg" | "image/webp" | "image/bmp" => {
                let text = try_ocr_image(path, kind.mime_type());
                return ParseResult { success: !text.is_empty(), text };
            }
            "application/zip" | "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
                // ZIP 基类：可能是 docx / xlsx / 纯 zip
                // 优先尝试 DOCX（内部 XML），失败回退 XLSX
                if let Ok(t) = read_docx(path) {
                    return ParseResult { success: !t.is_empty(), text: t };
                }
                if let Ok(t) = read_xlsx(path) {
                    return ParseResult { success: !t.is_empty(), text: t };
                }
                if let Ok(t) = read_pptx(path) {
                    return ParseResult { success: !t.is_empty(), text: t };
                }
            }
            "text/plain" | "text/csv" | "application/json" => {
                let text = std::fs::read_to_string(path).unwrap_or_default();
                return ParseResult { success: !text.is_empty(), text };
            }
            _ => {} // 未知二进制 — 转入后缀路由
        }
    }

    // ── 后缀路由 ──
    let ext = hinted_ext.to_lowercase();
    let text = match ext.as_str() {
        "txt" | "md" | "csv" | "json" => std::fs::read_to_string(path).unwrap_or_default(),
        "docx" => read_docx(path).unwrap_or_default(),
        "xlsx" | "xls" => read_xlsx(path).unwrap_or_default(),
        "pptx" | "ppt" => read_pptx(path).unwrap_or_default(),
        "pdf" => {
            // 后缀 PDF：OCR（pdfium + 视觉模型）优先 → pdfium 纯文本 → pdf-extract
            let ocr = try_ocr_pdf(path);
            if !ocr.is_empty() { ocr } else { try_pdfium_extract(path) }
        }
        "png" | "jpg" | "jpeg" | "webp" | "bmp" => {
            let mime = match ext.as_str() {
                "png" => "image/png", "jpg" | "jpeg" => "image/jpeg",
                "webp" => "image/webp", "bmp" => "image/bmp",
                _ => "image/png",
            };
            try_ocr_image(path, mime)
        }
        _ => String::new(),
    };
    if !text.is_empty() {
        return ParseResult { success: true, text };
    }

    // ── 轮询兜底：依次硬啃 PDF → DOCX → XLSX → TXT ──
    let fallbacks: &[fn(&str) -> String] = &[read_pdf, |p| read_docx(p).unwrap_or_default(), |p| read_xlsx(p).unwrap_or_default(), |p| read_pptx(p).unwrap_or_default(), |p| std::fs::read_to_string(p).unwrap_or_default()];
    for try_parse in fallbacks {
        let t = try_parse(path);
        if !t.is_empty() && t.len() > 20 {
            return ParseResult { success: true, text: t };
        }
    }

    ParseResult { text: String::new(), success: false }
}

/// 兼容旧调用
pub fn parse_file_with_hint(path: &str, ext: &str) -> ParseResult {
    parse_file_smart(path, ext)
}

// ═══════════════════════════════════════════════════════════════
// 两级清洗：基础消毒（分格式） vs 向量化清洗
// ═══════════════════════════════════════════════════════════════

/// 第一步：字符级消毒。所有格式共用——仅移除控制字符 / BOM / 非法 Unicode，
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
                        let parent_list = html[i.saturating_sub(50)..i].to_lowercase();
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

/// 读取 numbering.xml 获取列表编号格式（基于字符串解析，避免 XML API 兼容问题）
fn read_numbering(zip: &mut zip::ZipArchive<std::io::Cursor<Vec<u8>>>) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let Ok(mut num_file) = zip.by_name("word/numbering.xml") else { return map };
    let mut buf = Vec::new();
    let _ = std::io::Read::read_to_end(&mut num_file, &mut buf);
    let text = String::from_utf8_lossy(&buf);
    let mut num_id = String::new();
    let mut cur_ilvl = String::new();
    let mut cur_fmt = String::new();
    let mut in_num = false;
    let mut in_lvl = false;
    for line in text.lines() {
        let t = line.trim();
        if t.contains("<w:num ") { in_num = true; }
        if t.contains("</w:num>") && in_num { in_num = false; num_id.clear(); }
        if t.contains("<w:lvl ") && in_num { in_lvl = true; cur_ilvl.clear(); cur_fmt.clear(); }
        if t.contains("</w:lvl>") && in_lvl {
            if !num_id.is_empty() { map.insert(format!("{}-{}", num_id, cur_ilvl), cur_fmt.clone()); }
            in_lvl = false;
        }
        if in_num {
            if let Some(v) = extract_attr(t, "w:numId", "w:val") { num_id = v; }
        }
        if in_lvl {
            if let Some(v) = extract_attr(t, "w:ilvl", "w:val") { cur_ilvl = v; }
            if let Some(v) = extract_attr(t, "w:numFmt", "w:val") { cur_fmt = v; }
        }
    }
    map
}

fn extract_attr(xml: &str, tag: &str, attr: &str) -> Option<String> {
    let search = format!("{}=\"", attr);
    // Find tag first, then search for attr after it
    if let Some(tag_start) = xml.find(tag) {
        let after_tag = &xml[tag_start..];
        if let Some(val_start) = after_tag.find(&search) {
            let actual_start = val_start + search.len();
            if let Some(end) = after_tag[actual_start..].find('"') {
                return Some(after_tag[actual_start..actual_start+end].to_string());
            }
        }
    }
    None
}

/// DOCX → Markdown 解析：提取标题、表格、列表、正文
fn read_docx_to_markdown(p: &str) -> Result<String, Box<dyn std::error::Error>> {
    use quick_xml::events::Event;
    use quick_xml::Reader;
    use std::io::BufReader;

    fn flush_para(result: &mut String, buf: &mut String, style: &str, numbering: &std::collections::HashMap<String, String>, num_id: &str, ilvl: &str) {
        let text = buf.trim().to_string();
        if text.is_empty() { return; }
        let heading = match style {
            s if s.starts_with("Heading") || s.starts_with("heading") => {
                let level = s.trim_start_matches(|c: char| !c.is_ascii_digit()).chars().next().and_then(|c| c.to_digit(10)).unwrap_or(1);
                format!("{} ", "#".repeat(level as usize))
            }
            _ => String::new()
        };
        if !heading.is_empty() { result.push_str(&format!("{}{}\n\n", heading, text)); }
        else if !num_id.is_empty() {
            let fmt = numbering.get(&format!("{}-{}", num_id, ilvl));
            let prefix = match fmt.map(|s| s.as_str()) {
                Some("decimal") => "1. ", Some("lowerLetter") => "a. ", Some("lowerRoman") => "i. ",
                Some("upperRoman") => "I. ", _ => "- ",
            };
            let indent = "  ".repeat(ilvl.parse::<usize>().unwrap_or(0));
            result.push_str(&format!("{}{}{}\n", indent, prefix, text));
        } else { result.push_str(&format!("{}\n\n", text)); }
        buf.clear();
    }

    let data = std::fs::read(p)?;
    let cursor = std::io::Cursor::new(data);
    let mut zip = zip::ZipArchive::new(cursor)?;
    let numbering = read_numbering(&mut zip);
    let doc = zip.by_name("word/document.xml")?;
    let mut reader = Reader::from_reader(BufReader::new(doc));
    reader.config_mut().trim_text(true);

    let mut buf = Vec::new();
    let mut out = String::new();
    let mut para = String::new();
    let mut style = String::new();
    //
    let mut in_cell = false;
    let mut in_num = false;
    let mut num_id = String::new();
    let mut ilvl = String::new();
    let mut in_tbl = false;
    let mut first_row = true;
    let mut row_text = String::new();
    let mut table_html = String::new(); // buffered <table> rows
    let mut skip_instr = false; // 跳过 w:instrText 域代码指令文本

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let __name = e.name();
                let n = std::str::from_utf8(__name.as_ref()).unwrap_or("");
                match n {
                    "w:p" => {
                        if !in_cell { flush_para(&mut out, &mut para, &style, &numbering, &num_id, &ilvl); }
                        style.clear(); num_id.clear(); ilvl.clear();
                    }
                    "w:tc" => { in_cell = true; para.clear(); }
                    "w:tr" => { row_text.clear(); }
                    "w:tbl" => { in_tbl = true; first_row = true; table_html.clear(); out.push('\n'); }
                    "w:instrText" => { skip_instr = true; }
                    _ => {}
                }
            }
            Ok(Event::Empty(ref e)) => {
                let __name = e.name();
                let n = std::str::from_utf8(__name.as_ref()).unwrap_or("");
                match n {
                    "w:pStyle" => {
                        if let Ok(Some(a)) = e.try_get_attribute("w:val") {
                            style = a.unescape_value().unwrap_or_default().to_string();
                        }
                    }
                    "w:numId" if in_num => {
                        if let Ok(Some(a)) = e.try_get_attribute("w:val") { num_id = a.unescape_value().unwrap_or_default().to_string(); }
                    }
                    "w:ilvl" if in_num => {
                        if let Ok(Some(a)) = e.try_get_attribute("w:val") { ilvl = a.unescape_value().unwrap_or_default().to_string(); }
                    }
                    "w:br" => { para.push('\n'); }
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                let __name = e.name();
                let n = std::str::from_utf8(__name.as_ref()).unwrap_or("");
                match n {
                    "w:p" => {
                        if !in_cell { flush_para(&mut out, &mut para, &style, &numbering, &num_id, &ilvl); }
                       
                    }
                    "w:tc" => {
                        in_cell = false;
                        let t = para.trim().to_string();
                        row_text.push_str(&format!("<td>{}</td>", t));
                        para.clear();
                    }
                    "w:tr" => {
                        if in_tbl {
                            let html_row = if first_row {
                                row_text.replace("<td>", "<th>").replace("</td>", "</th>")
                            } else {
                                row_text.clone()
                            };
                            table_html.push_str(&format!("<tr>{}</tr>\n", html_row));
                            row_text.clear();
                            first_row = false;
                        }
                    }
                    "w:tbl" => {
                        if !table_html.is_empty() {
                            out.push_str("<table>\n");
                            out.push_str(&table_html);
                            out.push_str("</table>\n\n");
                            table_html.clear();
                        }
                        in_tbl = false;
                    }
                    "w:numPr" => { in_num = false; }
                    "w:instrText" => { skip_instr = false; }
                    _ => {}
                }
            }
            Ok(Event::Text(e)) => {
                if !skip_instr {
                    if let Ok(s) = e.unescape() { para.push_str(&s); }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    flush_para(&mut out, &mut para, &style, &numbering, &num_id, &ilvl);

    // collapse 3+ newlines to 2
    let mut result = String::new();
    let mut empty = 0;
    for ch in out.chars() {
        if ch == '\n' { empty += 1; if empty <= 2 { result.push(ch); } }
        else { empty = 0; result.push(ch); }
    }
    Ok(result.trim().to_string())
}

/// DOCX 解析：用 quick-xml 提取 word/document.xml 中的纯文本节点（保留旧接口调用但改用新实现）
fn read_docx(p: &str) -> Result<String, Box<dyn std::error::Error>> {
    read_docx_to_markdown(p)
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

            let mut rows: Vec<Vec<String>> = Vec::new();
            for row in ws.rows() {
                let cells: Vec<String> = row
                    .iter()
                    .map(|c| match c {
                        Data::Empty => String::new(),
                        Data::Error(e) => format!("#ERR {}", e),
                        Data::String(s) => {
                            let t = s.trim().replace('\r', "").replace('\n', " ");
                            let digit_count = t
                                .chars()
                                .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '_' || *c == '-')
                                .count();
                            let is_code = t.len() > 5
                                && digit_count as f64 / t.len() as f64 > 0.55
                                && t.as_bytes()
                                    .first()
                                    .map(|b| b.is_ascii_lowercase() || *b == b'_')
                                    .unwrap_or(false)
                                && t.contains('.')
                                && t.chars()
                                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-');
                            if is_code {
                                String::new()
                            } else {
                                t
                            }
                        }
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

                // Filter formula cache rows: 首个单元格以 ,productTotalPrice 等开头 → 整行跳过
                let formula_prefixes = [",productTotalPrice", ",serviceTotalPrice", ",projectTotalPrice"];
                if cells
                    .first()
                    .map(|c| formula_prefixes.iter().any(|p| c.starts_with(p)))
                    .unwrap_or(false)
                {
                    continue;
                }

                if cells.iter().all(|c| c.is_empty()) {
                    continue;
                }
                rows.push(cells);
            }

            if rows.is_empty() {
                continue;
            }

            let max_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
            if max_cols == 0 {
                continue;
            }

            // Detect empty columns — 跳过每一行都空的列
            let mut non_empty_cols = vec![false; max_cols];
            for row in &rows {
                for (ci, cell) in row.iter().enumerate() {
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
            for (ri, row) in rows.iter().enumerate() {
                result.push_str("<tr>");
                let tag = if ri == 0 { "th" } else { "td" };
                for &ci in &col_map {
                    // 跳过被合并覆盖的非起始单元格
                    if skip_cells.contains(&(ri, ci)) {
                        continue;
                    }
                    let cell = row.get(ci).map(|s| s.as_str()).unwrap_or("");
                    let cleaned = cell
                        .replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;");
                    let attrs = match merge_map.get(&(ri, ci)) {
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
                                let t = s.trim();
                                let digit_ratio = t
                                    .chars()
                                    .filter(|c| {
                                        c.is_ascii_digit() || *c == '.' || *c == '_' || *c == '-'
                                    })
                                    .count() as f64
                                    / t.len().max(1) as f64;
                                if t.len() > 5
                                    && digit_ratio > 0.55
                                    && t.contains('.')
                                    && t.chars().all(|c| {
                                        c.is_ascii_alphanumeric()
                                            || c == '.'
                                            || c == '_'
                                            || c == '-'
                                    }) {
                                } else {
                                    result.push_str(&format!("{} ", t));
                                }
                            }
                            Data::Float(f) => {
                                if *f != 0.0 {
                                    result.push_str(&format!("{} ", f));
                                }
                            }
                            Data::Int(i) => {
                                if *i != 0 {
                                    result.push_str(&format!("{} ", i));
                                }
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


