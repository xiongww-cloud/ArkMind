//! 文档写入：AI 设计的 JSON 内容 → Word(.docx) / Excel(.xlsx) → CAS 存储 → file_id
//!
//! 生成的文档通过 file_assets + file_relations(entity_type='generated') 入库：
//! - 复用现有 file_preview / file_export / file_copy_to_clipboard（预览/下载/复制）
//! - entity_type='generated' 不出现在资料库/项目文件列表（相关查询均带类型过滤）
//! - generated_docs 表保存原始 content JSON：多轮修改时 action=read 回读 + 30 天清理锚点

use crate::commands::file_commands::flat_disk_path;
use crate::database;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// 生成文档保留天数：启动时清理超过该天数的文档
const GENERATED_DOC_RETENTION_DAYS: i64 = 30;

pub struct GeneratedDoc {
    pub file_id: String,
    pub file_name: String,
    pub file_type: String,
    pub file_size: u64,
}

// ═══════════════════════════════════════════════════════════════
// 公共 API
// ═══════════════════════════════════════════════════════════════

/// 生成 Word 文档并入库，返回文档对象
pub fn write_docx(filename: &str, content: &Value, metadata: Option<&Value>) -> Result<GeneratedDoc, String> {
    let bytes = build_docx(content, metadata)?;
    save_bytes(&bytes, filename, "docx", content)
}

/// 生成 Excel 文档并入库，返回文档对象
pub fn write_xlsx(filename: &str, content: &Value) -> Result<GeneratedDoc, String> {
    let bytes = build_xlsx(content)?;
    save_bytes(&bytes, filename, "xlsx", content)
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

/// 启动清理：删除超过保留期的生成文档（记录 + 关系 + CAS 物理文件）
pub fn cleanup_generated_docs() {
    // 1. 孤儿清理：资产已被别处删除的残留记录
    let _ = database::run(
        "DELETE FROM generated_docs WHERE file_id NOT IN (SELECT id FROM file_assets)",
        &[],
    );

    // 2. 超期清理
    let sql = format!(
        "SELECT gd.file_id FROM generated_docs gd \
         WHERE gd.created_at < datetime('now','localtime','-{} days')",
        GENERATED_DOC_RETENTION_DAYS
    );
    let rows = database::all(&sql, &[]).unwrap_or_default();
    if rows.is_empty() {
        return;
    }
    let total = rows.len();
    for r in &rows {
        let fid = r["file_id"].as_str().unwrap_or("");
        if fid.is_empty() {
            continue;
        }
        let _ = database::run("DELETE FROM generated_docs WHERE file_id=?1", &[json!(fid)]);
        let _ = database::run(
            "DELETE FROM file_relations WHERE file_id=?1 AND entity_type='generated'",
            &[json!(fid)],
        );
        // 零引用释放：清知识库索引（若曾被补漏捞入）+ 物理文件 + 资产行；仍被引用时 no-op
        crate::commands::file_commands::release_asset_if_orphan(fid);
    }
    log::info!("[文档清理] 已清理 {} 个超过 {} 天的生成文档", total, GENERATED_DOC_RETENTION_DAYS);
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

// ═══════════════════════════════════════════════════════════════
// 入库：CAS 落盘 + file_assets + file_relations + generated_docs
// ═══════════════════════════════════════════════════════════════

fn save_bytes(bytes: &[u8], filename: &str, ext: &str, content: &Value) -> Result<GeneratedDoc, String> {
    if bytes.is_empty() {
        return Err("生成的文件内容为空".to_string());
    }
    let file_name = build_file_name(filename, ext);
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
                "INSERT INTO file_assets(id,file_ext,file_size,content_hash,original_name,created_at,updated_at) \
                 VALUES(?1,?2,?3,?4,?5,datetime('now','localtime'),datetime('now','localtime'))",
                &[json!(&id), json!(ext), json!(size), json!(&hash), json!(&file_name)],
            )
            .map_err(|e| e.to_string())?;
            id
        }
    };

    // file_relations(entity_type='generated')：预览/下载/复制的解析依赖此关系。
    // 不走 add_relation：避免 upsert 进业务关系图谱（生成文档不是业务实体）。
    database::run(
        "INSERT OR IGNORE INTO file_relations(id,file_id,entity_type,entity_id,custom_name,creator) \
         VALUES(?1,?2,'generated','',?3,'AI')",
        &[json!(uuid::Uuid::new_v4().to_string()), json!(&file_id), json!(&file_name)],
    )
    .map_err(|e| e.to_string())?;
    // CAS 复用同内容时刷新显示名（同内容不同文件名的边界场景）
    database::run(
        "UPDATE file_relations SET custom_name=?1 WHERE file_id=?2 AND entity_type='generated'",
        &[json!(&file_name), json!(&file_id)],
    )
    .ok();

    // 原始 content 快照：多轮 read 回读 + 30 天清理锚点（OR REPLACE 刷新保留期）
    database::run(
        "INSERT OR REPLACE INTO generated_docs(file_id, content, created_at) \
         VALUES(?1,?2,datetime('now','localtime'))",
        &[json!(&file_id), json!(content.to_string())],
    )
    .map_err(|e| e.to_string())?;

    log::info!("[文档生成] {} 已保存 file_id={} size={}B", file_name, file_id, size);
    Ok(GeneratedDoc {
        file_id,
        file_name,
        file_type: ext.to_string(),
        file_size: size as u64,
    })
}

