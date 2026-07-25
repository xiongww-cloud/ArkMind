//! RAG Interceptor — 检索领域 → file_id 数组翻译器
//!
//! 当大模型发起带业务领域限制的搜索时，此模块充当"翻译官"：
//!  1. 接收领域类型
//!  2. 查 SQLite 映射表（projects / products / library）
//!  3. 输出 file_id 数组，供 FTS5 / LanceDB 执行 WHERE file_id IN (...) 过滤

use crate::database;
use serde_json::{json, Value};

/// 将领域过滤器翻译为 file_id 列表
/// domain: "all" | "library" | "project" | "product"
/// project_ids / product_ids: 具体实体 ID 数组
pub fn translate_to_file_ids(
    domain: &str,
    _tag_names: &[String],
    project_ids: &[String],
    product_ids: &[String],
) -> Vec<String> {
    let mut file_ids: Vec<String> = Vec::new();

    // ── 2. 领域翻译 ──
    match domain {
        "library" => {
            if let Ok(rows) = database::all("SELECT file_id FROM file_relations WHERE entity_type='library'", &[]) {
                for r in &rows {
                    if let Some(id) = r["file_id"].as_str() {
                        file_ids.push(id.to_string());
                    }
                }
            }
        }
        "project" => {
            if !project_ids.is_empty() {
                let placeholders: Vec<String> = (0..project_ids.len()).map(|_| "?".to_string()).collect();
                let sql = format!(
                    "SELECT file_id FROM file_relations WHERE entity_type='project' AND entity_id IN ({})",
                    placeholders.join(",")
                );
                let params: Vec<Value> = project_ids.iter().map(|p| json!(p)).collect();
                if let Ok(rows) = database::all(&sql, &params) {
                    for r in &rows {
                        if let Some(id) = r["file_id"].as_str() {
                            file_ids.push(id.to_string());
                        }
                    }
                }
            }
        }
        "product" => {
            if !product_ids.is_empty() {
                let placeholders: Vec<String> = (0..product_ids.len()).map(|_| "?".to_string()).collect();
                let sql = format!(
                    "SELECT fr.file_id FROM file_relations fr \
                     INNER JOIN products p ON p.id = fr.entity_id \
                     WHERE fr.entity_type='product' AND p.id IN ({})",
                    placeholders.join(",")
                );
                let params: Vec<Value> = product_ids.iter().map(|p| json!(p)).collect();
                if let Ok(rows) = database::all(&sql, &params) {
                    for r in &rows {
                        if let Some(id) = r["file_id"].as_str() {
                            file_ids.push(id.to_string());
                        }
                    }
                }
            }
        }
        _ => {} // "all" — 不过滤，由 FTS5/LanceDB 全局搜索
    }

    // 去重
    file_ids.sort_unstable();
    file_ids.dedup();
    file_ids
}
