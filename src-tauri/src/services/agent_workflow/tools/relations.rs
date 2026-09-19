//! ext_tool_relations：实体关联查询（双向）

use crate::database;
use serde_json::{json, Value};
use crate::services::agent_workflow::*;

pub(crate) fn execute_relations(entity_type: &str, entity_id: &str, target_type: Option<&str>, limit: usize, offset: usize) -> String {
    let limit = limit.clamp(1, 100);
    if entity_type.is_empty() || entity_id.is_empty() {
        let envelope = ToolResultEnvelope {
            result_type: "relations_result".to_string(),
            data: json!({"source": {"type": entity_type, "id": entity_id}, "total": 0, "relations": []}),
            summary: Some("请提供 entity_type 和 entity_id。".to_string()),
        };
        return envelope.to_json();
    }
    let upper = entity_type.to_uppercase();
    // 去掉 TYPE- 前缀，business_relations 存储裸 UUID
    let bare_id = entity_id.strip_prefix(&format!("{}-", upper)).unwrap_or(entity_id);

    // 正向查询 + 反向查询 business_relations（含 metadata 供图谱导航透传）
    let forward_sql = "SELECT br.target_type, br.target_id, br.metadata, sd.title
        FROM business_relations br
        LEFT JOIN source_data sd ON sd.source_type = br.target_type AND sd.source_id = br.target_id
        WHERE br.source_type = ?1 AND br.source_id = ?2";
    let reverse_sql = "SELECT br.source_type as target_type, br.source_id as target_id, br.metadata, sd.title
        FROM business_relations br
        LEFT JOIN source_data sd ON sd.source_type = br.source_type AND sd.source_id = br.source_id
        WHERE br.target_type = ?1 AND br.target_id = ?2";

    let params = vec![json!(upper), json!(bare_id)];
    let mut all_rels: Vec<(String, String, String, String)> = Vec::new();
    let mut query_errors: Vec<String> = Vec::new();

    match crate::database::all(forward_sql, &params) {
        Ok(rows) => {
            for r in &rows {
                if let (Some(tt), Some(tid)) = (r["target_type"].as_str(), r["target_id"].as_str()) {
                    if !tt.is_empty() && !tid.is_empty() {
                        all_rels.push((tt.to_uppercase(), tid.to_string(), r["title"].as_str().unwrap_or("").to_string(), r["metadata"].as_str().unwrap_or("").to_string()));
                    }
                }
            }
        }
        Err(e) => query_errors.push(format!("正向: {}", e)),
    }
    match crate::database::all(reverse_sql, &params) {
        Ok(rows) => {
            for r in &rows {
                if let (Some(tt), Some(tid)) = (r["target_type"].as_str(), r["target_id"].as_str()) {
                    if !tt.is_empty() && !tid.is_empty() {
                        all_rels.push((tt.to_uppercase(), tid.to_string(), r["title"].as_str().unwrap_or("").to_string(), r["metadata"].as_str().unwrap_or("").to_string()));
                    }
                }
            }
        }
        Err(e) => query_errors.push(format!("反向: {}", e)),
    }

    // 去重 + 过滤自身 + 按 target_type 过滤
    all_rels.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    all_rels.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
    all_rels.retain(|(t, id, _, _)| !(t == &upper && id == entity_id));
    if let Some(tt) = target_type {
        let tt_upper = tt.to_uppercase();
        all_rels.retain(|(t, _, _, _)| t == &tt_upper);
    }

    let total = all_rels.len();

    // 获取源实体名称
    let source_name = crate::database::get(
        "SELECT title FROM source_data WHERE source_type=?1 AND source_id=?2",
        &[json!(&upper), json!(bare_id)],
    ).ok().flatten().and_then(|r| r["title"].as_str().map(String::from)).unwrap_or_default();

    // 分页
    let start = offset.min(total);
    let end = (offset + limit).min(total);

    let relations: Vec<Value> = all_rels[start..end].iter().map(|(tt, tid, name, metadata)| {
        // 图谱节点（方案/版本/设备实例）不在 source_data 中，直接从业务表解析名称
        let resolved = if name.is_empty() { resolve_graph_node_name(tt, tid) } else { name.clone() };
        let mut relation = json!({
            "type": tt,
            "name": if resolved.is_empty() { tid.as_str() } else { resolved.as_str() },
            "id": tid
        });
        // 透传边 metadata（如版本金额/状态、设备数量/单价）
        if !metadata.is_empty() {
            if let Ok(meta) = serde_json::from_str::<Value>(metadata) {
                if meta.as_object().map(|o| !o.is_empty()).unwrap_or(false) {
                    relation["metadata"] = meta;
                }
            }
        }
        // CONTACT 类型额外返回 phone、email、position
        if tt == "CONTACT" {
            if let Ok(Some(contact)) = database::get(
                "SELECT phone, email, position FROM partner_contacts WHERE id=?1",
                &[json!(tid)]
            ) {
                if let Some(phone) = contact["phone"].as_str() {
                    if !phone.is_empty() { relation["phone"] = json!(phone); }
                }
                if let Some(email) = contact["email"].as_str() {
                    if !email.is_empty() { relation["email"] = json!(email); }
                }
                if let Some(position) = contact["position"].as_str() {
                    if !position.is_empty() { relation["position"] = json!(position); }
                }
            }
        }
        relation
    }).collect();

    let summary = if total == 0 {
        if !query_errors.is_empty() {
            format!("{} 关联查询失败：{}（结果不可信，非'无关联'）", source_name, query_errors.join("；"))
        } else {
            format!("{} 没有关联对象。", source_name)
        }
    } else if total > relations.len() {
        format!("{}关联 {} 个对象，本页返回第 {}-{} 条，还有更多未返回——需要完整列表时用 offset={} 继续获取。",
            source_name, total, offset + 1, offset + relations.len(), offset + relations.len())
    } else {
        format!("{}关联 {} 个对象。", source_name, total)
    };

    let envelope = ToolResultEnvelope {
        result_type: "relations_result".to_string(),
        data: json!({
            "source": {
                "type": upper,
                "id": entity_id,
                "name": source_name
            },
            "total": total,
            "offset": offset,
            "limit": limit,
            "relations": relations
        }),
        summary: Some(summary),
    };
    envelope.to_json()
}

/// 解析图谱节点名称（SOLUTION/SOLUTION_VERSION/SOLUTION_ITEM 不进 source_data，直接查业务表）
pub(crate) fn resolve_graph_node_name(node_type: &str, id: &str) -> String {
    let sql = match node_type {
        "SOLUTION" => "SELECT name FROM solutions WHERE id=?1",
        "SOLUTION_VERSION" => "SELECT s.name || ' · ' || sv.version AS name FROM solution_versions sv JOIN solutions s ON s.id=sv.solution_id WHERE sv.id=?1",
        "SOLUTION_ITEM" => "SELECT CASE WHEN si.device_name IS NOT NULL AND si.device_name != '' THEN si.device_name || ' · ' || si.product_name ELSE si.product_name END AS name FROM solution_items si WHERE si.id=?1",
        _ => return String::new(),
    };
    database::get(sql, &[json!(id)]).ok().flatten()
        .and_then(|r| r["name"].as_str().map(String::from))
        .unwrap_or_default()
}