/// 文件名消毒：去路径分隔符与非法字符，保证非空并带扩展名
fn build_file_name(filename: &str, ext: &str) -> String {
    const ILLEGAL: [char; 9] = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    let base: String = filename
        .trim()
        .chars()
        .map(|c| if ILLEGAL.contains(&c) || (c as u32) < 0x20 { '_' } else { c })
        .collect();
    let base = base.trim().trim_matches('.');
    let base: String = base.chars().take(100).collect();
    let base = if base.is_empty() { "文档".to_string() } else { base };
    if base.to_lowercase().ends_with(&format!(".{}", ext)) {
        base
    } else {
        format!("{}.{}", base, ext)
    }
}

// ═══════════════════════════════════════════════════════════════
// Word (.docx) 构建 — rwml DocBuilder
// ═══════════════════════════════════════════════════════════════

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
            builder = builder.heading(1, t);
        }
        for line in text.split('\n') {
            if !line.trim().is_empty() {
                builder = builder.paragraph(line.trim_end());
            }
        }
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
                builder = builder.paragraph(s.trim_end());
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
                if align.is_some() || shading.is_some() {
                    // 带样式 → ParagraphBuilder（对齐/底纹）
                    let mut pb = rwml::ParagraphBuilder::text(text).heading_level(level);
                    if let Some(a) = align {
                        pb = pb.align(a);
                    }
                    if let Some((r, g, b)) = shading {
                        pb = pb.shading(rwml::Color::rgb(r, g, b));
                    }
                    builder = builder.rich_paragraph(pb.build());
                } else {
                    builder = builder.heading(level, text);
                }
            }
            "paragraph" | "p" => {
                let align = parse_docx_align(child);
                let shading = docx_shading_color(child);
                let styled = align.is_some() || shading.is_some();
                if let Some(runs) = child["runs"].as_array() {
                    let built: Vec<rwml::Run> = runs.iter().filter_map(build_run).collect();
                    if !built.is_empty() {
                        if styled {
                            let mut pb = rwml::ParagraphBuilder::new().runs(built);
                            if let Some(a) = align {
                                pb = pb.align(a);
                            }
                            if let Some((r, g, b)) = shading {
                                pb = pb.shading(rwml::Color::rgb(r, g, b));
                            }
                            builder = builder.rich_paragraph(pb.build());
                        } else {
                            builder = builder.paragraph_runs(built);
                        }
                    }
                } else {
                    let text = child["text"].as_str().unwrap_or("");
                    for line in text.split('\n') {
                        if line.trim().is_empty() {
                            continue;
                        }
                        if styled {
                            let mut pb = rwml::ParagraphBuilder::text(line.trim_end());
                            if let Some(a) = align {
                                pb = pb.align(a);
                            }
                            if let Some((r, g, b)) = shading {
                                pb = pb.shading(rwml::Color::rgb(r, g, b));
                            }
                            builder = builder.rich_paragraph(pb.build());
                        } else {
                            builder = builder.paragraph(line.trim_end());
                        }
                    }
                }
            }
            "list" | "bullet_list" | "numbered_list" => {
                let items: Vec<String> = child["items"]
                    .as_array()
                    .map(|a| a.iter().map(value_to_plain_string).collect())
                    .unwrap_or_default();
                if items.is_empty() {
                    continue;
                }
                let ordered = child["ordered"]
                    .as_bool()
                    .unwrap_or(kind == "numbered_list");
                let level = child["level"].as_u64().unwrap_or(0).min(8) as u8;
                builder = builder.list_level(items, ordered, level);
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
                    builder = builder.header(v);
                }
            }
            "footer" => {
                if let Some(v) = str_field(child, "text") {
                    builder = builder.footer(v);
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
                    builder = builder.paragraph(t);
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
                        builder = builder.paragraph(format!("{}{}", indent, text));
                    }
                }
            }
            _ => {
                // 未知 type：有 children 则递归展开（内容不丢，模型常编造 section/article 等容器名）
                if let Some(nested) = child["children"].as_array() {
                    if let Some(t) = str_field(child, "text") {
                        builder = builder.paragraph(t);
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
                        out.push((level, t));
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

fn build_run(r: &Value) -> Option<rwml::Run> {
    let text = r["text"].as_str().filter(|t| !t.is_empty())?;
    let mut b = rwml::RunBuilder::new(text);
    if r["bold"].as_bool().unwrap_or(false) { b = b.bold(); }
    if r["italic"].as_bool().unwrap_or(false) { b = b.italic(); }
    if r["underline"].as_bool().unwrap_or(false) { b = b.underline(); }
    if r["small_caps"].as_bool().unwrap_or(false) { b = b.small_caps(); }
    if let Some(f) = str_field(r, "font") { b = b.font(f); }
    if let Some(sz) = r["size_half_pt"].as_u64() {
        b = b.size_half_pt(sz.min(u16::MAX as u64) as u16);
    } else if let Some(pt) = r["size_pt"].as_u64() {
        b = b.size_half_pt((pt.saturating_mul(2)).min(u16::MAX as u64) as u16);
    }
    if let Some(hl) = str_field(r, "highlight") { b = b.highlight(hl); }
    if let Some(url) = str_field(r, "hyperlink") { b = b.hyperlink(url); }
    // 文字颜色（hex 如 "FF0000" / "#FF0000"）
    if let Some((cr, cg, cb)) = str_field(r, "color").and_then(|s| parse_hex_rgb(&s)) {
        b = b.color(rwml::Color::rgb(cr, cg, cb));
    }
    Some(b.build())
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
    if let Some(wp) = child["width_pct"].as_f64() {
        tb = tb.width_pct(wp as f32);
    }

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

    // 段落内容：空文本 cell 仍可由 shading 承载底纹（cell 级属性不依赖 blocks）
    let blocks = if text.is_empty() {
        vec![]
    } else if bold || color.is_some() {
        let mut rb = rwml::RunBuilder::new(text.as_str());
        if bold {
            rb = rb.bold();
        }
        if let Some((r, g, b)) = color {
            rb = rb.color(rwml::Color::rgb(r, g, b));
        }
        let mut pb = rwml::ParagraphBuilder::new().push_run(rb.build());
        if let Some(a) = align {
            pb = pb.align(a);
        }
        vec![rwml::Block::Paragraph(pb.build())]
    } else {
        let mut pb = rwml::ParagraphBuilder::text(text.as_str());
        if let Some(a) = align {
            pb = pb.align(a);
        }
        vec![rwml::Block::Paragraph(pb.build())]
    };

    rwml::Cell {
        blocks,
        is_header: cell_header,
        shading,
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
    use rust_xlsxwriter::{Chart, ChartType, Format, Image};

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
                let row = child["row"].as_u64().unwrap_or(row_cursor as u64) as u32;
                if let Some(h) = child["height"].as_f64() {
                    ws.set_row_height(row, h)
                        .map_err(|e| format!("设置行高失败: {}", e))?;
                }
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
                            if let Err(e) = ws.merge_range(r1, c1, r2, c2, text, &Format::new()) {
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
        let fmt = build_cell_format(cell, is_header);

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

/// 从单元格 JSON 构建样式 Format；无样式字段且非 header 行时返回 None
/// 支持：bold/italic/border/border_color/bg/color/align/valign/wrap/num_format/font/size/underline/strike
fn build_cell_format(cell: &Value, is_header: bool) -> Option<rust_xlsxwriter::Format> {
    use rust_xlsxwriter::{Format, FormatAlign, FormatBorder, FormatUnderline};

    // 默认常量：Office 经典表头配色（AI 未传样式时的兜底，显式传值永远优先）
    const DEFAULT_HEADER_BG: &str = "4472C4";
    const DEFAULT_HEADER_COLOR: &str = "FFFFFF";

    let mut fmt = Format::new();
    let mut has = false;

    // bold：显式字段优先，header 行默认加粗
    if cell["bold"].as_bool().unwrap_or(is_header) {
        fmt = fmt.set_bold();
        has = true;
    }
    if cell["italic"].as_bool().unwrap_or(false) {
        fmt = fmt.set_italic();
        has = true;
    }

    // border：显式 false 尊重（无边框）；显式 true/字符串用其值；未传 → 默认全表细边框
    match cell.get("border") {
        Some(Value::Bool(false)) => { /* 尊重显式关闭 */ }
        Some(Value::Bool(true)) => {
            fmt = fmt.set_border(FormatBorder::Thin);
            has = true;
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
            has = true;
        }
        // 未传（含标量 cell）→ 默认细线边框，保证表格网格完整
        _ => {
            fmt = fmt.set_border(FormatBorder::Thin);
            has = true;
        }
    }
    if let Some(c) = cell["border_color"].as_str().and_then(norm_hex) {
        fmt = fmt.set_border_color(c.as_str());
        has = true;
    }

    // 背景色：显式优先；header 行未传 → 默认 Office 蓝
    let bg_explicit = cell["bg"]
        .as_str()
        .or_else(|| cell["background"].as_str());
    if let Some(raw) = bg_explicit {
        if let Some(c) = norm_hex(raw) {
            fmt = fmt.set_background_color(c.as_str());
            has = true;
        }
    } else if is_header {
        fmt = fmt.set_background_color(DEFAULT_HEADER_BG);
        has = true;
    }

    // 字体颜色：显式优先；header 行未传 → 默认白字（配表头底色）
    if let Some(c) = cell["color"].as_str().and_then(norm_hex) {
        fmt = fmt.set_font_color(c.as_str());
        has = true;
    } else if is_header {
        fmt = fmt.set_font_color(DEFAULT_HEADER_COLOR);
        has = true;
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
            has = true;
        }
    } else if is_header {
        fmt = fmt.set_align(FormatAlign::Center);
        has = true;
    }
    // 垂直对齐
    if let Some(v) = cell["valign"].as_str() {
        let fa = match v.to_ascii_lowercase().as_str() {
            "center" | "middle" | "vertical-center" => Some(FormatAlign::VerticalCenter),
            "top" => Some(FormatAlign::Top),
            "bottom" => Some(FormatAlign::Bottom),
            _ => None,
        };
        if let Some(fa) = fa {
            fmt = fmt.set_align(fa);
            has = true;
        }
    }

    // 自动换行：显式 wrap 或文本含换行符
    if cell["wrap"].as_bool().unwrap_or(false) {
        fmt = fmt.set_text_wrap();
        has = true;
    } else if cell["text"].as_str().map_or(false, |t| t.contains('\n')) {
        fmt = fmt.set_text_wrap();
        has = true;
    }

    // 数字格式
    if let Some(nf) = str_field(cell, "num_format") {
        fmt = fmt.set_num_format(nf);
        has = true;
    }
    // 字体
    if let Some(name) = str_field(cell, "font") {
        fmt = fmt.set_font_name(name);
        has = true;
    }
    if let Some(sz) = cell["size"].as_f64() {
        fmt = fmt.set_font_size(sz);
        has = true;
    }
    if cell["underline"].as_bool().unwrap_or(false) {
        fmt = fmt.set_underline(FormatUnderline::Single);
        has = true;
    }
    if cell["strike"].as_bool().unwrap_or(false)
        || cell["strikethrough"].as_bool().unwrap_or(false)
    {
        fmt = fmt.set_font_strikethrough();
        has = true;
    }

    if has {
        Some(fmt)
    } else {
        None
    }
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

    /// 显式 border:false 且无其他样式 → 不套默认边框（尊重 AI 显式关闭）
    #[test]
    fn test_xlsx_explicit_border_false_respected() {
        let cell = json!({"text": "无边框", "border": false});
        assert!(
            build_cell_format(&cell, false).is_none(),
            "border:false 且无其他样式时不应套默认边框"
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

    /// 行高坐标0基回归：row_height{row:0} 必须落在第一行，第二行不受污染
    /// （历史 bug：AI 按1基传 row:1 意图表头，0基解释落到第二行 → schema 已标注基准）
    #[test]
    fn test_build_xlsx_row_height_zero_based() {
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
        let bytes = build_xlsx(&content).expect("build_xlsx failed");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        let mut sheet = String::new();
        zip.by_name("xl/worksheets/sheet1.xml")
            .unwrap()
            .read_to_string(&mut sheet)
            .unwrap();
        // 第一行有 ht=24
        assert!(
            sheet.contains(r#"<row r="1""#) && sheet.contains(r#"ht="24""#),
            "第一行应被设为 24 磅: {}",
            sheet
        );
        // 第二行不带 ht（默认行高）
        let r2_start = sheet.find(r#"<row r="2""#).expect("应有第二行");
        let r2_end = sheet[r2_start..].find('>').map(|p| r2_start + p).unwrap();
        let r2_tag = &sheet[r2_start..r2_end];
        assert!(
            !r2_tag.contains("ht="),
            "第二行不应被设置行高（0基 row:0 只作用第一行），实际: {}",
            r2_tag
        );
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
}
