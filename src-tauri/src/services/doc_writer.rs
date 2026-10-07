//! 文档写入：AI 设计的 JSON 内容 → Word(.docx) / Excel(.xlsx) → CAS 存储 → file_id
//!
//! 生成的文档通过 file_assets + file_relations(entity_type='generated') 入库：
//! - 复用现有 file_preview / file_export / file_copy_to_clipboard（预览/下载/复制）
//! - entity_type='generated' 不出现在资料库/项目文件列表（相关查询均带类型过滤）
//! - generated_docs 表保存原始 content JSON：多轮修改时 action=read 回读
//! - 生命周期由用户驱动：kb_delete_session 删会话时联动删除（含跨会话引用检查），
//!   无自动过期——文件与会话同生命周期，聊天中的文档卡片永不失效
//! - 生成后经 notify_file_upload 与上传文件同管道解析入知识库（检索侧按需隔离）

use crate::commands::file_commands::flat_disk_path;
use crate::database;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

// ── docx 预设样式（唯一真相源：字号字体行距全部由系统预设，AI 只选 text_type） ──
/// 正文/表格/页眉页脚统一字体（rwml rFonts 四体同设，中英文一致）
const DOC_PRESET_FONT: &str = "宋体";
/// 正文小四 12pt（half-pt=24）
const DOC_PRESET_BODY_HALF_PT: u16 = 24;
/// 附注/图注 text_type=caption 小五 9pt（half-pt=18）
const DOC_PRESET_CAPTION_HALF_PT: u16 = 18;
/// 表格小四 12pt（half-pt=24，与正文一致；caption 仍小五）
const DOC_PRESET_TABLE_HALF_PT: u16 = 24;
/// 页眉页脚小五 9pt（half-pt=18）
const DOC_PRESET_RUNNING_HALF_PT: u16 = 18;
/// 正文行距 1.5 倍（rwml line_pct ×240 → w:line=360 lineRule=auto）
const DOC_PRESET_LINE_PCT: f32 = 1.5;
/// 标题 1-6 级字号（half-pt）：小二18/三号16/四号14/12/11/11 pt，字体同宋体
/// （覆盖 rwml 内置西文惯例 32/28/…；标题行距与正文同为 1.5 倍）
const DOC_PRESET_HEADING_HALF_PT: [u16; 6] = [36, 32, 28, 24, 22, 22];

// ── xlsx 预设 ──
const XLSX_PRESET_FONT: &str = "宋体";
const XLSX_PRESET_SIZE_PT: f64 = 11.0;
const XLSX_CAPTION_SIZE_PT: f64 = 9.0;

/// text_type → 字号（half-pt）：caption=小五9pt，body/缺省=按作用域默认
fn text_type_size_half_pt(r: &Value, body_half_pt: u16) -> u16 {
    if r["text_type"].as_str() == Some("caption") {
        DOC_PRESET_CAPTION_HALF_PT
    } else {
        body_half_pt
    }
}

// ═══════════════ 行内 Markdown → 格式（确定性渲染，AI 只管原样传文） ═══════════════
/// 行内样式段（md_inline_runs 产出）
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub(crate) struct InlineStyle {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
}

/// 解析行内 **粗体** / *斜体* / `代码`（含 __/_ 变体与 ***组合***）为样式段序列。
/// 保守规则：必须成对、开侧后非空白、闭侧前非空白；单 * / _ 的两侧不得紧贴 ASCII 字母数字
/// （护住 1*2*3、1920*1080、user_name 这类工程文本）；未闭合标记原样输出不吞字。
pub(crate) fn md_inline_runs(text: &str) -> Vec<(String, InlineStyle)> {
    let mut out = Vec::new();
    parse_emph(text, InlineStyle::default(), &mut out);
    out.iter().for_each(|(s, _)| {
        debug_assert!(!s.is_empty());
    });
    out
}

/// 只保留纯文本（剥行内标记）：标题/目录/超链标签等自带粗体的位置使用
pub(crate) fn md_strip_inline(text: &str) -> String {
    md_inline_runs(text).into_iter().map(|(t, _)| t).collect()
}

/// 整段被 ** 包裹（用于 Excel 单样式单元格 → 整格加粗）
pub(crate) fn md_fully_bold(text: &str) -> bool {
    let t = text.trim();
    t.len() > 4 && t.starts_with("**") && t.ends_with("**") && !t[2..t.len() - 2].contains("**")
}

fn parse_emph(s: &str, base: InlineStyle, out: &mut Vec<(String, InlineStyle)>) {
    if let Some((pre, inner, post, bold, italic)) = take_emph(s) {
        parse_emph(&pre, base, out);
        parse_emph(&inner, InlineStyle { bold: base.bold || bold, italic: base.italic || italic, ..base }, out);
        parse_emph(&post, base, out);
        return;
    }
    parse_code(s, base, out);
}

fn parse_code(s: &str, base: InlineStyle, out: &mut Vec<(String, InlineStyle)>) {
    let mut rest = s;
    while let Some(open) = rest.find('`') {
        match rest[open + 1..].find('`') {
            Some(rel) if rel > 0 => {
                let close = open + 1 + rel;
                if open > 0 {
                    out.push((rest[..open].to_string(), base));
                }
                out.push((rest[open + 1..close].to_string(), InlineStyle { code: true, ..base }));
                rest = &rest[close + 1..];
            }
            _ => break,
        }
    }
    if !rest.is_empty() {
        out.push((rest.to_string(), base));
    }
}

/// 找第一个合法强调对：返回 (前文, 内文, 后文, bold, italic)
fn take_emph(s: &str) -> Option<(String, String, String, bool, bool)> {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let findable: [(&str, bool, bool); 6] =
        [("***", true, true), ("**", true, false), ("*", false, true), ("___", true, true), ("__", true, false), ("_", false, true)];
    for (marker, bold, italic) in findable {
        let m: Vec<char> = marker.chars().collect();
        let ml = m.len();
        let strict_boundary = ml == 1; // 单 * / 单 _：两侧不得紧贴 ASCII 字母数字
        let mut i = 0usize;
        while i + ml <= n {
            if chars[i..i + ml] != m[..] {
                i += 1;
                continue;
            }
            let prev_ok = i == 0 || !chars[i - 1].is_ascii_alphanumeric();
            let after = i + ml;
            let ok_open = after < n
                && !chars[after].is_whitespace()
                && chars[after] != m[0]
                && (!strict_boundary || prev_ok);
            if !ok_open {
                i += ml;
                continue;
            }
            let mut j = after;
            let mut close = None;
            while j + ml <= n {
                if chars[j..j + ml] == m[..] && j > after && !chars[j - 1].is_whitespace() {
                    let next_ok = j + ml >= n || !chars[j + ml].is_ascii_alphanumeric() || !strict_boundary;
                    if next_ok {
                        close = Some(j);
                        break;
                    }
                }
                j += 1;
            }
            if let Some(c) = close {
                let pre: String = chars[..i].iter().collect();
                let inner: String = chars[after..c].iter().collect();
                let post: String = chars[c + ml..].iter().collect();
                return Some((pre, inner, post, bold, italic));
            }
            i += ml;
        }
    }
    None
}

/// 正文 run（预设宋体小四）
fn body_run(text: &str) -> rwml::Run {
    rwml::RunBuilder::new(text).font(DOC_PRESET_FONT).size_half_pt(DOC_PRESET_BODY_HALF_PT).build()
}

/// 正文 run 序列：行内 markdown（**粗**/*斜*/`code`）解析为多 run；
/// 段落样式对象（text_type/bold）作基底，inline 叠加覆盖
fn body_runs(text: &str, r: Option<&Value>) -> Vec<rwml::Run> {
    let size = r
        .map(|v| text_type_size_half_pt(v, DOC_PRESET_BODY_HALF_PT))
        .unwrap_or(DOC_PRESET_BODY_HALF_PT);
    let base_bold = r.and_then(|v| v["bold"].as_bool()).unwrap_or(false);
    md_inline_runs(text)
        .into_iter()
        .map(|(t, st)| {
            let mut b = rwml::RunBuilder::new(t.as_str())
                .font(if st.code { "Consolas" } else { DOC_PRESET_FONT })
                .size_half_pt(size);
            if st.bold || base_bold {
                b = b.bold();
            }
            if st.italic {
                b = b.italic();
            }
            b.build()
        })
        .collect()
}

/// 正文段落（预设 run + 1.5 倍行距），替代 builder.paragraph 使预设生效
fn body_paragraph(builder: rwml::DocBuilder, text: &str) -> rwml::DocBuilder {
    body_paragraph_from(builder, text, None)
}
fn body_paragraph_from(builder: rwml::DocBuilder, text: &str, r: Option<&Value>) -> rwml::DocBuilder {
    let pb = rwml::ParagraphBuilder::new()
        .runs(body_runs(text, r))
        .line_pct(DOC_PRESET_LINE_PCT);
    builder.rich_paragraph(pb.build())
}

pub struct GeneratedDoc {
    pub file_id: String,
    pub file_name: String,
    pub file_type: String,
    pub file_size: u64,
}

// ═══════════════════════════════════════════════════════════════
// 公共 API
// ═══════════════════════════════════════════════════════════════

/// 生成 Word 文档并入库，返回文档对象。owner_session：归属会话 id（relation 行 entity_id），
/// 无会话上下文传空串（由 sweep 的 metadata 兜底分支保护）。
pub fn write_docx(filename: &str, content: &Value, metadata: Option<&Value>, owner_session: &str) -> Result<GeneratedDoc, String> {
    let bytes = build_docx(content, metadata)?;
    save_bytes(&bytes, filename, "docx", content, owner_session)
}

/// 生成 Excel 文档并入库，返回文档对象
pub fn write_xlsx(filename: &str, content: &Value, owner_session: &str) -> Result<GeneratedDoc, String> {
    let bytes = build_xlsx(content)?;
    save_bytes(&bytes, filename, "xlsx", content, owner_session)
}

/// 回读已生成文档的原始 content JSON（多轮修改：AI 先 read 结构再整体重新生成）
pub fn read_content(file_id: &str) -> Result<Value, String> {
    let mut fid = file_id.trim().to_string();
    if fid.is_empty() {
        return Err("缺少 source_file_id".to_string());
    }
    // 兼容传入 file_relations.id
    let known = database::get("SELECT 1 FROM generated_docs WHERE file_id=?1", &[json!(&fid)])
        .ok()
        .flatten()
        .is_some();
    if !known {
        if let Ok(Some(r)) = database::get("SELECT file_id FROM file_relations WHERE id=?1", &[json!(&fid)]) {
            if let Some(x) = r["file_id"].as_str().filter(|s| !s.is_empty()) {
                fid = x.to_string();
            }
        }
    }
    let row = database::get(
        "SELECT gd.content, fa.file_ext, fa.original_name FROM generated_docs gd \
         INNER JOIN file_assets fa ON fa.id = gd.file_id WHERE gd.file_id=?1",
        &[json!(&fid)],
    )
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "未找到该文档（可能已超过保留期被清理）".to_string())?;
    let content: Value =
        serde_json::from_str(row["content"].as_str().unwrap_or("{}")).unwrap_or_else(|_| json!({}));
    Ok(json!({
        "success": true,
        "file_id": fid,
        "file_name": row["original_name"].as_str().unwrap_or(""),
        "file_type": row["file_ext"].as_str().unwrap_or(""),
        "content": content,
    }))
}

/// 长文分批成文：把新章节块并入已有 content（纯函数，可单测）。
/// new 接受 {children:[...]} 或裸块数组；只追加 children，title/页眉页脚等顶层字段沿用旧文档。
pub(crate) fn merge_docx_children(old: &Value, new: &Value) -> Result<Value, String> {
    let mut merged = old.clone();
    let new_children: Vec<Value> = match new.as_array() {
        Some(a) => a.clone(),
        None => new
            .get("children")
            .and_then(|c| c.as_array())
            .cloned()
            .ok_or("append 的 content 需为 {children:[块,...]} 或块数组")?,
    };
    if new_children.is_empty() {
        return Err("append 的 children 为空，没有可追加的内容".to_string());
    }
    let obj = merged.as_object_mut().ok_or("原文档 content 非对象，无法追加")?;
    let mut children = obj
        .get("children")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();
    children.extend(new_children);
    obj.insert("children".to_string(), json!(children));
    Ok(merged)
}

/// 原地覆盖保存：合并后的 content 重新渲染为 docx 字节，**沿用同一 file_id**
/// （更新 file_assets 字节指向/大小/hash 与 generated_docs.content），聊天里的文档卡片持续指向同一文件。
fn save_overwrite(bytes: &[u8], file_id: &str, file_name: &str, content: &Value) -> Result<GeneratedDoc, String> {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let hash = hex::encode(&hasher.finalize()[..32]);
    let size = bytes.len() as i64;
    let disk = flat_disk_path(&hash);
    if disk.is_empty() {
        return Err("存储路径无效".to_string());
    }
    if !std::path::Path::new(&disk).exists() {
        if let Some(p) = std::path::Path::new(&disk).parent() {
            std::fs::create_dir_all(p).map_err(|e| format!("创建存储目录失败: {}", e))?;
        }
        std::fs::write(&disk, bytes).map_err(|e| format!("写入文件失败: {}", e))?;
    }
    database::run(
        "UPDATE file_assets SET content_hash=?1, file_size=?2, updated_at=datetime('now','localtime') WHERE id=?3",
        &[json!(&hash), json!(size), json!(file_id)],
    )
    .map_err(|e| e.to_string())?;
    database::run(
        "INSERT OR REPLACE INTO generated_docs(file_id, content, created_at) VALUES(?1,?2,datetime('now','localtime'))",
        &[json!(file_id), json!(content.to_string())],
    )
    .map_err(|e| e.to_string())?;
    // 刷新知识库正文仅对已定稿文件生效（草稿期 append 不入索引，定稿由 finalize 统一进管道）
    let ready = database::get("SELECT status FROM file_assets WHERE id=?1", &[json!(file_id)])
        .ok().flatten()
        .map(|r| r["status"].as_str() != Some("draft"))
        .unwrap_or(true);
    if ready {
        crate::services::knowledge_lifecycle::notify_file_upload(file_id);
    }
    log::info!("[文档生成] append 已原地更新 file_id={} 累计size={}B", file_id, size);
    Ok(GeneratedDoc {
        file_id: file_id.to_string(),
        file_name: file_name.to_string(),
        file_type: "docx".to_string(),
        file_size: size as u64,
    })
}

