//! ext_tool_chunk_navigate：知识块前后翻页

use crate::database;
use serde_json::json;
use crate::services::agent_workflow::*;

pub(crate) fn execute_traverse(chunk_id: &str, edge_type: &str, radius: usize, visited: &std::collections::HashSet<String>, _target_name: &str, traverse_source_counter: &mut std::collections::HashMap<String, usize>) -> String {
    if chunk_id.trim().is_empty() {
        let envelope = ToolResultEnvelope {
            result_type: "chunk_navigate_result".to_string(),
            data: json!({"current_index": 0, "total_chunks": 0, "coverage": 0.0, "chunks": [], "warnings": ["请提供 ChunkID。"]}),
            summary: Some("请提供 ChunkID。".to_string()),
        };
        return envelope.to_json();
    }
    let cid = chunk_id.trim();
    let row = match database::get("SELECT p.chunk_id, p.content, p.chunk_index, p.source_data_id, sd.source_type, sd.source_id FROM knowledge_chunks p LEFT JOIN source_data sd ON sd.id=p.source_data_id WHERE p.chunk_type='parent' AND p.chunk_id LIKE ?1",&[json!(format!("{}%", cid))]) { Ok(Some(r)) => r, _ => {
        let envelope = ToolResultEnvelope {
            result_type: "chunk_navigate_result".to_string(),
            data: json!({"current_index": 0, "total_chunks": 0, "coverage": 0.0, "chunks": [], "warnings": [format!("未找到 ChunkID「{}」对应的块。", cid)]}),
            summary: Some(format!("未找到 ChunkID「{}」对应的块。", cid)),
        };
        return envelope.to_json();
    }};
    let current_index = row["chunk_index"].as_i64().unwrap_or(0);
    let sdid = match row["source_data_id"].as_str() { Some(id) => id.to_string(), None => {
        let envelope = ToolResultEnvelope {
            result_type: "chunk_navigate_result".to_string(),
            data: json!({"current_index": 0, "total_chunks": 0, "coverage": 0.0, "chunks": [], "warnings": ["该块无关联源数据。"]}),
            summary: Some("该块无关联源数据。".to_string()),
        };
        return envelope.to_json();
    }};
    let total: i64 = database::get("SELECT COUNT(*) as cnt FROM knowledge_chunks WHERE source_data_id=?1 AND chunk_type='parent'",&[json!(sdid)]).ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
    let r = radius.max(1).min(20);
    // 将认知边缘映射到物理边缘
    let edge = match edge_type {
        "next" | "Next" => "Next",
        "prev" | "Prev" => "Prev",
        _ => edge_type,
    };
    match edge {
        "Next" | "Prev" => {
            let remaining = total - (current_index + 1);
            let _coverage_pct = ((current_index + 1) as f64 / total as f64 * 100.0) as i64;
            let est_steps = if remaining > 0 { (remaining + r as i64 - 1) / r as i64 } else { 0 };
            let get_chunk = |offset: i64| -> Option<(String, String, i64)> {
                let target = current_index + offset;
                if target < 0 || target >= total { return None; }
                let r2 = database::get("SELECT chunk_id, content, chunk_index FROM knowledge_chunks WHERE source_data_id=?1 AND chunk_type='parent' AND chunk_index=?2",&[json!(sdid),json!(target)]).ok().flatten()?;
                let cid_out = r2["chunk_id"].as_str().unwrap_or("").to_string();
                let short = cid_out.chars().take(8).collect::<String>();
                let preview = r2["content"].as_str().unwrap_or("").to_string();
                Some((short, preview, target))
            };
            let mut chunks = Vec::new();
            let mut warnings = Vec::new();
            let mut visited_indices: Vec<i64> = Vec::new();
            // ★ 遍历 radius 个步长，跳过已访问的 chunk，直到集齐 radius 个新块或到边界
            if edge == "Prev" {
                let mut got = 0usize;
                for step in 1..=r.max(r*2) {
                    if got >= r as usize { break; }
                    if let Some((short, preview, target)) = get_chunk(-(step as i64)) {
                        if visited.contains(&short) { visited_indices.push(target); continue; }
                        got += 1;
                        chunks.push(json!({
                            "chunk_id": short,
                            "index": target + 1,
                            "total": total,
                            "content": preview,
                            "direction": "prev",
                            "step": step
                        }));
                    }
                }
            }
            if edge == "Next" {
                let mut got = 0usize;
                for step in 1..=r.max(r*2) {
                    if got >= r as usize { break; }
                    if let Some((short, preview, target)) = get_chunk(step as i64) {
                        if visited.contains(&short) { visited_indices.push(target); continue; }
                        got += 1;
                        chunks.push(json!({
                            "chunk_id": short,
                            "index": target + 1,
                            "total": total,
                            "content": preview,
                            "direction": "next",
                            "step": step
                        }));
                    } else { break; }
                }
            }
            // ★ 标记已跳过哪些块
            if !visited_indices.is_empty() {
                let skipped: Vec<String> = visited_indices.iter().map(|i| format!("{}", i+1)).collect();
                warnings.push(format!("跳过 {}（已在前面展示过）", skipped.join(", ")));
            }
            // ★ 连续遍历同一文件的阻断信号
            if edge == "Next" || edge == "Prev" {
                // 新 source → 重置计数器；同一 source → 递增
                let consecutive = if let Some(c) = traverse_source_counter.get_mut(&sdid) {
                    *c += 1; *c
                } else {
                    traverse_source_counter.clear();
                    traverse_source_counter.insert(sdid, 1);
                    1
                };
                if consecutive > 5 {
                    warnings.push(format!("已在该文件中连续遍历 {} 次（剩余约 {} 块，预计还需约 {} 步）。建议使用 search 工具定位目标关键词，而非继续翻页。", consecutive, remaining, est_steps));
                }
            }
            // 边界阻断信号：无后续/前置块时明确提示 LLM
            if chunks.is_empty() {
                warnings.push(match edge {
                    "Next" => "已到达当前文档的物理【最末尾】，下方已无后续内容。请立即停止向后漫游，总结已有信息或切换到其他文档。".to_string(),
                    "Prev" => "已到达当前文档的物理【最开头】，上方已无前文内容。请立即停止向前漫游。".to_string(),
                    _ => String::new(),
                });
            }
            let coverage = (current_index + 1) as f64 / total as f64;
            let envelope = ToolResultEnvelope {
                result_type: "chunk_navigate_result".to_string(),
                data: json!({
                    "current_index": current_index + 1,
                    "total_chunks": total,
                    "coverage": coverage,
                    "radius": r,
                    "direction": edge_type,
                    "chunks": chunks,
                    "warnings": warnings
                }),
                summary: Some(format!("已导航到 {}/{} 块，覆盖率 {:.0}%。", current_index + 1, total, coverage * 100.0)),
            };
            envelope.to_json()
        }
        _ => {
            let envelope = ToolResultEnvelope {
                result_type: "chunk_navigate_result".to_string(),
                data: json!({"current_index": 0, "total_chunks": 0, "coverage": 0.0, "chunks": [], "warnings": [format!("不支持的边类型「{}」。支持：next, prev。", edge)]}),
                summary: Some(format!("不支持的边类型「{}」。支持：next, prev。", edge)),
            };
            envelope.to_json()
        }
    }
}