/// 向已生成的 docx 追加章节块并原地重建（长方案分块成文的组装端）。
/// 文档属性（metadata）未持久化于 generated_docs，追加后按系统默认重建——可接受降级。
pub fn append_docx(source_file_id: &str, new_content: &Value, owner_session: &str) -> Result<GeneratedDoc, String> {
    let cur = read_content(source_file_id)?;
    if cur["file_type"].as_str() != Some("docx") {
        return Err("append 目前仅支持 docx；xlsx 请读取结构后整体重新 create".to_string());
    }
    let merged = merge_docx_children(&cur["content"], new_content)?;
    let bytes = build_docx(&merged, None)?;
    let doc = save_overwrite(&bytes, cur["file_id"].as_str().unwrap_or(source_file_id), cur["file_name"].as_str().unwrap_or("文档"), &merged)?;
    // 跨会话续写：为当前会话补挂 generated 归属行（unique(file_id,entity_type,entity_id) 幂等），
    // 保证删除本会话时该资产引用计数正确（他会话仍引用则资产保留）
    if !owner_session.is_empty() {
        let _ = database::run(
            "INSERT OR IGNORE INTO file_relations(id,file_id,entity_type,entity_id,custom_name,creator) \
             VALUES(?1,?2,'generated',?3,?4,'AI')",
            &[json!(uuid::Uuid::new_v4().to_string()), json!(doc.file_id), json!(owner_session), json!(doc.file_name)],
        );
    }
    Ok(doc)
}

/// 回合末定稿翻正：本轮持久化到消息卡片的生成文档置 ready 并首次进解析管道。
/// 只在真正发生 draft→ready 时 enqueue（CAS 复用已定稿文件/已定稿续写不重复入队）。
pub fn finalize_generated_docs(docs: &[Value]) {
    for d in docs {
        let Some(fid) = d["file_id"].as_str().filter(|s| !s.is_empty()) else { continue };
        let flipped = database::run(
            "UPDATE file_assets SET status='ready', updated_at=datetime('now','localtime') WHERE id=?1 AND status='draft'",
            &[json!(fid)],
        )
        .map(|r| r.changes)
        .unwrap_or(0)
            > 0;
        if flipped {
            crate::services::knowledge_lifecycle::notify_file_upload(fid);
            log::info!("[文档生成] 定稿翻正 file_id={}", fid);
        }
    }
}

/// 启动清理：1) 崩溃/取消残留的超期草稿（48h）按删除路径整链回收；
/// 2) 仅回收孤儿快照（资产已被别处删除的残留记录）。不再按时间自动过期 ready 文档。
pub fn cleanup_generated_docs() {
    let stale = database::all(
        "SELECT id FROM file_assets WHERE status='draft' AND created_at < datetime('now','localtime','-48 hours')",
        &[],
    )
    .unwrap_or_default();
    for r in &stale {
        if let Some(fid) = r["id"].as_str() {
            delete_generated_doc(fid);
        }
    }
    if !stale.is_empty() {
        log::info!("[文档清理] 回收超期草稿 {} 个（生成中断）", stale.len());
    }
    let n = database::run(
        "DELETE FROM generated_docs WHERE file_id NOT IN (SELECT id FROM file_assets)",
        &[],
    ).map(|r| r.changes).unwrap_or(0);
    if n > 0 {
        log::info!("[文档清理] 回收孤儿生成文档快照 {} 条（资产已不存在）", n);
    }
}

/// 删除单个生成文档（删除会话时联动调用）：
/// 删 generated 关系行 + content 快照 → 零引用释放（知识库/物理文件/资产行）。
/// 仍被其他关系（项目/资料库/邮件附件等）引用时保留资产本体。
pub fn delete_generated_doc(file_id: &str) {
    if file_id.is_empty() {
        return;
    }
    let _ = database::run(
        "DELETE FROM file_relations WHERE file_id=?1 AND entity_type='generated'",
        &[json!(file_id)],
    );
    let _ = database::run("DELETE FROM generated_docs WHERE file_id=?1", &[json!(file_id)]);
    crate::commands::file_commands::release_asset_if_orphan(file_id);
}

/// 按归属会话收集生成的文档资产 id（在删除会话行/消息之前调用——与 collect_entity_asset_ids 同构）
pub fn collect_generated_file_ids(session_ids: &[String]) -> Vec<String> {
    if session_ids.is_empty() {
        return vec![];
    }
    let ph = session_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let mut vals: Vec<Value> = Vec::with_capacity(session_ids.len() + 1);
    vals.extend(session_ids.iter().map(|s| json!(s)));
    database::all(
        &format!("SELECT DISTINCT file_id FROM file_relations WHERE entity_type='generated' AND entity_id IN ({})", ph),
        &vals,
    )
    .unwrap_or_default()
    .iter()
    .filter_map(|r| r["file_id"].as_str().filter(|s| !s.is_empty()).map(String::from))
    .collect()
}

/// 会话域统一释放（会话/消息已删后调用）：删这批会话的 generated 关系行 →
/// 资产再无 generated 引用时清 content 快照 → release_asset_if_orphan 按剩余引用
/// （他会话 generated 行 / 资料库 / 项目 / 邮件等）决定物理文件与资产行是否回收。
pub fn release_generated_for_sessions(session_ids: &[String], file_ids: &[String]) {
    if session_ids.is_empty() || file_ids.is_empty() {
        return;
    }
    let ph = session_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    for fid in file_ids {
        let mut vals: Vec<Value> = vec![json!(fid)];
        vals.extend(session_ids.iter().map(|s| json!(s)));
        let _ = database::run(
            &format!("DELETE FROM file_relations WHERE file_id=? AND entity_type='generated' AND entity_id IN ({})", ph),
            &vals,
        );
        let remains: i64 = database::get(
            "SELECT COUNT(*) AS c FROM file_relations WHERE file_id=?1 AND entity_type='generated'",
            &[json!(fid)],
        )
        .ok()
        .flatten()
        .and_then(|r| r["c"].as_i64())
        .unwrap_or(0);
        if remains == 0 {
            let _ = database::run("DELETE FROM generated_docs WHERE file_id=?1", &[json!(fid)]);
        }
        crate::commands::file_commands::release_asset_if_orphan(fid);
    }
    log::info!("[文档回收] {} 个会话释放处理了 {} 个生成文档资产", session_ids.len(), file_ids.len());
}

// ═══════════════════════════════════════════════════════════════
// 入库：CAS 落盘 + file_assets + file_relations + generated_docs
// ═══════════════════════════════════════════════════════════════

fn save_bytes(bytes: &[u8], filename: &str, ext: &str, content: &Value, owner_session: &str) -> Result<GeneratedDoc, String> {
    if bytes.is_empty() {
        return Err("生成的文件内容为空".to_string());
    }
    let file_name_base = build_file_name(filename, ext);
    // 同秒撞名兜底：目标名已被占用（同秒双生成不同内容）→ 扩展名前追加序号
    let dotted = format!(".{}", ext);
    let mut file_name = file_name_base.clone();
    let mut seq = 1u32;
    while seq <= 99 {
        let taken = database::get("SELECT 1 FROM file_assets WHERE original_name=?1", &[json!(&file_name)])
            .ok().flatten().is_some();
        if !taken { break; }
        let core = if file_name_base.to_lowercase().ends_with(&dotted) {
            file_name_base[..file_name_base.len() - dotted.len()].to_string()
        } else { file_name_base.clone() };
        file_name = format!("{}_{}.{}", core, seq, ext);
        seq += 1;
    }
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let hash = hex::encode(&hasher.finalize()[..32]);
    let size = bytes.len() as i64;
    log::info!("[文档生成] save_bytes: file_name={} ext={} size={}B hash={}", file_name, ext, size, &hash[..16]);

    // CAS 落盘（内容相同自动去重复用）
    let disk = flat_disk_path(&hash);
    if disk.is_empty() {
        return Err("存储路径无效".to_string());
    }
    if !std::path::Path::new(&disk).exists() {
        if let Some(p) = std::path::Path::new(&disk).parent() {
            std::fs::create_dir_all(p).map_err(|e| format!("创建存储目录失败: {}", e))?;
        }
        std::fs::write(&disk, bytes).map_err(|e| format!("写入文件失败: {}", e))?;
    }

    // file_assets：复用或新建
    let file_id = match database::get("SELECT id FROM file_assets WHERE content_hash=?1 LIMIT 1", &[json!(&hash)])
        .ok()
        .flatten()
        .and_then(|r| r["id"].as_str().map(String::from))
    {
        Some(existing) => existing,
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            database::run(
                "INSERT INTO file_assets(id,file_ext,file_size,content_hash,original_name,status,created_at,updated_at) \
                 VALUES(?1,?2,?3,?4,?5,'draft',datetime('now','localtime'),datetime('now','localtime'))",
                &[json!(&id), json!(ext), json!(size), json!(&hash), json!(&file_name)],
            )
            .map_err(|e| e.to_string())?;
            id
        }
    };

    // file_relations(entity_type='generated')：预览/下载/复制的解析依赖此关系；
    // entity_id=归属会话（删除会话与项目/笔记等域同构回收；无会话上下文为空由 sweep 兜底）。
    // 不走 add_relation：避免 upsert 进业务关系图谱（生成文档不是业务实体）。
    database::run(
        "INSERT OR IGNORE INTO file_relations(id,file_id,entity_type,entity_id,custom_name,creator) \
         VALUES(?1,?2,'generated',?4,?3,'AI')",
        &[json!(uuid::Uuid::new_v4().to_string()), json!(&file_id), json!(&file_name), json!(owner_session)],
    )
    .map_err(|e| e.to_string())?;
    // CAS 复用同内容时刷新显示名（同内容不同文件名的边界场景）
    database::run(
        "UPDATE file_relations SET custom_name=?1 WHERE file_id=?2 AND entity_type='generated'",
        &[json!(&file_name), json!(&file_id)],
    )
    .ok();

    // 原始 content 快照：多轮 read 回读（OR REPLACE 刷新）
    database::run(
        "INSERT OR REPLACE INTO generated_docs(file_id, content, created_at) \
         VALUES(?1,?2,datetime('now','localtime'))",
        &[json!(&file_id), json!(content.to_string())],
    )
    .map_err(|e| e.to_string())?;

    // ★ 草稿态：不进解析管道（半截文档不入知识库、不出现在任何列表）——
    // 回合末持久化文档卡片时 finalize_generated_docs 统一翻正并入解析
    log::info!("[文档生成] {} 已保存(草稿) file_id={} size={}B", file_name, file_id, size);
    Ok(GeneratedDoc {
        file_id,
        file_name,
        file_type: ext.to_string(),
        file_size: size as u64,
    })
}

/// 文件名消毒：去路径分隔符与非法字符，保证非空并带扩展名。
/// 秒级时间戳后缀区分会话内同名文件：设备清单 → 设备清单_20260929_143052.docx；
/// 输入已带扩展名时在扩展名前插时间戳（不双拼）。AI 无需自带日期（prompt 已声明）。
fn build_file_name(filename: &str, ext: &str) -> String {
    const ILLEGAL: [char; 9] = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    let base: String = filename
        .trim()
        .chars()
        .map(|c| if ILLEGAL.contains(&c) || (c as u32) < 0x20 { '_' } else { c })
        .collect();
    let base = base.trim().trim_matches('.');
    // 截断预算：core ≤78 + '_' + 15位时间戳 + '.' + ext(≤4) ≤ 100
    let base: String = base.chars().take(78).collect();
    let base = if base.is_empty() { "文档".to_string() } else { base };
    let dotted = format!(".{}", ext);
    let core = if base.to_lowercase().ends_with(&dotted) {
        base[..base.len() - dotted.len()].to_string()
    } else {
        base
    };
    let core = core.trim_end_matches(['.', '_']);
    let core = if core.is_empty() { "文档" } else { core };
    let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
    format!("{}_{}.{}", core, ts, ext)
}

// ═══════════════════════════════════════════════════════════════
// Word (.docx) 构建 — rwml DocBuilder
// ═══════════════════════════════════════════════════════════════

/// header/footer 文本中的 {PAGE}/{NUMPAGES} 占位符 → Word 页码域（fldSimple）。
/// 不转换时 Word 把 {PAGE} 当字面文本显示（AI 生成页脚的常见模板写法）。
/// 无占位 → 原 plain 路径；有占位 → 按段拆成 文本run + 域run 混排（footer_runs/header_runs）。
fn apply_running_text(builder: rwml::DocBuilder, text: String, is_header: bool) -> rwml::DocBuilder {
    let segments = split_running_placeholders(&text);
    if segments.iter().all(|(_, instr)| instr.is_none()) {
        // 无占位：仍走 runs 以套页眉页脚预设（宋体小五 9pt）
        let run = running_run(&text, None);
        return if is_header { builder.header_runs(vec![run]) } else { builder.footer_runs(vec![run]) };
    }
    // 域 run 的 cached 文本给 "1"（与 rwml 自带 page_numbers 段一致），Word 打开时按域指令刷新为真实页码
    let runs: Vec<rwml::Run> = segments
        .into_iter()
        .map(|(seg_text, instr)| running_run(&seg_text, instr))
        .collect();
    if is_header { builder.header_runs(runs) } else { builder.footer_runs(runs) }
}

/// 页眉页脚 run：宋体小五 9pt 预设（域 run 的 cached 文本同此样式）
fn running_run(text: &str, field: Option<&str>) -> rwml::Run {
    let mut b = rwml::RunBuilder::new(text)
        .font(DOC_PRESET_FONT)
        .size_half_pt(DOC_PRESET_RUNNING_HALF_PT);
    if let Some(ins) = field {
        b = b.field(ins);
    }
    b.build()
}

/// 是否存在 footer 块（递归扫 group/未知容器）——决定是否兜底注入页码页脚
fn has_footer_block(children: &[Value]) -> bool {
    children.iter().any(|c| {
        if !c.is_object() {
            return false;
        }
        if c["type"].as_str().unwrap_or("").trim().eq_ignore_ascii_case("footer") {
            return true;
        }
        c["children"].as_array().map_or(false, |nested| has_footer_block(nested))
    })
}

/// header/footer 文本占位符分段（纯函数，可单测）：
/// 返回 (文本段, Some(域指令) | None)；{PAGE}→PAGE、{NUMPAGES}→NUMPAGES，其余原样成段。
fn split_running_placeholders(text: &str) -> Vec<(String, Option<&'static str>)> {
    const PLACEHOLDERS: [(&str, &str); 2] = [("{PAGE}", "PAGE"), ("{NUMPAGES}", "NUMPAGES")];
    let mut out: Vec<(String, Option<&'static str>)> = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        // 找最早出现的占位符
        let mut earliest: Option<(usize, usize, &'static str)> = None;
        for (pat, instr) in PLACEHOLDERS {
            if let Some(pos) = rest.find(pat) {
                if earliest.map_or(true, |(ep, _, _)| pos < ep) {
                    earliest = Some((pos, pat.len(), instr));
                }
            }
        }
        match earliest {
            Some((pos, len, instr)) => {
                if pos > 0 {
                    out.push((rest[..pos].to_string(), None));
                }
                // 域段：cached result 文本 "1"（与 rwml 自带 page_numbers 段一致），Word 打开按域指令刷新真值
                out.push(("1".to_string(), Some(instr)));
                rest = &rest[pos + len..];
            }
            None => {
                out.push((rest.to_string(), None));
                break;
            }
        }
    }
    out
}

fn build_docx(content: &Value, metadata: Option<&Value>) -> Result<Vec<u8>, String> {
    log::info!("[文档生成] build_docx 输入 content: {}", content);
    let mut builder = rwml::DocBuilder::new();

    if let Some(meta) = metadata.filter(|m| m.is_object()) {
        if let Some(v) = str_field(meta, "title") { builder = builder.title(v); }
        if let Some(v) = str_field(meta, "subject") { builder = builder.subject(v); }
        if let Some(v) = str_field(meta, "creator") { builder = builder.creator(v); }
        if let Some(v) = str_field(meta, "description") { builder = builder.description(v); }
        if let Some(v) = str_field(meta, "keywords") { builder = builder.keywords(v); }
        if let Some(v) = str_field(meta, "category") { builder = builder.category(v); }
    }

    // 兜底页码页脚判定（footer 块常位于顶层，group/未知容器递归穿透）
    let has_footer = content["children"]
        .as_array()
        .map_or(false, |ch| has_footer_block(ch));

    if let Some(children) = content["children"].as_array() {
        if children.is_empty() {
            return Err("content.children 为空，没有可写入的内容".to_string());
        }
        // 预扫描标题：toc 块常排在最前、标题在后，先收集供静态目录使用
        let headings = collect_docx_headings(children);
        builder = apply_docx_blocks(builder, children, &headings)?;
    } else {
        // 兜底：title + text 的最简结构
        let title = str_field(content, "title");
        let text = content["text"].as_str().unwrap_or("").to_string();
        if title.is_none() && text.trim().is_empty() {
            return Err("content 缺少 children 数组（或 title/text 兜底字段）".to_string());
        }
        if let Some(t) = title {
            builder = builder.rich_paragraph(
                rwml::ParagraphBuilder::new().runs(vec![heading_run(t.as_str(), 1)]).heading_level(1).line_pct(DOC_PRESET_LINE_PCT).build(),
            );
        }
        for line in text.split('\n') {
            if !line.trim().is_empty() {
                builder = body_paragraph(builder, line.trim_end());
            }
        }
    }

    // 兜底页码页脚（方案A）：AI 未提供 footer 块时自动注入右对齐"第 X 页 共 Y 页"
    // （{PAGE}/{NUMPAGES} 域 + 小五宋体预设；注入发生在渲染期，不进 content 快照，
    //  多轮 read→create 每次都会自动补齐，AI 读回结构也不会出现页脚块）
    if !has_footer {
        let runs: Vec<rwml::Run> = split_running_placeholders("第 {PAGE} 页 共 {NUMPAGES} 页")
            .into_iter()
            .map(|(t, f)| running_run(&t, f))
            .collect();
        let pb = rwml::ParagraphBuilder::new()
            .runs(runs)
            .align(rwml::Align::Right);
        builder = builder.push_footer_block(rwml::Block::Paragraph(pb.build()));
    }

    let model = builder.build();
    let bytes =
        rwml::try_write_docx(&model).map_err(|e| format!("序列化 Word 文档失败: {}", e))?;
    if bytes.is_empty() {
        return Err("生成的 Word 文档为空".to_string());
    }
    Ok(bytes)
}

fn apply_docx_blocks(
    mut builder: rwml::DocBuilder,
    children: &[Value],
    headings: &[(u8, String)],
) -> Result<rwml::DocBuilder, String> {
    for child in children {
        // 容错1：纯字符串子块 → 视为段落（不再静默丢弃）
        if let Some(s) = child.as_str() {
            if !s.trim().is_empty() {
                builder = body_paragraph(builder, s.trim_end());
            }
            continue;
        }
        if !child.is_object() {
            log::warn!("[文档生成] 非对象子块跳过: {}", child);
            continue;
        }
        // 容错2：type 缺失/为空 → 按结构特征推断块类型（模型常漏写 type 字段）
        let raw_kind = child["type"].as_str().unwrap_or("").trim().to_lowercase();
        let kind = if raw_kind.is_empty() {
            infer_docx_block_kind(child).to_string()
        } else {
            raw_kind
        };
        match kind.as_str() {
            "heading" => {
                let level = child["level"].as_u64().unwrap_or(1).clamp(1, 6) as u8;
                let text = child["text"].as_str().unwrap_or("").trim().to_string();
                if text.is_empty() {
                    continue;
                }
                let align = parse_docx_align(child);
                let shading = docx_shading_color(child);
                // 标题预设：宋体 + 级别字号 + 1.5 倍行距（预设表唯一真相源）；heading_level 保留大纲/目录能力
                let hrun = heading_run(&text, level);
                let mut pb = rwml::ParagraphBuilder::new().runs(vec![hrun]).heading_level(level).line_pct(DOC_PRESET_LINE_PCT);
                if let Some(a) = align {
                    pb = pb.align(a);
                }
                if let Some((r, g, b)) = shading {
                    pb = pb.shading(rwml::Color::rgb(r, g, b));
                }
                builder = builder.rich_paragraph(pb.build());
            }
            "paragraph" | "p" => {
                let align = parse_docx_align(child);
                let shading = docx_shading_color(child);
                let styled = align.is_some() || shading.is_some();
                if let Some(runs) = child["runs"].as_array() {
                    let built: Vec<rwml::Run> = runs.iter().flat_map(build_runs).collect();
                    if !built.is_empty() {
                        let mut pb = rwml::ParagraphBuilder::new().runs(built).line_pct(DOC_PRESET_LINE_PCT);
                        if let Some(a) = align {
                            pb = pb.align(a);
                        }
                        if let Some((r, g, b)) = shading {
                            pb = pb.shading(rwml::Color::rgb(r, g, b));
                        }
                        builder = builder.rich_paragraph(pb.build());
                    }
                } else {
                    let text = child["text"].as_str().unwrap_or("");
                    for line in text.split('\n') {
                        if line.trim().is_empty() {
                            continue;
                        }
                        if styled {
                            let mut pb = rwml::ParagraphBuilder::new()
                                .runs(body_runs(line.trim_end(), Some(child)))
                                .line_pct(DOC_PRESET_LINE_PCT);
                            if let Some(a) = align {
                                pb = pb.align(a);
                            }
                            if let Some((r, g, b)) = shading {
                                pb = pb.shading(rwml::Color::rgb(r, g, b));
                            }
                            builder = builder.rich_paragraph(pb.build());
                        } else {
                            builder = body_paragraph_from(builder, line.trim_end(), Some(child));
                        }
                    }
                }
            }
            "list" | "bullet_list" | "numbered_list" => {
                let ordered = child["ordered"]
                    .as_bool()
                    .unwrap_or(kind == "numbered_list");
                let level = child["level"].as_u64().unwrap_or(0).min(8) as u8;
                let items: Vec<Value> = child["items"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                if items.is_empty() {
                    continue;
                }
                builder = render_list_paragraphs(builder, &items, ordered, level);
            }
            "table" => {
                let has_headers = child["headers"].is_array();
                // 收集原始值行（保留对象 cell 以读样式）
                let mut raw_rows: Vec<Vec<Value>> = Vec::new();
                if let Some(headers) = child["headers"].as_array() {
                    raw_rows.push(headers.iter().cloned().collect());
                }
                if let Some(data) = child["rows"].as_array() {
                    for r in data {
                        if let Some(cells) = r.as_array() {
                            raw_rows.push(cells.iter().cloned().collect());
                        }
                    }
                }
                if raw_rows.is_empty() {
                    continue;
                }
                // 所有表格统一走富路径：带默认样式兜底（细边框 + 表头浅灰底黑字加粗居中）
                builder = build_styled_docx_table(builder, child, has_headers, raw_rows)?;
            }
            "image" => {
                match decode_base64_image(child["base64"].as_str().unwrap_or("")) {
                    Ok(bytes) => {
                        let mime = sniff_image_mime(&bytes);
                        builder = builder.image(bytes, mime);
                    }
                    Err(e) => log::warn!("[文档生成] Word 图片解码失败，已跳过: {}", e),
                }
            }
            "hyperlink" => {
                let text = child["text"].as_str().unwrap_or("").trim();
                let url = child["url"].as_str().unwrap_or("").trim();
                if !text.is_empty() && !url.is_empty() {
                    builder = builder.hyperlink(text, url);
                }
            }
            "page_break" => builder = builder.page_break(),
            "section_break" => builder = builder.section_break(),
            "header" => {
                if let Some(v) = str_field(child, "text") {
                    builder = apply_running_text(builder, v, true);
                }
            }
            "footer" => {
                if let Some(v) = str_field(child, "text") {
                    builder = apply_running_text(builder, v, false);
                }
            }
            "page_numbers" => builder = builder.page_numbers(),
            "field" => {
                if let Some(ins) = str_field(child, "instruction") {
                    let res = child["result"].as_str().unwrap_or("");
                    builder = builder.field(ins, res);
                }
            }
            "chart" => match build_docx_chart(child) {
                Some(chart) => builder = builder.chart(chart),
                None => log::warn!("[文档生成] Word 图表数据不完整（需 series），已跳过"),
            },
            // 容器块：先输出自身文本（若有），再递归展开 children
            "group" => {
                if let Some(t) = str_field(child, "text") {
                    builder = body_paragraph(builder, &t);
                }
                if let Some(nested) = child["children"].as_array() {
                    builder = apply_docx_blocks(builder, nested, headings)?;
                }
            }
            // 目录：生成静态目录列表（打开 Word 立即可见；rwml 的 dirty TOC 域需 Word 更新域才渲染）
            "toc" => {
                if headings.is_empty() {
                    log::warn!("[文档生成] toc 块但未扫描到任何标题，保留原 TOC 域兜底");
                    let start = child["start"].as_u64().unwrap_or(1).clamp(1, 6) as u8;
                    let end = child["end"].as_u64().unwrap_or(3).clamp(1, 6) as u8;
                    builder = builder.toc_heading_range(start, end);
                } else {
                    for (level, text) in headings {
                        let indent = "  ".repeat((*level).saturating_sub(1) as usize);
                        builder = body_paragraph(builder, &format!("{}{}", indent, text));
                    }
                }
            }
            _ => {
                // 未知 type：有 children 则递归展开（内容不丢，模型常编造 section/article 等容器名）
                if let Some(nested) = child["children"].as_array() {
                    if let Some(t) = str_field(child, "text") {
                        builder = body_paragraph(builder, &t);
                    }
                    builder = apply_docx_blocks(builder, nested, headings)?;
                } else {
                    log::warn!("[文档生成] 未知 Word 块类型「{}」，已跳过: {}", kind, child);
                }
            }
        }
    }
    Ok(builder)
}

/// 空 type 块的结构特征推断（模型常漏写 type 字段）
fn infer_docx_block_kind(child: &Value) -> &'static str {
    if child["children"].is_array() {
        "group"
    } else if child["items"].is_array() {
        "list"
    } else if child["headers"].is_array() || child["rows"].is_array() {
        "table"
    } else if child["url"].is_string() && child["text"].is_string() {
        "hyperlink"
    } else if child["base64"].is_string() {
        "image"
    } else if child["level"].is_number() && child["text"].is_string() {
        "heading"
    } else if child["text"].is_string() || child["runs"].is_array() {
        "paragraph"
    } else if child["instruction"].is_string() {
        "field"
    } else if child["start"].is_number() || child["toc"].is_object() {
        "toc"
    } else {
        ""
    }
}

/// 预扫描收集标题 (level, text)：识别 type=heading、缺 type 的 level+text 推断块，
/// 并递归容器（group / 未知 type 带 children）。供 toc 块生成静态目录（目录常排在标题之前）
fn collect_docx_headings(children: &[Value]) -> Vec<(u8, String)> {
    fn walk(nodes: &[Value], out: &mut Vec<(u8, String)>) {
        for child in nodes {
            if !child.is_object() {
                continue;
            }
            let raw_kind = child["type"].as_str().unwrap_or("").trim().to_lowercase();
            let kind = if raw_kind.is_empty() {
                infer_docx_block_kind(child)
            } else {
                raw_kind.as_str()
            };
            match kind {
                "heading" => {
                    let level = child["level"].as_u64().unwrap_or(1).clamp(1, 6) as u8;
                    if let Some(t) = str_field(child, "text") {
                        out.push((level, md_strip_inline(&t))); // 目录条目去行内标记（标题渲染自带粗体）
                    }
                }
                "group" => {
                    // 容器自身若带标题特征也收集（children 优先于 level 的推断顺序会漏它）
                    if child["level"].is_number() {
                        if let Some(t) = str_field(child, "text") {
                            out.push((child["level"].as_u64().unwrap_or(1).clamp(1, 6) as u8, t));
                        }
                    }
                    if let Some(nested) = child["children"].as_array() {
                        walk(nested, out);
                    }
                }
                _ => {
                    // 未知 type 容器同样递归（section/article 等）
                    if let Some(nested) = child["children"].as_array() {
                        walk(nested, out);
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(children, &mut out);
    out
}

/// 显式 runs 数组的单条目 → run 序列：显式样式为基底，其 text 内行内 markdown 再细分
///（AI 直接透传 markdown 文本也能得到正确格式，无需手工拆 runs）
fn build_runs(r: &Value) -> Vec<rwml::Run> {
    let Some(text) = r["text"].as_str().filter(|t| !t.is_empty()) else { return vec![] };
    let size = text_type_size_half_pt(r, DOC_PRESET_BODY_HALF_PT);
    let color = str_field(r, "color").and_then(|s| parse_hex_rgb(&s));
    md_inline_runs(text)
        .into_iter()
        .map(|(t, st)| {
            let mut b = rwml::RunBuilder::new(t.as_str());
            if r["bold"].as_bool().unwrap_or(false) || st.bold { b = b.bold(); }
            if r["italic"].as_bool().unwrap_or(false) || st.italic { b = b.italic(); }
            if r["underline"].as_bool().unwrap_or(false) { b = b.underline(); }
            if r["small_caps"].as_bool().unwrap_or(false) { b = b.small_caps(); }
            // 字号字体由系统预设（text_type 选择类型：body/caption）——自由 font/size 字段已废除，传了忽略
            b = b.font(if st.code { "Consolas" } else { DOC_PRESET_FONT });
            b = b.size_half_pt(size);
            if let Some(hl) = str_field(r, "highlight") { b = b.highlight(hl); }
            if let Some(url) = str_field(r, "hyperlink") { b = b.hyperlink(url); }
            if let Some((cr, cg, cb)) = color {
                b = b.color(rwml::Color::rgb(cr, cg, cb));
            }
            b.build()
        })
        .collect()
}

/// 标题 run：宋体 + 级别字号（预设表唯一真相源；heading_level 保留大纲/目录能力）。
/// 标题自带粗体语义：行内标记只剥除防残留星号，不做分段
fn heading_run(text: &str, level: u8) -> rwml::Run {
    let idx = (level.clamp(1, 6) - 1) as usize;
    rwml::RunBuilder::new(md_strip_inline(text).as_str())
        .font(DOC_PRESET_FONT)
        .size_half_pt(DOC_PRESET_HEADING_HALF_PT[idx])
        .build()
}

/// Word 列表渲染：**字面编号**（每组每级独立计数，不跨组延续）+ 预设样式
/// （宋体小四 run + 1.5 倍行距 + 层级左缩进与悬挂）。rwml 合成 numbering 用固定 numId
/// 导致有序列表跨块编号延续、level 基准混乱——生成即终稿的场景字面前缀显示 100% 可控。
fn render_list_paragraphs(
    mut builder: rwml::DocBuilder,
    items: &[Value],
    ordered: bool,
    base_level: u8,
) -> rwml::DocBuilder {
    let mut flat: Vec<(u8, bool, String)> = Vec::new();
    collect_list_items(items, base_level, ordered, &mut flat);
    // AI 混用 0/1 基：以组内最小 level 作为顶层基准归一
    let min_level = flat.iter().map(|(l, _, _)| *l).min().unwrap_or(0);
    let mut counters: std::collections::BTreeMap<u8, u32> = Default::default();
    for (lvl, item_ordered, text) in flat {
        let lvl = lvl.saturating_sub(min_level);
        let bare = strip_list_prefix(&text);
        let prefix = if item_ordered {
            let n = counters.entry(lvl).or_insert(0);
            *n += 1;
            format!("{}. ", n)
        } else {
            match lvl % 3 { 0 => "• ", 1 => "◦ ", _ => "▪ " }.to_string()
        };
        let mut item_runs = vec![body_run(&prefix)];
        item_runs.extend(body_runs(&bare, None)); // 列表项文本同样支持行内加粗
        let pb = rwml::ParagraphBuilder::new()
            .runs(item_runs)
            .line_pct(DOC_PRESET_LINE_PCT)
            .indent_left_pt(21.0 * (lvl as f32 + 1.0))
            .hanging_pt(21.0);
        builder = builder.rich_paragraph(pb.build());
    }
    builder
}

/// 展平列表项：字符串 → 本级；对象 {text|label, ordered?, children|items?} → 子级自动 +1 层；
/// items 内嵌数组 → 子级列表 +1 层。子级默认无序（一级编号、二级圆点的行文惯例）。
fn collect_list_items(items: &[Value], level: u8, ordered: bool, out: &mut Vec<(u8, bool, String)>) {
    for it in items {
        match it {
            Value::String(s) => {
                let t = s.trim();
                if !t.is_empty() {
                    out.push((level, ordered, t.to_string()));
                }
            }
            Value::Object(o) => {
                // 注意：o 是 Map（缺失键索引会 panic），一律 get
                let text = o.get("text").and_then(|v| v.as_str())
                    .or_else(|| o.get("label").and_then(|v| v.as_str()))
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let item_ordered = o.get("ordered").and_then(|v| v.as_bool()).unwrap_or(ordered);
                if !text.is_empty() {
                    out.push((level, item_ordered, text));
                }
                let nested = o.get("children").and_then(|v| v.as_array())
                    .or_else(|| o.get("items").and_then(|v| v.as_array()));
                if let Some(kids) = nested {
                    let kid_ordered = o.get("children_ordered").and_then(|v| v.as_bool()).unwrap_or(false);
                    collect_list_items(kids, level.saturating_add(1), kid_ordered, out);
                }
            }
            Value::Array(nested) => {
                collect_list_items(nested, level.saturating_add(1), false, out);
            }
            _ => {}
        }
    }
}

/// 剥除 AI 常自带的序号/项目符号前缀（"1. "/"2、"/"3）"/"- "/"• "），返回纯文本
/// （系统统一重新编号，避免 "1. 1. 文本" 双重前缀）
fn strip_list_prefix(text: &str) -> String {
    if let Some(pos) = text.find(|c: char| !c.is_ascii_digit()) {
        if pos > 0 && pos <= 3 {
            let rest = &text[pos..];
            if let Some(r) = rest.strip_prefix(". ") {
                return r.trim_start().to_string();
            }
            if let Some(r) = rest.strip_prefix("、") {
                return r.trim_start().to_string();
            }
            if let Some(r) = rest.strip_prefix('.') {
                return r.trim_start().to_string();
            }
            if let Some(r) = rest.strip_prefix('）').or_else(|| rest.strip_prefix(')')) {
                return r.trim_start().to_string();
            }
        }
    }
    for b in ["- ", "• ", "· ", "* ", "◦ "] {
        if let Some(r) = text.strip_prefix(b) {
            return r.trim_start().to_string();
        }
    }
    text.to_string()
}

/// 表格单元格字号：text_type=caption→小五9pt，缺省小四12pt（字体恒宋体——预设唯一真相源）
fn table_run_size(v: &Value) -> u16 {
    text_type_size_half_pt(v, DOC_PRESET_TABLE_HALF_PT)
}

/// 解析段落对齐字段（align: center/right/justify/left → rwml::Align）
fn parse_docx_align(v: &Value) -> Option<rwml::Align> {
    str_field(v, "align").map(|a| match a.to_ascii_lowercase().as_str() {
        "center" | "centre" | "middle" => rwml::Align::Center,
        "right" => rwml::Align::Right,
        "justify" => rwml::Align::Justify,
        _ => rwml::Align::Left,
    })
}

/// 解析段落底纹字段（shading/bg → hex rgb）
fn docx_shading_color(v: &Value) -> Option<(u8, u8, u8)> {
    str_field(v, "shading")
        .or_else(|| str_field(v, "bg"))
        .and_then(|s| parse_hex_rgb(&s))
}

/// 构建带样式的 Word 表格：默认细边框 + 表头浅灰底黑字加粗居中（显式传值永远优先）
fn build_styled_docx_table(
    mut builder: rwml::DocBuilder,
    child: &Value,
    has_headers: bool,
    raw_rows: Vec<Vec<Value>>,
) -> Result<rwml::DocBuilder, String> {
    let mut tb = rwml::TableBuilder::new();

    // 表头行数：显式 header_rows 优先，否则有 headers 数组视为 1
    let header_rows = child["header_rows"]
        .as_u64()
        .map(|n| n as usize)
        .unwrap_or(if has_headers { 1 } else { 0 })
        .min(raw_rows.len());
    if header_rows > 0 {
        tb = tb.header_rows(header_rows);
    }

    // 边框：字符串指定线型；未传/true → 显式单线（Word 无法完全关闭边框，rwml 无 None 样式）
    match &child["border"] {
        Value::String(s) => {
            let st = match s.to_ascii_lowercase().as_str() {
                "double" => rwml::TableBorderStyle::Double,
                "dotted" => rwml::TableBorderStyle::Dotted,
                "dashed" => rwml::TableBorderStyle::Dashed,
                _ => rwml::TableBorderStyle::Single,
            };
            tb = tb.border_style(st);
        }
        _ => {
            tb = tb.border_style(rwml::TableBorderStyle::Single);
        }
    }
    if let Some((r, g, b)) = str_field(child, "border_color").and_then(|s| parse_hex_rgb(&s)) {
        tb = tb.border_color(rwml::Color::rgb(r, g, b));
    }

    // 列宽（占表格宽度的比例 0.0-1.0）
    if let Some(widths) = child["col_widths_pct"].as_array() {
        let w: Vec<f32> = widths
            .iter()
            .filter_map(|v| v.as_f64())
            .map(|v| v as f32)
            .collect();
        if !w.is_empty() {
            tb = tb.col_widths_pct(w);
        }
    }
    // 表格恒铺满页宽（fixed layout）：列宽由 col_widths_pct 比例或等分决定，
    // 行高由 Word 按单元格文本自动撑开（不写 trHeight）；AI 不再控制总宽
    tb = tb.width_pct(1.0).fixed_layout();

    // 表格对齐（table 节点上的 align 字段）
    if let Some(a) = parse_docx_align(child) {
        tb = tb.align(a);
    }

    // 行与单元格
    for (ri, row) in raw_rows.iter().enumerate() {
        let row_is_header = ri < header_rows;
        let cells: Vec<rwml::Cell> = row
            .iter()
            .map(|c| build_docx_cell(c, row_is_header))
            .collect();
        tb = tb.row(cells);
    }

    builder = builder.rich_table(tb.build());
    Ok(builder)
}

/// 表格单元格垂直对齐：默认 center（上下居中）；AI 可传 valign: top|center|bottom 覆盖。
/// 标量 cell（非对象）无该字段 → 恒居中。
fn parse_cell_valign(v: &Value) -> rwml::VCell {
    match v["valign"].as_str().unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "top" => rwml::VCell::Top,
        "bottom" => rwml::VCell::Bottom,
        _ => rwml::VCell::Center,
    }
}

/// 构建 Word 表格单元格：标量 → 纯文本；对象 → text + 样式字段
/// 表头行默认样式：浅灰底(D9D9D9) + 加粗 + 段落居中（黑字为 Word 默认，无需显式）；
/// 对象 cell 显式传值永远优先。
fn build_docx_cell(v: &Value, is_header: bool) -> rwml::Cell {
    // Word 表头默认底纹：浅灰（与 Excel 的蓝白方案区分，更偏公文风格）
    const DEFAULT_HEADER_BG: (u8, u8, u8) = (0xD9, 0xD9, 0xD9);

    let is_obj = v.is_object();
    let text = if is_obj {
        child_text_or_value(v)
    } else {
        value_to_plain_string(v)
    };
    let cell_header = if is_obj {
        v["is_header"].as_bool().unwrap_or(is_header)
    } else {
        is_header
    };

    // 底纹：显式 bg/shading 优先；表头行未传 → 浅灰
    let shading = if is_obj {
        str_field(v, "bg")
            .or_else(|| str_field(v, "shading"))
            .and_then(|s| parse_hex_rgb(&s))
    } else {
        None
    };
    let shading = shading
        .map(|(r, g, b)| rwml::Color::rgb(r, g, b))
        .or_else(|| {
            if cell_header {
                let (r, g, b) = DEFAULT_HEADER_BG;
                Some(rwml::Color::rgb(r, g, b))
            } else {
                None
            }
        });

    // 加粗：显式 bold 优先；表头行默认加粗
    let bold = if is_obj {
        v["bold"].as_bool().unwrap_or(cell_header)
    } else {
        cell_header
    };

    // 字色：仅显式传 color 时设置（表头黑字即 Word 默认，不显式设）
    let color = if is_obj {
        str_field(v, "color").and_then(|s| parse_hex_rgb(&s))
    } else {
        None
    };

    // 段落对齐：显式 align 优先；表头行默认居中
    let align = if is_obj { parse_docx_align(v) } else { None };
    let align = align.or_else(|| if cell_header { Some(rwml::Align::Center) } else { None });

    // 段落内容：空文本 cell 仍可由 shading 承载底纹（cell 级属性不依赖 blocks）；
    // 显式 bold/color 作全段基底，text 内行内 **粗**/*斜* 叠加为部分加粗
    let blocks = if text.is_empty() {
        vec![]
    } else {
        let runs: Vec<rwml::Run> = md_inline_runs(&text)
            .into_iter()
            .map(|(t, st)| {
                let mut rb = rwml::RunBuilder::new(t.as_str())
                    .font(DOC_PRESET_FONT)
                    .size_half_pt(table_run_size(v));
                if bold || st.bold {
                    rb = rb.bold();
                }
                if st.italic {
                    rb = rb.italic();
                }
                if let Some((r, g, b)) = color {
                    rb = rb.color(rwml::Color::rgb(r, g, b));
                }
                rb.build()
            })
            .collect();
        let mut pb = rwml::ParagraphBuilder::new().runs(runs);
        if let Some(a) = align {
            pb = pb.align(a);
        }
        vec![rwml::Block::Paragraph(pb.build())]
    };

    rwml::Cell {
        blocks,
        is_header: cell_header,
        shading,
        // 默认上下居中（方案B）：表头与数据格统一；valign 显式值覆盖
        valign: parse_cell_valign(v),
        ..Default::default()
    }
}

fn build_docx_chart(c: &Value) -> Option<rwml::Chart> {
    let series_arr = c["series"].as_array().filter(|a| !a.is_empty())?;
    let series: Vec<rwml::ChartSeries> = series_arr
        .iter()
        .map(|s| rwml::ChartSeries {
            name: s["name"].as_str().unwrap_or("系列").to_string(),
            values: s["values"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
                .unwrap_or_default(),
            bubble_sizes: s["bubble_sizes"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
                .unwrap_or_default(),
        })
        .collect();
    Some(rwml::Chart {
        kind: parse_docx_chart_kind(
            child_str(c, "kind").or_else(|| child_str(c, "chart_type")).as_deref().unwrap_or("column"),
        ),
        title: str_field(c, "title"),
        categories: c["categories"]
            .as_array()
            .map(|a| a.iter().map(value_to_plain_string).collect())
            .unwrap_or_default(),
        series,
        width_px: c["width_px"].as_u64().map(|v| v as u32),
        height_px: c["height_px"].as_u64().map(|v| v as u32),
        alt: str_field(c, "alt"),
        ..Default::default()
    })
}

fn parse_docx_chart_kind(s: &str) -> rwml::ChartKind {
    use rwml::ChartKind as K;
    let normalized = s.to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "bar" => K::Bar,
        "stacked_bar" => K::StackedBar,
        "line" => K::Line,
        "smooth_line" => K::SmoothLine,
        "stacked_line" => K::StackedLine,
        "area" => K::Area,
        "stacked_area" => K::StackedArea,
        "pie" => K::Pie,
        "exploded_pie" => K::ExplodedPie,
        "doughnut" => K::Doughnut,
        "radar" => K::Radar,
        "filled_radar" => K::FilledRadar,
        "scatter" => K::Scatter,
        "stacked_column" => K::StackedColumn,
        "bar_3d" | "bar3d" => K::Bar3D,
        "column_3d" | "column3d" => K::Column3D,
        _ => K::Column,
    }
}

// ═══════════════════════════════════════════════════════════════
// Excel (.xlsx) 构建 — rust_xlsxwriter
// ═══════════════════════════════════════════════════════════════

fn build_xlsx(content: &Value) -> Result<Vec<u8>, String> {
    use rust_xlsxwriter::Workbook;

    log::info!("[文档生成] build_xlsx 输入 content: {}", content);

    let mut workbook = Workbook::new();

    // sheets 结构；缺省时用单层 children 兜底为 Sheet1
    let fallback_sheet;
    let sheets: &[Value] = match content["sheets"].as_array().filter(|a| !a.is_empty()) {
        Some(arr) => {
            log::info!("[文档生成] 解析到 {} 个 sheets", arr.len());
            arr
        }
        None => {
            let children = if content["children"].is_array() {
                content["children"].clone()
            } else {
                json!([])
            };
            log::info!("[文档生成] 无 sheets，使用 fallback children: {}", children);
            fallback_sheet = json!({"name": "Sheet1", "children": children});
            std::slice::from_ref(&fallback_sheet)
        }
    };

    let mut used_names: Vec<String> = Vec::new();
    for (si, sheet) in sheets.iter().enumerate() {
        let name = unique_sheet_name(sheet["name"].as_str().unwrap_or(""), si, &mut used_names);
        let ws = workbook
            .add_worksheet()
            .set_name(&name)
            .map_err(|e| format!("设置工作表名「{}」失败: {}", name, e))?;
        // 容错：children 之外还接受 rows / data（数组的数组），归一化为 row 子块
        let normalized: Vec<Value>;
        let children: &[Value] = if let Some(arr) = sheet["children"].as_array() {
            arr
        } else if let Some(rows) = sheet["rows"].as_array().or_else(|| sheet["data"].as_array()) {
            normalized = rows
                .iter()
                .map(|r| {
                    if r.is_array() {
                        json!({"type": "row", "cells": r})
                    } else {
                        r.clone()
                    }
                })
                .collect();
            &normalized
        } else {
            &[]
        };
        log::info!("[文档生成] sheet[{}]「{}」有 {} 个子块", si, name, children.len());
        if children.is_empty() && sheets.len() == 1 {
            return Err("content 缺少可写入的单元格内容".to_string());
        }
        apply_xlsx_children(ws, children)?;
    }

    // save 到临时文件后读回字节（save 权威 API 接受路径）
    let tmp = std::env::temp_dir().join(format!("arkmind_gen_{}.xlsx", uuid::Uuid::new_v4()));
    let tmp_str = tmp.to_string_lossy().to_string();
    workbook
        .save(tmp_str.as_str())
        .map_err(|e| format!("保存 Excel 失败: {}", e))?;
    let bytes = std::fs::read(&tmp).map_err(|e| format!("读取生成文件失败: {}", e));
    let _ = std::fs::remove_file(&tmp);
    bytes
}

fn apply_xlsx_children(
    ws: &mut rust_xlsxwriter::Worksheet,
    children: &[Value],
) -> Result<(), String> {
    use rust_xlsxwriter::{Chart, ChartType, Image};

    let mut row_cursor: u32 = 0;

    for child in children {
        // 容错1：子块本身就是数组 → 视为一行 cells（模型常直接给 [["a","b"],[1,2]]）
        if let Some(cells) = child.as_array() {
            write_xlsx_row(ws, cells, false, &mut row_cursor)?;
            continue;
        }
        if !child.is_object() {
            log::warn!("[文档生成] 非对象子块跳过: {:?}", child);
            continue;
        }
        // 容错2：type 缺失/为空但带 cells → 视为 row（模型常漏写 type 字段）
        let raw_kind = child["type"].as_str().unwrap_or("").trim().to_lowercase();
        let kind = if raw_kind.is_empty() && child["cells"].is_array() {
            "row".to_string()
        } else {
            raw_kind
        };
        log::info!("[文档生成] 处理子块 type=\"{}\"", kind);
        match kind.as_str() {
            "row" => {
                let Some(cells) = child["cells"].as_array() else {
                    log::warn!("[文档生成] row 子块缺少 cells 数组: {}", child);
                    continue;
                };
                let is_header = child["header"].as_bool().unwrap_or(false);
                write_xlsx_row(ws, cells, is_header, &mut row_cursor)?;
            }
            "column_width" | "column_widths" => {
                if let Some(widths) = child["widths"].as_array() {
                    for (i, w) in widths.iter().enumerate() {
                        let width = w.as_f64().unwrap_or(10.0);
                        ws.set_column_width(i as u16, width)
                            .map_err(|e| format!("设置列宽失败: {}", e))?;
                    }
                }
            }
            "row_height" => {
                // 行高由 Excel 按内容自适应——固定行高会截断/挤压文本，AI 设置已弃用
                log::warn!("[文档生成] xlsx row_height 已弃用（行高自适应文本），已忽略: {:?}", child);
            }
            "image" => match decode_base64_image(child["base64"].as_str().unwrap_or("")) {
                Ok(bytes) => match Image::new_from_buffer(&bytes) {
                    Ok(img) => {
                        let mut img = img;
                        if let Some(w) = child["width_px"].as_u64() {
                            img = img.set_width(w as u32);
                        }
                        if let Some(h) = child["height_px"].as_u64() {
                            img = img.set_height(h as u32);
                        }
                        if let Some(alt) = str_field(child, "alt") {
                            img = img.set_alt_text(alt);
                        }
                        let row = child["row"].as_u64().unwrap_or(row_cursor as u64) as u32;
                        let col = child["col"].as_u64().unwrap_or(0) as u16;
                        if let Err(e) = ws.insert_image(row, col, &img) {
                            log::warn!("[文档生成] Excel 插入图片失败，已跳过: {}", e);
                        }
                    }
                    Err(e) => log::warn!("[文档生成] Excel 图片解析失败，已跳过: {}", e),
                },
                Err(e) => log::warn!("[文档生成] 图片 base64 解码失败，已跳过: {}", e),
            },
            "chart" => {
                let ct = child_str(child, "chart_type")
                    .or_else(|| child_str(child, "kind"))
                    .unwrap_or_else(|| "column".to_string());
                let chart_type = match ct.to_ascii_lowercase().replace('-', "_").as_str() {
                    "bar" => ChartType::Bar,
                    "stacked_bar" | "bar_stacked" => ChartType::BarStacked,
                    "stacked_column" | "column_stacked" => ChartType::ColumnStacked,
                    "line" => ChartType::Line,
                    "pie" => ChartType::Pie,
                    "area" => ChartType::Area,
                    "doughnut" => ChartType::Doughnut,
                    "radar" => ChartType::Radar,
                    "scatter" => ChartType::Scatter,
                    _ => ChartType::Column,
                };
                let mut chart = Chart::new(chart_type);
                if let Some(t) = str_field(child, "title") {
                    chart.title().set_name(t.as_str());
                }
                if let Some(series) = child["series"].as_array() {
                    for s in series {
                        let se = chart.add_series();
                        if let Some(n) = s["name"].as_str() {
                            se.set_name(n);
                        }
                        if let Some(c) = s["categories"].as_str() {
                            se.set_categories(c);
                        }
                        if let Some(v) = s["values"].as_str() {
                            se.set_values(v);
                        }
                    }
                }
                let row = child["row"].as_u64().unwrap_or(row_cursor as u64) as u32;
                let col = child["col"].as_u64().unwrap_or(0) as u16;
                if let Err(e) = ws.insert_chart(row, col, &chart) {
                    log::warn!("[文档生成] Excel 插入图表失败，已跳过: {}", e);
                }
            }
            "merge_range" | "merge_cells" => {
                if let Some(range) = str_field(child, "range") {
                    let text = child["text"].as_str().unwrap_or("");
                    match parse_a1_range(&range) {
                        Some((r1, c1, r2, c2)) => {
                            let merge_fmt = rust_xlsxwriter::Format::new()
                                .set_font_name(XLSX_PRESET_FONT)
                                .set_font_size(XLSX_PRESET_SIZE_PT);
                            if let Err(e) = ws.merge_range(r1, c1, r2, c2, text, &merge_fmt) {
                                log::warn!("[文档生成] Excel 合并单元格失败（{}），已跳过: {}", range, e);
                            }
                        }
                        None => log::warn!("[文档生成] Excel 合并区域格式无效（{}），已跳过", range),
                    }
                }
            }
            _ => log::warn!("[文档生成] 未知 Excel 块类型「{}」，已跳过: {}", kind, child),
        }
    }
    Ok(())
}

/// 写入一行单元格并推进 row_cursor（供 row 分支与数组容错共用）
/// 每格解析样式字段构建 Format；公式带样式写入；数字文本自动数值化
fn write_xlsx_row(
    ws: &mut rust_xlsxwriter::Worksheet,
    cells: &[Value],
    is_header: bool,
    row_cursor: &mut u32,
) -> Result<(), String> {
    use rust_xlsxwriter::XlsxError;

    let row = *row_cursor;
    log::info!("[文档生成] 写入 row={} cells={} header={}", row, cells.len(), is_header);
    for (ci, cell) in cells.iter().enumerate() {
        if cell.is_null() {
            continue;
        }
        let col = ci as u16;
        // 归一预设字体（宋体11pt）：build_cell_format 内已对有格式单元格补齐；
        // "零格式"单元格（如显式 border:false）在调用侧补齐——所有 xlsx 文字统一宋体11
        let fmt = Some(build_cell_format(cell, is_header).unwrap_or_else(|| {
            rust_xlsxwriter::Format::new()
                .set_font_name(XLSX_PRESET_FONT)
                .set_font_size(XLSX_PRESET_SIZE_PT)
        }));

        // 公式：带样式写入
        if let Some(formula) = cell.get("formula").and_then(|f| f.as_str()) {
            let res: Result<(), XlsxError> = match &fmt {
                Some(f) => ws.write_formula_with_format(row, col, formula, f).map(|_| ()),
                None => ws.write_formula(row, col, formula).map(|_| ()),
            };
            res.map_err(|e| xlsx_cell_err(row, ci, &e))?;
            continue;
        }

        let content = extract_cell_content(cell);
        let res: Result<(), XlsxError> = match content {
            CellContent::Empty => match &fmt {
                // 空内容但有样式（如合计行的空格仍要边框/背景）→ 写空白格
                Some(f) => ws.write_blank(row, col, f).map(|_| ()),
                None => Ok(()),
            },
            CellContent::Bool(b) => match &fmt {
                Some(f) => ws.write_with_format(row, col, b, f).map(|_| ()),
                None => ws.write(row, col, b).map(|_| ()),
            },
            CellContent::Int(i) => match &fmt {
                Some(f) => ws.write_with_format(row, col, i, f).map(|_| ()),
                None => ws.write(row, col, i).map(|_| ()),
            },
            CellContent::Num(n) => match &fmt {
                Some(f) => ws.write_with_format(row, col, n, f).map(|_| ()),
                None => ws.write(row, col, n).map(|_| ()),
            },
            CellContent::Text(s) => {
                // Excel 单元格单样式：整段 **包裹** → 转整格加粗；其余行内标记仅剥除（不改字）
                let (s, wrap_bold) = if md_fully_bold(&s) {
                    (md_strip_inline(&s), true)
                } else if s.contains("**") {
                    (md_strip_inline(&s), false)
                } else {
                    (s, false)
                };
                let fmt = if wrap_bold { fmt.map(|f| f.set_bold()) } else { fmt };
                if s.is_empty() {
                    match &fmt {
                        Some(f) => ws.write_blank(row, col, f).map(|_| ()),
                        None => Ok(()),
                    }
                } else {
                    match &fmt {
                        Some(f) => ws.write_with_format(row, col, s.as_str(), f).map(|_| ()),
                        None => ws.write(row, col, s.as_str()).map(|_| ()),
                    }
                }
            }
        };
        res.map_err(|e| xlsx_cell_err(row, ci, &e))?;
    }
    *row_cursor += 1;
    Ok(())
}

/// 单元格内容分类（数值化后的写入意图）
enum CellContent {
    Empty,
    Bool(bool),
    Int(i64),
    Num(f64),
    Text(String),
}

/// 从单元格 JSON 提取内容；字符串按需数值化（num_format="@" 强制文本）
fn extract_cell_content(cell: &Value) -> CellContent {
    let force_text = cell["num_format"].as_str().map_or(false, |s| s.trim() == "@");
    match cell {
        Value::Bool(b) => CellContent::Bool(*b),
        Value::Number(n) => n
            .as_i64()
            .map(CellContent::Int)
            .or_else(|| n.as_f64().map(CellContent::Num))
            .unwrap_or_else(|| CellContent::Text(n.to_string())),
        Value::String(s) => text_to_content(s, force_text),
        Value::Object(_) => {
            if let Some(v) = cell.get("value").filter(|v| !v.is_null()) {
                return match v {
                    Value::Bool(b) => CellContent::Bool(*b),
                    Value::Number(n) => n
                        .as_i64()
                        .map(CellContent::Int)
                        .or_else(|| n.as_f64().map(CellContent::Num))
                        .unwrap_or_else(|| CellContent::Text(n.to_string())),
                    Value::String(s) => text_to_content(s, force_text),
                    other => CellContent::Text(value_to_plain_string(other)),
                };
            }
            let t = cell["text"].as_str().unwrap_or("");
            text_to_content(t, force_text)
        }
        Value::Array(_) => CellContent::Text(value_to_plain_string(cell)),
        Value::Null => CellContent::Empty,
    }
}

fn text_to_content(s: &str, force_text: bool) -> CellContent {
    if s.is_empty() {
        return CellContent::Empty;
    }
    if !force_text {
        if let Some(n) = parse_numeric_text(s) {
            if n.fract() == 0.0 && n >= i64::MIN as f64 && n <= i64::MAX as f64 {
                return CellContent::Int(n as i64);
            }
            return CellContent::Num(n);
        }
    }
    CellContent::Text(s.to_string())
}

/// 字符串数值化：可解析为 f64 且无前导零（"0.x" 除外）→ 数值
/// "10"→10、"12.5"→12.5、"007"→保持文本、"007.5"→保持文本
fn parse_numeric_text(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let unsigned = t.strip_prefix('-').unwrap_or(t);
    if unsigned.len() > 1 && unsigned.starts_with('0') && !unsigned.as_bytes()[1].eq(&b'.') {
        return None;
    }
    t.parse::<f64>().ok()
}

/// 从单元格 JSON 构建样式 Format；恒返回 Some（预设字体/字号/垂直居中无条件生效，
/// 仅显式 valign: top|bottom 可覆盖垂直居中）
/// 支持：bold/italic/border/border_color/bg/color/align/valign/wrap/num_format/font/size/underline/strike
fn build_cell_format(cell: &Value, is_header: bool) -> Option<rust_xlsxwriter::Format> {
    use rust_xlsxwriter::{Format, FormatAlign, FormatBorder, FormatUnderline};

    // 默认常量：Office 经典表头配色（AI 未传样式时的兜底，显式传值永远优先）
    const DEFAULT_HEADER_BG: &str = "4472C4";
    const DEFAULT_HEADER_COLOR: &str = "FFFFFF";

    let mut fmt = Format::new();

    // bold：显式字段优先，header 行默认加粗
    if cell["bold"].as_bool().unwrap_or(is_header) {
        fmt = fmt.set_bold();
    }
    if cell["italic"].as_bool().unwrap_or(false) {
        fmt = fmt.set_italic();
    }

    // border：显式 false 尊重（无边框）；显式 true/字符串用其值；未传 → 默认全表细边框
    match cell.get("border") {
        Some(Value::Bool(false)) => { /* 尊重显式关闭 */ }
        Some(Value::Bool(true)) => {
            fmt = fmt.set_border(FormatBorder::Thin);
        }
        Some(Value::String(s)) => {
            let b = match s.to_ascii_lowercase().as_str() {
                "medium" => FormatBorder::Medium,
                "thick" => FormatBorder::Thick,
                "double" => FormatBorder::Double,
                "dotted" => FormatBorder::Dotted,
                "dashed" => FormatBorder::Dashed,
                _ => FormatBorder::Thin,
            };
            fmt = fmt.set_border(b);
        }
        // 未传（含标量 cell）→ 默认细线边框，保证表格网格完整
        _ => {
            fmt = fmt.set_border(FormatBorder::Thin);
        }
    }
    if let Some(c) = cell["border_color"].as_str().and_then(norm_hex) {
        fmt = fmt.set_border_color(c.as_str());
    }

    // 背景色：显式优先；header 行未传 → 默认 Office 蓝
    let bg_explicit = cell["bg"]
        .as_str()
        .or_else(|| cell["background"].as_str());
    if let Some(raw) = bg_explicit {
        if let Some(c) = norm_hex(raw) {
            fmt = fmt.set_background_color(c.as_str());
        }
    } else if is_header {
        fmt = fmt.set_background_color(DEFAULT_HEADER_BG);
    }

    // 字体颜色：显式优先；header 行未传 → 默认白字（配表头底色）
    if let Some(c) = cell["color"].as_str().and_then(norm_hex) {
        fmt = fmt.set_font_color(c.as_str());
    } else if is_header {
        fmt = fmt.set_font_color(DEFAULT_HEADER_COLOR);
    }

    // 水平对齐：显式优先；header 行未传 → 居中
    if let Some(a) = cell["align"].as_str() {
        let fa = match a.to_ascii_lowercase().as_str() {
            "center" | "centre" | "middle" => Some(FormatAlign::Center),
            "left" => Some(FormatAlign::Left),
            "right" => Some(FormatAlign::Right),
            _ => None,
        };
        if let Some(fa) = fa {
            fmt = fmt.set_align(fa);
        }
    } else if is_header {
        fmt = fmt.set_align(FormatAlign::Center);
    }
    // 垂直对齐统一在尾部预设处理（默认垂直居中，显式 top/bottom 覆盖）

    // 自动换行：显式 wrap 或文本含换行符
    if cell["wrap"].as_bool().unwrap_or(false) {
        fmt = fmt.set_text_wrap();
    } else if cell["text"].as_str().map_or(false, |t| t.contains('\n')) {
        fmt = fmt.set_text_wrap();
    }

    // 数字格式
    if let Some(nf) = str_field(cell, "num_format") {
        fmt = fmt.set_num_format(nf);
    }
    // 字号字体：系统预设唯一真相源（text_type=caption 小五9pt，否则 11pt；自由 font/size 已废除）
    // 注：set_align 的水平/垂直是独立槽位（rust_xlsxwriter 按值分派），与表头水平 center 共存
    fmt = fmt.set_font_name(XLSX_PRESET_FONT);
    let sz = if cell["text_type"].as_str() == Some("caption") {
        XLSX_CAPTION_SIZE_PT
    } else {
        XLSX_PRESET_SIZE_PT
    };
    fmt = fmt.set_font_size(sz);
    // 垂直对齐：默认居中（更易读），显式 valign: top|bottom 覆盖
    match cell["valign"].as_str().unwrap_or("").to_ascii_lowercase().as_str() {
        "top" => { fmt = fmt.set_align(FormatAlign::Top); }
        "bottom" => { fmt = fmt.set_align(FormatAlign::Bottom); }
        _ => { fmt = fmt.set_align(FormatAlign::VerticalCenter); }
    }
    if cell["underline"].as_bool().unwrap_or(false) {
        fmt = fmt.set_underline(FormatUnderline::Single);
    }
    if cell["strike"].as_bool().unwrap_or(false)
        || cell["strikethrough"].as_bool().unwrap_or(false)
    {
        fmt = fmt.set_font_strikethrough();
    }

    Some(fmt)
}

/// 归一化 hex 颜色：去 # 前缀，校验 6 位十六进制
fn norm_hex(s: &str) -> Option<String> {
    let t = s.trim().trim_start_matches('#');
    if t.len() == 6 && t.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(t.to_string())
    } else {
        None
    }
}

/// hex 颜色 → (r, g, b)
fn parse_hex_rgb(s: &str) -> Option<(u8, u8, u8)> {
    let t = norm_hex(s)?;
    let r = u8::from_str_radix(&t[0..2], 16).ok()?;
    let g = u8::from_str_radix(&t[2..4], 16).ok()?;
    let b = u8::from_str_radix(&t[4..6], 16).ok()?;
    Some((r, g, b))
}

fn xlsx_cell_err(row: u32, col: usize, e: &rust_xlsxwriter::XlsxError) -> String {
    format!("写入单元格失败(R{}C{}): {}", row + 1, col + 1, e)
}

fn unique_sheet_name(raw: &str, idx: usize, used: &mut Vec<String>) -> String {
    const ILLEGAL: [char; 7] = ['[', ']', ':', '*', '?', '/', '\\'];
    let cleaned: String = raw
        .trim()
        .chars()
        .map(|c| if ILLEGAL.contains(&c) || (c as u32) < 0x20 { '_' } else { c })
        .collect();
    let base = if cleaned.trim().is_empty() {
        format!("Sheet{}", idx + 1)
    } else {
        cleaned.trim().to_string()
    };
    let base: String = base.chars().take(31).collect();
    let mut name = base.clone();
    let mut n = 2;
    while used.contains(&name) {
        let suffix = format!("_{}", n);
        let mut trimmed: String = base.chars().take(31usize.saturating_sub(suffix.chars().count())).collect();
        trimmed.push_str(&suffix);
        name = trimmed;
        n += 1;
    }
    used.push(name.clone());
    name
}

// ═══════════════════════════════════════════════════════════════
// 通用工具
// ═══════════════════════════════════════════════════════════════

fn str_field(v: &Value, key: &str) -> Option<String> {
    v[key]
        .as_str()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

fn child_str(v: &Value, key: &str) -> Option<String> {
    v[key].as_str().map(String::from)
}

/// JSON 单元格 → 纯文本（数字/布尔原样，对象取 text/formula，数组逗号拼接）
fn value_to_plain_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        Value::Object(_) => child_text_or_value(v),
        Value::Array(a) => a.iter().map(value_to_plain_string).collect::<Vec<_>>().join(", "),
    }
}

fn child_text_or_value(v: &Value) -> String {
    if let Some(t) = v.get("text").and_then(|t| t.as_str()) {
        return t.to_string();
    }
    if !v.get("value").map_or(true, |val| val.is_null()) {
        let s = value_to_plain_string(&v["value"]);
        if !s.is_empty() {
            return s;
        }
    }
    if let Some(f) = v.get("formula").and_then(|f| f.as_str()) {
        return f.to_string();
    }
    String::new()
}

/// 解析 A1 记法区域（如 "A1:B2" 或 "A1"）为 0 基 (row1, col1, row2, col2)
fn parse_a1_range(range: &str) -> Option<(u32, u16, u32, u16)> {
    let (start, end) = match range.split_once(':') {
        Some((s, e)) => (s.trim(), e.trim()),
        None => (range.trim(), range.trim()),
    };
    let (c1, r1) = parse_a1_cell(start)?;
    let (c2, r2) = parse_a1_cell(end)?;
    Some((r1, c1, r2, c2))
}

/// 解析单个 A1 单元格（如 "AB12"）为 0 基 (col, row)
fn parse_a1_cell(cell: &str) -> Option<(u16, u32)> {
    let mut col: u32 = 0;
    let mut row_str = String::new();
    let mut seen_digit = false;
    for ch in cell.chars() {
        if ch.is_ascii_digit() {
            seen_digit = true;
            row_str.push(ch);
        } else if !seen_digit && ch.is_ascii_alphabetic() {
            col = col * 26 + (ch.to_ascii_uppercase() as u32 - 'A' as u32 + 1);
        } else {
            return None;
        }
    }
    if col == 0 || col > 16384 || row_str.is_empty() {
        return None;
    }
    let row: u32 = row_str.parse().ok()?;
    if row == 0 || row > 1_048_576 {
        return None;
    }
    Some((col as u16 - 1, row - 1))
}

/// 解码 base64 图片，兼容 data URI 前缀
fn decode_base64_image(raw: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    let s = raw.trim();
    if s.is_empty() {
        return Err("图片 base64 为空".to_string());
    }
    let payload = if s.starts_with("data:") {
        s.split(',').nth(1).ok_or_else(|| "data URI 格式无效".to_string())?
    } else {
        s
    };
    base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|e| format!("base64 解码失败: {}", e))
}

/// 魔数嗅探图片 MIME（内容权威，优先于 AI 声明的 mime）
fn sniff_image_mime(b: &[u8]) -> &'static str {
    if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if b.starts_with(b"GIF8") {
        "image/gif"
    } else if b.starts_with(b"BM") {
        "image/bmp"
    } else if b.len() > 12 && b.starts_with(b"RIFF") && &b[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "image/png"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_xlsx_basic() {
        let content = json!({
            "sheets": [{
                "name": "测试表",
                "children": [
                    {"type": "row", "header": true, "cells": ["名称", "数量", "单价"]},
                    {"type": "row", "cells": ["产品A", 10, 99.5]},
                    {"type": "row", "cells": ["产品B", 20, 199]},
                    {"type": "row", "cells": ["合计", "", {"formula": "=SUM(C2:C3)"}]}
                ]
            }]
        });
        let result = build_xlsx(&content);
        assert!(result.is_ok(), "build_xlsx failed: {:?}", result.err());
        let bytes = result.unwrap();
        assert!(!bytes.is_empty(), "xlsx bytes should not be empty");
        // XLSX 文件头是 PK (ZIP)
        assert_eq!(&bytes[..2], b"PK", "not a valid zip/xlsx file");
        println!("xlsx size: {} bytes", bytes.len());
    }

    #[test]
    fn merge_docx_children_appends_and_validates() {
        let old = json!({"title": "方案", "children": [{"type": "heading", "text": "一、背景"}]});
        // 对象 {children:[...]} 追加
        let merged = merge_docx_children(&old, &json!({"children": [{"type": "heading", "text": "二、需求"}]})).unwrap();
        assert_eq!(merged["title"], "方案");
        assert_eq!(merged["children"].as_array().unwrap().len(), 2);
        // 裸数组追加同样可行
        let m2 = merge_docx_children(&old, &json!([{"type": "paragraph", "text": "p"}])).unwrap();
        assert_eq!(m2["children"].as_array().unwrap().len(), 2);
        // 空 children / 结构非法 → 报错不产出
        assert!(merge_docx_children(&old, &json!({"children": []})).is_err());
        assert!(merge_docx_children(&old, &json!({"foo": 1})).is_err());
        // 旧文档无 children 时以追加块为全新 children
        let m3 = merge_docx_children(&json!({"title": "t"}), &json!({"children": [{"type": "paragraph", "text": "x"}]})).unwrap();
        assert_eq!(m3["children"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_build_xlsx_fallback_children() {
        let content = json!({
            "children": [
                {"type": "row", "cells": ["A", "B"]},
                {"type": "row", "cells": [1, 2]}
            ]
        });
        let result = build_xlsx(&content);
        assert!(result.is_ok(), "build_xlsx failed: {:?}", result.err());
        assert!(!result.unwrap().is_empty());
    }

    /// 容错：子块缺少 type 字段但带 cells（模型常见漏写）
    #[test]
    fn test_build_xlsx_missing_type_fallback() {
        let content = json!({
            "sheets": [{
                "name": "表1",
                "children": [
                    {"header": true, "cells": ["名称", "数量"]},
                    {"cells": ["产品A", 10]},
                    {"cells": ["合计", {"formula": "=SUM(B2:B2)"}]}
                ]
            }]
        });
        let result = build_xlsx(&content);
        assert!(result.is_ok(), "build_xlsx failed: {:?}", result.err());
        assert_eq!(&result.unwrap()[..2], b"PK");
    }

    /// 容错：children 元素直接是数组（行即数组）
    #[test]
    fn test_build_xlsx_array_rows_fallback() {
        let content = json!({
            "sheets": [{
                "name": "表1",
                "children": [["名称", "数量"], ["产品A", 10]]
            }]
        });
        let result = build_xlsx(&content);
        assert!(result.is_ok(), "build_xlsx failed: {:?}", result.err());
        assert_eq!(&result.unwrap()[..2], b"PK");
    }

    /// 容错：sheet 用 rows 字段而非 children，行是数组的数组
    #[test]
    fn test_build_xlsx_sheet_rows_field() {
        let content = json!({
            "sheets": [{
                "name": "表1",
                "rows": [["名称", "数量"], ["产品A", 10], ["产品B", 20]]
            }]
        });
        let result = build_xlsx(&content);
        assert!(result.is_ok(), "build_xlsx failed: {:?}", result.err());
        assert_eq!(&result.unwrap()[..2], b"PK");
    }

    #[test]
    fn test_build_docx_basic() {
        let content = json!({
            "children": [
                {"type": "heading", "level": 1, "text": "测试标题"},
                {"type": "paragraph", "text": "这是一段正文"},
                {"type": "table", "headers": ["列1", "列2"], "rows": [["值1", "值2"]]}
            ]
        });
        let result = build_docx(&content, None);
        assert!(result.is_ok(), "build_docx failed: {:?}", result.err());
        let bytes = result.unwrap();
        assert!(!bytes.is_empty());
        // DOCX 文件头也是 PK (ZIP)
        assert_eq!(&bytes[..2], b"PK");
    }

    /// Excel 单元格样式（bg/border/color/align/bold）实际写入 + 数字文本数值化
    #[test]
    fn test_build_xlsx_cell_styles_and_numeric() {
        let content = json!({
            "sheets": [{
                "name": "测试数据",
                "children": [
                    {"type": "row", "header": true, "cells": [
                        {"align": "center", "bg": "4472C4", "bold": true, "border": true, "color": "FFFFFF", "text": "序号"},
                        {"align": "center", "bg": "4472C4", "bold": true, "border": true, "color": "FFFFFF", "text": "金额"}
                    ]},
                    {"type": "row", "cells": [
                        {"align": "center", "border": true, "text": "1"},
                        {"border": true, "text": "100"}
                    ]},
                    {"type": "row", "cells": [
                        {"align": "center", "border": true, "text": "2"},
                        {"border": true, "text": "250"}
                    ]},
                    {"type": "row", "cells": [
                        {"bold": true, "border": true, "bg": "D9E1F2", "text": "合计"},
                        {"border": true, "formula": "=SUM(B2:B3)"}
                    ]},
                    {"type": "column_width", "widths": [8, 14]}
                ]
            }]
        });
        let bytes = build_xlsx(&content).expect("build_xlsx failed");

        // 1. styles.xml 应包含背景色与边框定义（样式真的写进了文件）
        {
            use std::io::Read;
            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
            let mut styles = zip.by_name("xl/styles.xml").unwrap();
            let mut s = String::new();
            styles.read_to_string(&mut s).unwrap();
            let lower = s.to_lowercase();
            assert!(lower.contains("4472c4"), "styles.xml 应包含表头背景色 4472C4");
            assert!(lower.contains("d9e1f2"), "styles.xml 应包含合计行背景色 D9E1F2");
            assert!(lower.contains("border"), "styles.xml 应包含边框定义");
        }

        // 2. calamine 读回：数字文本应已数值化（否则 SUM 公式失效）
        {
            use calamine::Reader as _;
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("styles_test.xlsx");
            std::fs::write(&path, &bytes).unwrap();
            let mut wb = calamine::open_workbook_auto(&path).expect("calamine 打开失败");
            let names = wb.sheet_names().to_vec();
            let range = wb.worksheet_range(&names[0]).expect("读取工作表失败");
            let mut has_100 = false;
            let mut has_250 = false;
            let mut text_count = 0;
            for row in range.rows() {
                for c in row {
                    match c {
                        calamine::Data::Int(100) => has_100 = true,
                        calamine::Data::Int(250) => has_250 = true,
                        calamine::Data::Float(f) if *f == 100.0 => has_100 = true,
                        calamine::Data::Float(f) if *f == 250.0 => has_250 = true,
                        calamine::Data::String(s) => {
                            if s == "100" || s == "250" {
                                text_count += 1;
                            }
                        }
                        _ => {}
                    }
                }
            }
            assert!(has_100 && has_250, "数字文本应已数值化（100/250 需为数值类型）");
            assert_eq!(text_count, 0, "100/250 不应仍是文本");
        }
    }

    /// 强制默认样式：零样式字段的纯标量 content，输出也必须带边框 + 表头蓝底白字居中加粗
    #[test]
    fn test_xlsx_forced_default_styles() {
        use std::io::Read;
        let content = json!({
            "sheets": [{
                "name": "表1",
                "children": [
                    {"type": "row", "header": true, "cells": ["名称", "数量"]},
                    {"type": "row", "cells": ["产品A", 10]}
                ]
            }]
        });
        let bytes = build_xlsx(&content).expect("build_xlsx failed");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        let mut styles = String::new();
        zip.by_name("xl/styles.xml")
            .unwrap()
            .read_to_string(&mut styles)
            .unwrap();
        let lower = styles.to_lowercase();
        assert!(lower.contains("border"), "默认应带边框定义");
        assert!(lower.contains("4472c4"), "表头默认应有蓝底 4472C4");
        assert!(lower.contains("ffffff"), "表头默认应有白字 FFFFFF");
        assert!(
            styles.contains(r#"horizontal="center""#),
            "表头默认应居中对齐"
        );
        assert!(styles.contains("<b/>") || styles.contains("<b></b>"), "表头默认应加粗");
    }

    /// 显式 border:false 不套默认边框；垂直居中预设使函数恒返回 Some
    /// （"无默认边框"由 border match 分支保证，Format 无公开 getter 故不深检）
    #[test]
    fn test_xlsx_explicit_border_false_respected() {
        let cell = json!({"text": "无边框", "border": false});
        assert!(
            build_cell_format(&cell, false).is_some(),
            "预设垂直居中/字体使任何 cell 均带格式（显式 border:false 不再产生默认边框）"
        );
        // header 零字段 → 应有默认样式（bold+center+bg+color）
        assert!(
            build_cell_format(&json!({}), true).is_some(),
            "header 行即使零字段也应有默认样式"
        );
        // 数据行零字段 → 仍有默认边框
        assert!(
            build_cell_format(&json!({}), false).is_some(),
            "数据行零字段也应有默认边框"
        );
    }

    /// 强制默认样式（Word）：零样式的平面标量表 → 表头浅灰底黑字加粗居中、全表细边框
    #[test]
    fn test_docx_forced_default_table_styles() {
        let content = json!({
            "children": [
                {"type": "table", "headers": ["列A", "列B"], "rows": [["x", "y"], ["1", "2"]]}
            ]
        });
        let bytes = build_docx(&content, None).expect("build_docx failed");
        let doc = rwml::Document::open(&bytes).expect("重新打开失败");
        let model = doc.model();
        let table = model
            .blocks
            .iter()
            .find_map(|b| match b {
                rwml::Block::Table(t) => Some(t),
                _ => None,
            })
            .expect("应生成表格");
        assert_eq!(table.header_rows, 1, "headers 数组应标记 1 行表头");

        // 表头单元格：浅灰底 + 加粗 + 居中
        let header_cell = &table.rows[0].cells[0];
        assert!(header_cell.is_header, "第一行应为表头单元格");
        assert_eq!(
            header_cell.shading,
            Some(rwml::Color::rgb(0xD9, 0xD9, 0xD9)),
            "表头默认应有浅灰底纹 D9D9D9"
        );
        let rwml::Block::Paragraph(hp) = &header_cell.blocks[0] else {
            panic!("表头单元格应有段落");
        };
        assert!(
            hp.runs.iter().any(|r| r.props.bold),
            "表头单元格应默认加粗"
        );
        assert_eq!(hp.props.align, rwml::Align::Center, "表头段落应居中");

        // 数据行单元格：无底纹、不强制居中
        let data_cell = &table.rows[1].cells[0];
        assert!(data_cell.shading.is_none(), "数据行不应有底纹");
        if let Some(rwml::Block::Paragraph(dp)) = data_cell.blocks.first() {
            assert_ne!(dp.props.align, rwml::Align::Center, "数据行不应强制居中");
            assert!(
                !dp.runs.iter().any(|r| r.props.bold),
                "数据行不应默认加粗"
            );
        }
    }

    /// 容错（docx 对齐 Excel 三层容错）：缺 type 推断、容器递归、纯字符串段落、未知 type 带 children 递归
    #[test]
    fn test_build_docx_missing_type_fallback() {
        let content = json!({
            "children": [
                {"text": "无type段落"},
                {"items": ["条目A", "条目B"]},
                {"headers": ["列一", "列二"], "rows": [["值1", "值2"]]},
                {"text": "外层文本", "children": [{"text": "内层文本"}]},
                "纯字符串块",
                {"type": "section", "children": [{"text": "容器内文本"}]},
                {"level": 2, "text": "推断标题"}
            ]
        });
        let bytes = build_docx(&content, None).expect("build_docx failed（容错应保证生成成功）");
        assert_eq!(&bytes[..2], b"PK");

        // rwml 读回：所有文本都必须进入文档模型（内容零丢失）
        let doc = rwml::Document::open(&bytes).expect("重新打开失败");
        let text = doc.text();
        for expected in [
            "无type段落",
            "条目A",
            "条目B",
            "列一",
            "值1",
            "外层文本",
            "内层文本",
            "纯字符串块",
            "容器内文本",
            "推断标题",
        ] {
            assert!(text.contains(expected), "文档应包含「{}」，实际文本: {}", expected, text);
        }
        // 推断标题应为 heading（level 2）而非普通段落
        let model = doc.model();
        let has_heading = model.blocks.iter().any(|b| {
            matches!(b, rwml::Block::Paragraph(p)
                if p.props.heading_level.is_some()
                    && p.runs.iter().any(|r| r.text.contains("推断标题")))
        });
        assert!(has_heading, "带 level+text 的缺type块应推断为 heading");
    }

    /// TOC 静态目录：toc 块（排最前）应基于预扫描标题生成可见的目录列表，
    /// 而非 rwml 的 dirty TOC 域（Word 打开不更新域时显示为空）
    #[test]
    fn test_docx_toc_static_list() {
        let content = json!({
            "children": [
                {"type": "toc", "start": 1, "end": 3},
                {"type": "heading", "level": 1, "text": "第一章 概述"},
                {"type": "heading", "level": 2, "text": "1.1 背景"},
                {"level": 2, "text": "缺type推断标题"},
                {"type": "paragraph", "text": "正文内容"}
            ]
        });
        let bytes = build_docx(&content, None).expect("build_docx failed");
        let doc = rwml::Document::open(&bytes).expect("重新打开失败");
        let text = doc.text();

        // 静态目录行：标题文本全部可见（toc 在最前，靠预扫描拿到后置标题）
        for expected in ["第一章 概述", "1.1 背景", "缺type推断标题"] {
            assert!(text.contains(expected), "目录/标题应包含「{}」，实际: {}", expected, text);
        }
        // 目录列表应排在正文之前：第一章出现在"正文内容"之前
        let pos_toc = text.find("第一章").expect("应有第一章");
        let pos_body = text.find("正文内容").expect("应有正文");
        assert!(pos_toc < pos_body, "静态目录应生成在正文之前");

        // 标题仍是 heading 样式（供 Word 插入原生带页码目录用）
        let model = doc.model();
        let heading_count = model.blocks.iter().filter(|b| {
            matches!(b, rwml::Block::Paragraph(p) if p.props.heading_level.is_some())
        }).count();
        assert!(heading_count >= 3, "应有至少3个 heading，实际: {}", heading_count);
    }

    /// toc 但零标题：保留原 dirty TOC 域兜底（不 panic、可生成）
    #[test]
    fn test_docx_toc_without_headings_keeps_field() {
        let content = json!({
            "children": [
                {"type": "toc", "start": 1, "end": 3},
                {"type": "paragraph", "text": "只有正文没有标题"}
            ]
        });
        let bytes = build_docx(&content, None).expect("零标题 toc 也应生成成功");
        assert_eq!(&bytes[..2], b"PK");
    }

    /// xlsx 行高自适应：AI 传 row_height 块被忽略（不生成 ht 属性），行高交由 Excel 按内容自适应
    #[test]
    fn test_build_xlsx_row_height_ignored() {
        use std::io::Read;
        let content = json!({
            "sheets": [{
                "name": "表1",
                "children": [
                    {"type": "row", "header": true, "cells": ["列A", "列B"]},
                    {"type": "row", "cells": ["值1", "值2"]},
                    {"type": "row_height", "row": 0, "height": 24}
                ]
            }]
        });
        let bytes = build_xlsx(&content).expect("row_height 块应被安全忽略而非报错");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        let mut sheet = String::new();
        zip.by_name("xl/worksheets/sheet1.xml")
            .unwrap()
            .read_to_string(&mut sheet)
            .unwrap();
        // 行属性形式 ` ht="`（勿用裸 ht= ——会误中 defaultRowHeight="15"）
        assert!(
            !sheet.contains(r#" ht=""#),
            "行高不得固化（Excel 自适应文本），实际: {}",
            sheet
        );
    }

    /// Word 表格恒铺满页宽（fixed layout）且行高自适应：tblW=5000pct + tblLayout fixed + 无 trHeight
    #[test]
    fn docx_table_full_width_autofit_height() {
        let xml = docx_part(
            &json!({"children": [
                {"type": "table", "headers": ["甲", "乙"], "rows": [
                    ["内容很长的单元格文本内容很长的单元格文本内容很长的单元格文本", "短"]
                ]},
            ]}),
            "document.xml",
        );
        assert!(
            xml.contains(r#"<w:tblW w:w="5000" w:type="pct"/>"#),
            "表格应恒铺满页宽(5000=100%): {}",
            &xml[..xml.len().min(700)]
        );
        assert!(xml.contains(r#"<w:tblLayout w:type="fixed"/>"#), "固定布局防内容撑破列宽");
        assert!(!xml.contains("<w:trHeight"), "不得写死行高（Word 按单元格文本自动撑开）");
    }

    /// Word 样式：段落对齐/底纹、run 颜色、表格边框/列宽/单元格样式
    #[test]
    fn test_build_docx_styles() {
        let content = json!({
            "children": [
                {"type": "heading", "level": 1, "text": "居中标题", "align": "center", "shading": "EEEEEE"},
                {"type": "paragraph", "align": "right", "runs": [
                    {"text": "红色加粗", "bold": true, "color": "FF0000"}
                ]},
                {"type": "table",
                 "headers": ["列1", "列2"],
                 "rows": [
                     [{"text": "A", "bg": "D9E1F2", "bold": true}, {"text": "B", "color": "00FF00"}],
                     ["x", "y"]
                 ],
                 "border": true, "border_color": "000000", "col_widths_pct": [0.3, 0.7]}
            ]
        });
        let bytes = build_docx(&content, None).expect("build_docx failed");
        assert_eq!(&bytes[..2], b"PK");

        // rwml 读回验证样式进入文档模型
        let doc = rwml::Document::open(&bytes).expect("重新打开失败");
        let model = doc.model();
        let mut has_center_para = false;
        let mut has_red_run = false;
        let mut has_table = false;
        for block in &model.blocks {
            match block {
                rwml::Block::Paragraph(p) => {
                    if p.props.align == rwml::Align::Center {
                        has_center_para = true;
                    }
                    for run in &p.runs {
                        if run.props.color == Some(rwml::Color::rgb(255, 0, 0)) {
                            has_red_run = true;
                        }
                    }
                }
                rwml::Block::Table(_) => has_table = true,
                _ => {}
            }
        }
        assert!(has_center_para, "段落对齐(center)应进入文档模型");
        assert!(has_red_run, "run 颜色(FF0000)应进入文档模型");
        assert!(has_table, "带样式的表格应生成");
    }

    // ── F: 秒级时间戳文件名（区分会话内同名文件）──

    /// 时间戳后缀：设备清单 → 设备清单_YYYYMMDD_HHMMSS.docx（精确到秒）
    #[test]
    fn file_name_gets_second_precision_timestamp() {
        let n = build_file_name("设备清单", "docx");
        let re = regex::Regex::new(r"^设备清单_\d{8}_\d{6}\.docx$").unwrap();
        assert!(re.is_match(&n), "实际: {}", n);
        // 截断预算：总长 ≤100
        assert!(n.chars().count() <= 100, "过长: {}", n.chars().count());
    }

    /// 非法字符清洗 + 已带扩展名不双拼 + 空输入兜底
    #[test]
    fn file_name_sanitizes_and_avoids_double_ext() {
        // 已带扩展名：时间戳插在扩展名前，不出现 .docx.docx
        let n = build_file_name("报告.docx", "docx");
        let re = regex::Regex::new(r"^报告_\d{8}_\d{6}\.docx$").unwrap();
        assert!(re.is_match(&n), "实际: {}", n);
        // 非法字符 → 下划线
        let n2 = build_file_name("a/b:c", "xlsx");
        assert!(n2.starts_with("a_b_c_"), "实际: {}", n2);
        assert!(n2.ends_with(".xlsx"));
        assert!(!n2.contains(':'));
        // 空输入 → 文档兜底
        let n3 = build_file_name("   ", "docx");
        assert!(n3.starts_with("文档_"), "实际: {}", n3);
    }

    // ── header/footer 的 {PAGE}/{NUMPAGES} 占位 → Word 页码域 ──

    /// 占位符分段纯逻辑
    #[test]
    fn split_running_placeholders_segments_fields() {
        let segs = split_running_placeholders("第 {PAGE} 页 共 {NUMPAGES} 页");
        assert_eq!(segs.len(), 5, "{:?}", segs);
        assert_eq!(segs[0].0, "第 ");
        assert!(segs[0].1.is_none());
        assert_eq!(segs[1].1, Some("PAGE"));
        assert_eq!(segs[2].0, " 页 共 ");
        assert_eq!(segs[3].1, Some("NUMPAGES"));
        assert_eq!(segs[4].0, " 页");
        assert!(segs[4].1.is_none());

        // 无占位 → 单段纯文本
        let s2 = split_running_placeholders("公司内部文件");
        assert_eq!(s2.len(), 1);
        assert!(s2[0].1.is_none());

        // 占位开头 + 连续占位
        let s3 = split_running_placeholders("{PAGE}/{NUMPAGES}");
        assert_eq!(s3.len(), 3, "{:?}", s3);
        assert_eq!(s3[0].1, Some("PAGE"));
        assert_eq!(s3[1].0, "/");
        assert_eq!(s3[2].1, Some("NUMPAGES"));
    }

    /// 集成：生成的 docx footer part 必须含 fldSimple 域指令，不得残留字面 {PAGE}
    #[test]
    fn footer_page_placeholder_becomes_field() {
        use std::io::Read as _;
        let content = json!({"children": [
            {"type": "paragraph", "text": "正文"},
            {"type": "footer", "text": "第 {PAGE} 页 共 {NUMPAGES} 页"},
        ]});
        let bytes = build_docx(&content, None).expect("build_docx");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).expect("open docx zip");
        let names: Vec<String> = (0..zip.len())
            .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
            .collect();
        let footer_name = names.iter().find(|n| n.contains("footer")).expect("应有 footer part");
        let mut xml = String::new();
        zip.by_name(footer_name).unwrap().read_to_string(&mut xml).unwrap();
        assert!(xml.contains("fldSimple"), "footer 应含 fldSimple 域: {}", &xml[..xml.len().min(500)]);
        assert!(xml.contains("PAGE"), "应含 PAGE 域指令");
        assert!(xml.contains("NUMPAGES"), "应含 NUMPAGES 域指令");
        assert!(!xml.contains("{PAGE}"), "不得残留字面 {{PAGE}}");
    }

    /// 兜底页码页脚：无 footer 块时自动注入右对齐"第 X 页 共 Y 页"（域 + 小五宋体预设）
    #[test]
    fn docx_default_page_footer_when_absent() {
        let xml = docx_part(&json!({"children": [{"type": "paragraph", "text": "正文"}]}), "footer");
        assert!(xml.contains("fldSimple"), "应含页码域: {}", &xml[..xml.len().min(700)]);
        assert!(xml.contains("PAGE") && xml.contains("NUMPAGES"), "应含 PAGE 与 NUMPAGES 域");
        assert!(xml.contains(r#"<w:jc w:val="right"/>"#), "页码页脚应右对齐");
        assert!(xml.contains(r#"w:sz w:val="18""#) && xml.contains("宋体"), "应为小五宋体预设");
        assert!(xml.contains("第") && xml.contains("页"), "应含固定文案");
    }

    /// AI 已提供自定义页脚 → 不叠加兜底页码（单一页脚、无域）
    #[test]
    fn docx_ai_footer_suppresses_default() {
        let xml = docx_part(&json!({"children": [
            {"type": "paragraph", "text": "正文"},
            {"type": "footer", "text": "内部资料·禁止外发"},
        ]}), "footer");
        assert!(xml.contains("内部资料·禁止外发"), "应保留自定义页脚");
        assert!(!xml.contains("fldSimple"), "不得叠加兜底页码页脚");
    }

    // ── 字体字号预设（方案A：缺省补齐，显式覆盖） ──

    /// 解压 docx 取 part 文本（测试 helper）
    fn docx_part(content: &Value, name_contains: &str) -> String {
        use std::io::Read as _;
        let bytes = build_docx(content, None).expect("build_docx");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).expect("open zip");
        let names: Vec<String> = (0..zip.len())
            .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
            .collect();
        let name = names.iter().find(|n| n.contains(name_contains) && !n.contains("_rels"))
            .unwrap_or_else(|| panic!("无匹配 part: {}（现有 {:?}）", name_contains, names));
        let mut xml = String::new();
        zip.by_name(name).unwrap().read_to_string(&mut xml).unwrap();
        xml
    }

    /// 正文纯文本段落缺省 → 宋体小四（w:sz 24）+ 1.5 倍行距（w:line=360）
    #[test]
    fn docx_body_preset_font_size() {
        let xml = docx_part(&json!({"children": [{"type": "paragraph", "text": "默认正文"}]}), "document.xml");
        assert!(xml.contains(r#"w:sz w:val="24""#), "正文应为小四 24half: {}", &xml[..xml.len().min(600)]);
        assert!(xml.contains("宋体"), "正文应为宋体");
        assert!(xml.contains(r#"w:line="360""#), "正文应为 1.5 倍行距(w:line=360): {}", &xml[..xml.len().min(600)]);
    }

    /// 标题：级别字号预设（H1=小二 36half/18pt）且保留 Heading 样式引用（大纲/目录能力）；标题 1.5 倍行距
    #[test]
    fn docx_heading_preset_size_keeps_style() {
        let xml = docx_part(&json!({"children": [
            {"type": "heading", "level": 1, "text": "一级标题"},
            {"type": "heading", "level": 2, "text": "二级标题"},
        ]}), "document.xml");
        assert!(xml.contains(r#"w:sz w:val="36""#), "H1 应为小二 36half(18pt)");
        assert!(xml.contains(r#"w:sz w:val="32""#), "H2 应为 32half(16pt)");
        assert!(xml.contains("Heading1") || xml.contains("outlineLvl"), "标题应保留样式/大纲引用");
        assert!(xml.contains("宋体"), "标题字体应为宋体");
        assert!(xml.contains(r#"w:line="360""#), "标题应为 1.5 倍行距(w:line=360)");
    }

    /// text_type 选择文本类型（caption=小五）；自由 font/size 字段已废除（传了忽略）
    #[test]
    fn docx_text_type_selects_size_and_free_fields_ignored() {
        let xml = docx_part(&json!({"children": [
            {"type": "paragraph", "runs": [
                {"text": "附注说明", "text_type": "caption"},
                {"text": "默认正文"},
                {"text": "试图覆盖", "size_pt": 99, "font": "Comic Sans"}
            ]},
        ]}), "document.xml");
        assert!(xml.contains(r#"w:sz w:val="18""#), "caption 应为小五 18half");
        assert!(xml.contains(r#"w:sz w:val="24""#), "body 应为小四 24half");
        assert!(xml.contains("宋体"), "字体恒宋体");
        assert!(!xml.contains("Comic Sans"), "自由 font 字段应被忽略");
        assert!(!xml.contains(r#"w:val="198""#), "自由 size_pt=99(198half) 应被忽略");
    }

    /// 表格单元格缺省 → 宋体小四（24half=12pt，与正文一致）
    #[test]
    fn docx_table_cell_preset() {
        let xml = docx_part(&json!({"children": [
            {"type": "table", "headers": ["列"], "rows": [["值"]]},
        ]}), "document.xml");
        assert!(xml.contains(r#"w:sz w:val="24""#), "表格单元格应为小四 24half");
        assert!(xml.contains("宋体"), "表格单元格应为宋体");
    }

    /// 页眉页脚缺省 → 宋体小五（18half=9pt）
    #[test]
    fn docx_footer_plain_preset() {
        let xml = docx_part(&json!({"children": [
            {"type": "paragraph", "text": "正文"},
            {"type": "footer", "text": "公司内部资料"},
        ]}), "footer");
        assert!(xml.contains(r#"w:sz w:val="18""#), "页脚应为小五 18half");
        assert!(xml.contains("宋体"), "页脚应为宋体");
    }

    /// 表格单元格垂直对齐：默认上下居中（center）；valign 显式值覆盖
    /// （rwml 对 Top 不输出 vAlign——Word 默认即顶对齐，显式 top = 清除居中标记）
    #[test]
    fn docx_table_cell_valign_defaults_center() {
        let xml = docx_part(&json!({"children": [
            {"type": "table", "headers": ["列"], "rows": [
                ["默认居中"],
                [{"text": "顶对齐", "valign": "top"}]
            ]},
        ]}), "document.xml");
        // 3 格：表头 + 默认行 → center（2 个）；显式 top 行不输出 vAlign（不被强制居中）
        let center_count = xml.matches(r#"<w:vAlign w:val="center"/>"#).count();
        assert_eq!(center_count, 2, "表头与默认格应上下居中、top 格不强制: {}", &xml[..xml.len().min(900)]);
        assert!(!xml.contains(r#"<w:vAlign w:val="top"/>"#), "Top 由 rwml 省略输出（Word 默认顶对齐）");
    }

    /// xlsx：styles.xml 含宋体 + 11pt（build_cell_format 缺省补齐 + 调用侧零格式归一）；
    /// text_type=caption → 9pt
    #[test]
    fn xlsx_preset_font_in_styles() {
        use std::io::Read as _;
        let content = json!({"sheets": [{"name": "S1", "rows": [
            ["表头"], ["数据"], [{"text": "无边框", "border": false}], [{"text": "附注", "text_type": "caption"}]
        ]}]});
        let bytes = build_xlsx(&content).expect("build_xlsx");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).expect("open xlsx zip");
        let mut xml = String::new();
        zip.by_name("xl/styles.xml").unwrap().read_to_string(&mut xml).unwrap();
        assert!(xml.contains("宋体"), "xlsx 样式表应含宋体: {}", &xml[..xml.len().min(600)]);
        assert!(xml.contains(r#"val="11""#), "xlsx 默认字号应含 11pt");
        assert!(xml.contains(r#"val="9""#), "caption 应为 9pt");
        assert!(xml.contains(r#"vertical="center""#), "所有单元格应垂直居中");
    }

    /// Word 列表：预设样式（宋体小四 + 1.5倍行距）+ 字面编号
    /// （组间独立计数、嵌套自动二级圆点、剥 AI 自带前缀防双重编号、层级缩进）
    #[test]
    fn docx_list_preset_style_and_numbering() {
        let xml = docx_part(&json!({"children": [
            {"type": "list", "ordered": true, "items": ["第一步", "1. 自带序号第二步"]},
            {"type": "list", "ordered": true, "items": ["新组应从1开始"]},
            {"type": "list", "items": [{"text": "父项", "children": ["子项甲"]}]},
        ]}), "document.xml");
        assert!(xml.contains(r#"w:sz w:val="24""#), "列表应宋体小四");
        assert!(xml.contains(r#"w:line="360""#), "列表应 1.5 倍行距");
        // 前缀与正文现为独立 run（行内加粗支持）：分别断言存在、无双重编号残留
        assert!(
            xml.contains(">2. <") && xml.contains("自带序号第二步") && !xml.contains("1. 自带"),
            "自带前缀应被剥除并统一重编号: {}",
            &xml[..xml.len().min(1600)]
        );
        assert!(xml.contains(">1. ") && xml.contains("新组应从1开始"), "有序组间编号应独立重置");
        assert!(!xml.contains("1. 1."), "不得双重前缀");
        assert!(xml.contains(">◦ ") && xml.contains("子项甲"), "嵌套子级应为二级圆点");
        assert!(xml.contains("w:left"), "列表应有层级缩进");
    }

    /// 行内 markdown 解析：成对标记 → 样式段；工程文本/未闭合 → 原样字面
    #[test]
    fn md_inline_runs_parser_table() {
        let segs = |s: &str| md_inline_runs(s);
        let plain = |st: &InlineStyle| !st.bold && !st.italic && !st.code;
        // 基础粗体
        let r = segs("采用**冗余**设计");
        assert_eq!(r.len(), 3);
        assert_eq!(r[1].0, "冗余");
        assert!(r[1].1.bold && !r[0].1.bold && plain(&r[2].1));
        // 斜体（中文紧邻合法）
        let r = segs("重点*双机热备*架构");
        assert_eq!(r[1].0, "双机热备");
        assert!(r[1].1.italic && !r[1].1.bold);
        // 组合 ***x*** = 粗+斜
        let r = segs("***组合***");
        assert!(r.len() == 1 && r[0].0 == "组合" && r[0].1.bold && r[0].1.italic);
        // 代码段
        let r = segs("执行 `select 1` 语句");
        assert_eq!(r[1].0, "select 1");
        assert!(r[1].1.code);
        // 嵌套：粗内含斜
        let r = segs("混**粗*斜*续**尾");
        let has = |txt: &str, b: bool, i: bool| r.iter().any(|(t, st)| t == txt && st.bold == b && st.italic == i);
        assert!(has("斜", true, true) && has("粗", true, false) && has("续", true, false));
        // 工程文本保护：单 * / _ 紧贴 ASCII 字母数字不生效
        for lit in ["1920*1080 分辨率", "算式 1*2*3", "字段 user_name 与 a__b"] {
            let r = segs(lit);
            assert_eq!(r.iter().map(|(t, _)| t.as_str()).collect::<String>(), lit, "应原样: {}", lit);
            assert!(r.iter().all(|(_, st)| plain(st)), "不应产生样式: {}", lit);
        }
        // 未闭合标记原样保留不吞字
        let r = segs("未闭合 **粗体 与 *斜 尾巴");
        assert_eq!(r.iter().map(|(t, _)| t.as_str()).collect::<String>(), "未闭合 **粗体 与 *斜 尾巴");
        assert!(r.iter().all(|(_, st)| plain(st)));
        // 剥除工具
        assert_eq!(md_strip_inline("一、**总体**架构"), "一、总体架构");
        assert!(md_fully_bold("**合计**"));
        assert!(!md_fully_bold("**局部**加粗其余"));
        assert!(!md_fully_bold("**"));
    }

    /// docx 渲染：段落/表格 cell 部分加粗为独立 run 且无 ** 残留；标题与目录剥标记
    #[test]
    fn docx_inline_markdown_rendering() {
        let xml = docx_part(&json!({"children": [
            {"type": "paragraph", "text": "采用**双机热备**保障可用性"},
            {"type": "table", "rows": [["项目", "说明"], ["SLA", "**99.999%** 承诺"]]},
            {"type": "heading", "level": 2, "text": "二、**建设**目标"},
        ]}), "document.xml");
        assert!(xml.contains("双机热备") && xml.contains("<w:b/>"), "段落应出现加粗 run");
        assert!(xml.contains("99.999%"), "cell 文本保留");
        assert!(!xml.contains("**"), "全文档不得残留 ** 标记: {}", &xml[..xml.len().min(400)]);
        assert!(xml.contains("二、建设目标"), "标题剥标记保留文本");
    }

    /// docx 显式 runs 通道：run.text 内的行内标记同样解析（叠加基底样式）
    #[test]
    fn docx_explicit_runs_inline_md() {
        let xml = docx_part(&json!({"children": [{
            "type": "paragraph",
            "runs": [{"text": "总体：承诺 **99.99%** 可用性", "italic": true}]
        }]}), "document.xml");
        assert!(xml.contains("99.99%") && xml.contains("<w:b/>"), "显式 run 内标记应转粗体");
        assert!(!xml.contains("**"), "无标记残留: {}", &xml[..xml.len().min(400)]);
        assert!(xml.contains("<w:i/>"), "基底 italic 样式保留");
    }

    /// xlsx：整格 **包裹** → 整格加粗（styles.xml 出现 bold 字体）且无标记残留
    #[test]
    fn xlsx_wrap_bold_cell_and_strip() {
        use std::io::Read as _;
        let content = json!({"sheets": [{"name": "S1", "rows": [
            ["**合计**"], ["**局部**其余不加粗"], ["普通文本"]
        ]}]});
        let bytes = build_xlsx(&content).expect("build_xlsx");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).expect("open xlsx zip");
        let mut styles = String::new();
        zip.by_name("xl/styles.xml").unwrap().read_to_string(&mut styles).unwrap();
        assert!(styles.contains("<b/>"), "整格包裹应产生加粗字体: {}", &styles[..styles.len().min(500)]);
        let mut sheet = String::new();
        zip.by_name("xl/worksheets/sheet1.xml").unwrap().read_to_string(&mut sheet).unwrap();
        let mut ss = String::new();
        if let Ok(mut f) = zip.by_name("xl/sharedStrings.xml") {
            f.read_to_string(&mut ss).unwrap();
        }
        let text = format!("{}{}", sheet, ss);
        assert!(!text.contains("**"), "xlsx 单元格不得残留 ** 标记");
        assert!(text.contains("合计") && text.contains("局部") && text.contains("普通文本"), "文本逐字保留");
    }
}
