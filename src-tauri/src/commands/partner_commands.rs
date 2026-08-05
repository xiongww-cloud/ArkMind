use serde_json::{json, Value};
use crate::database;

// ═══════════════════════════════════════════════════════════════════
// 统一合作伙伴 CRUD（基于 partners 表 + business_relations）
// ═══════════════════════════════════════════════════════════════════

/// 列出项目关联的合作伙伴（project_id 为空时返回全局列表）
#[tauri::command]
pub async fn partner_list(project_id: String, partner_type: Option<String>) -> Result<Vec<Value>, String> {
    let is_global = project_id.is_empty();
    let (sql, params): (String, Vec<Value>) = if is_global {
        if let Some(ref pt) = partner_type {
            ("SELECT DISTINCT p.* FROM partners p WHERE p.type = ?1".into(), vec![json!(pt)])
        } else {
            ("SELECT DISTINCT p.* FROM partners p".into(), vec![])
        }
    } else if let Some(ref pt) = partner_type {
        (
            "SELECT p.*, ppm.remark as project_remark FROM partners p INNER JOIN partner_project_mapping ppm ON p.id = ppm.partner_id WHERE ppm.project_id = ?1 AND p.type = ?2".into(),
            vec![json!(&project_id), json!(pt)]
        )
    } else {
        (
            "SELECT p.*, ppm.remark as project_remark FROM partners p INNER JOIN partner_project_mapping ppm ON p.id = ppm.partner_id WHERE ppm.project_id = ?1".into(),
            vec![json!(&project_id)]
        )
    };
    let partners = database::all(&sql, &params).map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    for p in &partners {
        let pid = p["id"].as_str().unwrap_or("");
        // 查该项目下该 Partner 的联系人
        let contact_ids: Vec<String> = if is_global {
            database::all(
                "SELECT DISTINCT contact_id FROM partner_contact_mapping WHERE partner_id=?",
                &[json!(pid)]
            ).unwrap_or_default().iter()
                .filter_map(|r| r["contact_id"].as_str().map(String::from))
                .collect()
        } else {
            database::all(
                "SELECT DISTINCT pcm.contact_id FROM partner_contact_mapping pcm INNER JOIN contact_project_mapping cpm ON pcm.contact_id = cpm.contact_id WHERE pcm.partner_id=?1 AND cpm.project_id=?2",
                &[json!(pid), json!(&project_id)]
            ).unwrap_or_default().iter()
                .filter_map(|r| r["contact_id"].as_str().map(String::from))
                .collect()
        };
        let contacts = if contact_ids.is_empty() {
            vec![]
        } else {
            let ph: String = contact_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let params: Vec<Value> = contact_ids.iter().map(|id| json!(id)).collect();
            database::all(&format!("SELECT DISTINCT * FROM partner_contacts WHERE id IN ({})", ph), &params).unwrap_or_default()
        };
        let mut obj = p.clone();
        obj["selected_contact_id"] = json!("");
        obj["contacts"] = json!(contacts);
        result.push(obj);
    }
    Ok(result)
}

/// 创建合作伙伴
#[tauri::command]
pub async fn partner_create(data: Value) -> Result<Value, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let partner_type = data["type"].as_str().unwrap_or("CHANNEL").to_string();
    let name = data["name"].as_str().unwrap_or("").to_string();
    let phone = data["phone"].as_str().unwrap_or("").to_string();
    let company = data["company"].as_str().unwrap_or("").to_string();
    let remark = data["remark"].as_str().unwrap_or("").to_string();
    let contact_person = data["contact_person"].as_str().unwrap_or("").to_string();
    let contact_phone = data["contact_phone"].as_str().unwrap_or("").to_string();
    let pid = data["projectId"].as_str().unwrap_or("").to_string();
    
    database::run(
        "INSERT INTO partners(id,type,name,phone,address,remark,contact_person,contact_phone) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        &[json!(&id), json!(&partner_type), json!(&name), json!(&phone), json!(&company), json!(&remark), json!(&contact_person), json!(&contact_phone)]
    ).map_err(|e| e.to_string())?;
    
    if !pid.is_empty() {
        database::run(
            "INSERT OR IGNORE INTO partner_project_mapping(partner_id, project_id, partner_type, remark) VALUES(?, ?, ?, ?)",
            &[json!(&id), json!(&pid), json!(&partner_type), json!("")]
        ).ok();
        crate::services::business_relations::upsert(&partner_type, &id, "PROJECT", &pid, "belongs_to", "{}");
    }
    crate::services::knowledge_lifecycle::notify_changed(&partner_type, &id);
    emit_for_project(&pid);
    Ok(json!({"success":true,"id":id,"name":name}))
}

/// 更新合作伙伴
#[tauri::command]
pub async fn partner_update(data: Value) -> Result<Value, String> {
    let id = data["id"].as_str().ok_or("missing id")?;
    let mut fields: Vec<String> = Vec::new();
    let mut values: Vec<Value> = Vec::new();
    if let Some(v) = data["name"].as_str() { fields.push("name=?".into()); values.push(json!(v)); }
    if let Some(v) = data["phone"].as_str() { fields.push("phone=?".into()); values.push(json!(v)); }
    if let Some(v) = data["company"].as_str() { fields.push("address=?".into()); values.push(json!(v)); }
    if let Some(v) = data["remark"].as_str() { fields.push("remark=?".into()); values.push(json!(v)); }
    if let Some(v) = data["contact_person"].as_str() { fields.push("contact_person=?".into()); values.push(json!(v)); }
    if let Some(v) = data["contact_phone"].as_str() { fields.push("contact_phone=?".into()); values.push(json!(v)); }
    if fields.is_empty() { return Ok(json!({"success":true})); }
    fields.push("updated_at=datetime('now','localtime')".into());
    values.push(json!(id));
    database::run(&format!("UPDATE partners SET {} WHERE id=?", fields.join(",")), &values).map_err(|e| e.to_string())?;
    let ptype = database::get("SELECT type FROM partners WHERE id=?", &[json!(id)]).ok().flatten().and_then(|r| r["type"].as_str().map(String::from)).unwrap_or_else(|| "CHANNEL".into());
    crate::services::knowledge_lifecycle::notify_changed(&ptype, id);
    emit_for_partner(id);
    Ok(json!({"success":true}))
}

/// 删除合作伙伴
#[tauri::command]
pub async fn partner_delete(partner_id: String) -> Result<Value, String> {
    // 去掉可能的 TYPE- 前缀（CHANNEL-xxx / VENDOR-xxx / CUSTOMER-xxx → xxx）
    let clean_id = partner_id
        .strip_prefix("CHANNEL-").or_else(|| partner_id.strip_prefix("VENDOR-")).or_else(|| partner_id.strip_prefix("CUSTOMER-"))
        .unwrap_or(&partner_id);
    let ptype = database::get("SELECT type FROM partners WHERE id=?", &[json!(clean_id)]).ok().flatten().and_then(|r| r["type"].as_str().map(String::from)).unwrap_or_else(|| "CHANNEL".into());
    crate::services::knowledge_lifecycle::notify_deleted(&ptype, clean_id);
    crate::services::source_doc_sync::delete_entity_source_doc(&ptype, clean_id);
    // 清理 partner_project_mapping
    database::run("DELETE FROM partner_project_mapping WHERE partner_id=?", &[json!(clean_id)]).ok();
    // 清理该 Partner 下所有联系人→项目的 contact_project_mapping
    database::run("DELETE FROM contact_project_mapping WHERE contact_id IN (SELECT contact_id FROM partner_contact_mapping WHERE partner_id=?)", &[json!(clean_id)]).ok();
    crate::services::business_relations::delete_by_source(&ptype, clean_id);
    emit_for_partner(clean_id);
    // 删除关联的联系人记录
    let contact_ids: Vec<String> = database::all("SELECT contact_id FROM partner_contact_mapping WHERE partner_id=?", &[json!(clean_id)]).unwrap_or_default()
        .into_iter().filter_map(|r| r["contact_id"].as_str().map(String::from)).collect();
    for cid in &contact_ids {
        // M4: 先删当前伙伴的映射
        database::run("DELETE FROM partner_contact_mapping WHERE partner_id=? AND contact_id=?", &[json!(clean_id), json!(cid)]).ok();
        database::run("DELETE FROM contact_project_mapping WHERE contact_id=?", &[json!(cid)]).ok();
        // M4: 检查联系人是否还挂在其他伙伴下，只有无其他映射时才删除联系人本身
        let other_mappings: i64 = database::with_db(|c| c.query_row(
            "SELECT COUNT(*) FROM partner_contact_mapping WHERE contact_id=?", rusqlite::params![cid], |r| r.get(0)
        )).unwrap_or(0);
        if other_mappings == 0 {
            crate::services::knowledge_lifecycle::notify_deleted("CONTACT", cid);
            crate::services::source_doc_sync::delete_entity_source_doc("CONTACT", cid);
            crate::services::business_relations::delete_by_source("CONTACT", cid);
            database::run("DELETE FROM partner_contacts WHERE id=?", &[json!(cid)]).ok();
        }
    }
    database::run("DELETE FROM partner_contact_mapping WHERE partner_id=?",&[json!(clean_id)]).ok();
    let changes = database::run("DELETE FROM partners WHERE id=?",&[json!(clean_id)]).map_err(|e| e.to_string())?;
    if changes.changes == 0 {
        return Err(format!("未找到合作伙伴: {}", partner_id));
    }
    Ok(json!({"success":true}))
}

/// 关联合作伙伴到项目（Partner 直接映射，联系人可选）
#[tauri::command]
pub async fn partner_link_project(partner_id: String, project_id: String, contact_id: Option<String>, remark: Option<String>) -> Result<Value, String> {
    let ptype = database::get("SELECT type FROM partners WHERE id=?", &[json!(&partner_id)]).ok().flatten().and_then(|r| r["type"].as_str().map(String::from)).unwrap_or_else(|| "CHANNEL".into());
    // WRITE 1: partner_project_mapping（始终写入，Partner 直接关联项目）
    database::run(
        "INSERT OR IGNORE INTO partner_project_mapping(partner_id, project_id, partner_type, remark) VALUES(?, ?, ?, ?)",
        &[json!(&partner_id), json!(&project_id), json!(&ptype), json!(remark.as_deref().unwrap_or(""))]
    ).ok();
    // WRITE 2: business_relations（Partner → Project）
    crate::services::business_relations::upsert(&ptype, &partner_id, "PROJECT", &project_id, "belongs_to", "{}");
    // 如果指定了联系人，同时关联联系人到项目
    if let Some(ref cid) = contact_id {
        if !cid.is_empty() {
            database::run(
                "INSERT OR IGNORE INTO contact_project_mapping(contact_id, project_id) VALUES(?, ?)",
                &[json!(cid), json!(&project_id)]
            ).ok();
            crate::services::business_relations::upsert("CONTACT", cid, "PROJECT", &project_id, "belongs_to", "{}");
        }
    }
    crate::services::knowledge_lifecycle::notify_changed(&ptype, &partner_id);
    Ok(json!({"success":true}))
}

/// 取消合作伙伴与项目的关联
#[tauri::command]
pub async fn partner_unlink_project(partner_id: String, project_id: String) -> Result<Value, String> {
    let ptype = database::get("SELECT type FROM partners WHERE id=?", &[json!(&partner_id)]).ok().flatten().and_then(|r| r["type"].as_str().map(String::from)).unwrap_or_else(|| "CHANNEL".into());
    // DELETE 1: partner_project_mapping
    database::run(
        "DELETE FROM partner_project_mapping WHERE partner_id=? AND project_id=?",
        &[json!(&partner_id), json!(&project_id)]
    ).ok();
    // DELETE 2: 该 Partner 下所有联系人→该项目的 contact_project_mapping
    database::run(
        "DELETE FROM contact_project_mapping WHERE project_id=?1 AND contact_id IN (SELECT contact_id FROM partner_contact_mapping WHERE partner_id=?2)",
        &[json!(&project_id), json!(&partner_id)]
    ).ok();
    // DELETE 3: business_relations（Partner → Project）
    crate::services::business_relations::delete(&ptype, &partner_id, "PROJECT", &project_id, "belongs_to");
    // DELETE 4: business_relations（该 Partner 下联系人的 CONTACT → Project）
    let contact_ids: Vec<String> = database::all(
        "SELECT contact_id FROM partner_contact_mapping WHERE partner_id=?",
        &[json!(&partner_id)]
    ).unwrap_or_default().iter()
        .filter_map(|r| r["contact_id"].as_str().map(String::from))
        .collect();
    for cid in &contact_ids {
        crate::services::business_relations::delete("CONTACT", cid, "PROJECT", &project_id, "belongs_to");
    }
    Ok(json!({"success":true}))
}

/// 更新合作伙伴的项目级备注
#[tauri::command]
pub async fn partner_update_remark(partner_id: String, project_id: String, remark: Option<String>) -> Result<Value, String> {
    let new_remark = remark.as_deref().unwrap_or("");
    // UPDATE 1: partner_project_mapping
    database::run(
        "UPDATE partner_project_mapping SET remark=? WHERE partner_id=? AND project_id=?",
        &[json!(new_remark), json!(&partner_id), json!(&project_id)]
    ).ok();
    // UPDATE 2: contact_project_mapping（该 Partner 下所有联系人→该项目的 remark）
    database::run(
        "UPDATE contact_project_mapping SET remark=? WHERE project_id=?1 AND contact_id IN (SELECT contact_id FROM partner_contact_mapping WHERE partner_id=?2)",
        &[json!(new_remark), json!(&project_id), json!(&partner_id)]
    ).ok();
    Ok(json!({"success":true}))
}

// ═══════════════════════════════════════════════════════════════════
// 联系人 CRUD（独立对象，通过 partner_contact_mapping 关联到 Partner）
// ═══════════════════════════════════════════════════════════════════

/// 创建联系人
#[tauri::command]
pub async fn partner_contact_create(data: Value) -> Result<Value, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let partner_id = data["partnerId"].as_str().or(data["channelId"].as_str()).or(data["vendorId"].as_str()).or(data["customerId"].as_str()).unwrap_or("").to_string();
    let name = data["name"].as_str().unwrap_or("").to_string();
    let phone = data["phone"].as_str().unwrap_or("").to_string();
    let email = data["email"].as_str().unwrap_or("").to_string();
    let position = data["position"].as_str().unwrap_or("").to_string();
    let remark = data["remark"].as_str().unwrap_or("").to_string();
    database::run("INSERT INTO partner_contacts(id,name,phone,email,position,remark) VALUES(?1,?2,?3,?4,?5,?6)", &[json!(id),json!(&name),json!(&phone),json!(&email),json!(&position),json!(&remark)]).map_err(|e|e.to_string())?;
    crate::services::knowledge_lifecycle::notify_changed("CONTACT", &id);
    if !partner_id.is_empty() {
        let ptype = database::get("SELECT type FROM partners WHERE id=?", &[json!(&partner_id)]).ok().flatten().and_then(|r| r["type"].as_str().map(String::from)).unwrap_or_else(|| "CHANNEL".into());
        database::run("INSERT OR IGNORE INTO partner_contact_mapping(partner_id,contact_id) VALUES(?1,?2)", &[json!(&partner_id), json!(&id)]).map_err(|e|e.to_string())?;
        crate::services::business_relations::upsert("CONTACT", &id, &ptype, &partner_id, "employed_by", "{}");
        emit_for_partner(&partner_id);
    }
    Ok(json!({"success":true,"id":id,"name":name,"phone":phone,"email":email,"position":position,"remark":remark}))
}

/// 更新联系人
#[tauri::command]
pub async fn partner_contact_update(data: Value) -> Result<Value, String> {
    let id = data["id"].as_str().ok_or("missing id")?;
    let mut fields: Vec<String> = Vec::new();
    let mut values: Vec<Value> = Vec::new();
    if let Some(v) = data["name"].as_str() { fields.push("name=?".into()); values.push(json!(v)); }
    if let Some(v) = data["phone"].as_str() { fields.push("phone=?".into()); values.push(json!(v)); }
    if let Some(v) = data["email"].as_str() { fields.push("email=?".into()); values.push(json!(v)); }
    if let Some(v) = data["position"].as_str() { fields.push("position=?".into()); values.push(json!(v)); }
    if let Some(v) = data["remark"].as_str() { fields.push("remark=?".into()); values.push(json!(v)); }
    if fields.is_empty() { return Ok(json!({"success":true})); }
    values.push(json!(id));
    database::run(&format!("UPDATE partner_contacts SET {} WHERE id=?", fields.join(",")), &values).map_err(|e| e.to_string())?;
    crate::services::knowledge_lifecycle::notify_changed("CONTACT", id);
    emit_for_partner_contact(id);
    Ok(json!({"success":true}))
}

/// 删除联系人
#[tauri::command]
pub async fn partner_contact_delete(contact_id: String) -> Result<Value, String> {
    // 去掉可能的 CONTACT- 前缀
    let clean_id = contact_id.strip_prefix("CONTACT-").unwrap_or(&contact_id);
    crate::services::knowledge_lifecycle::notify_deleted("CONTACT", clean_id);
    crate::services::source_doc_sync::delete_entity_source_doc("CONTACT", clean_id);
    database::run("DELETE FROM contact_project_mapping WHERE contact_id=?", &[json!(clean_id)]).ok();
    crate::services::business_relations::delete_by_source("CONTACT", clean_id);
    emit_for_partner_contact(clean_id);
    database::run("DELETE FROM partner_contact_mapping WHERE contact_id=?",&[json!(clean_id)]).ok();
    let changes = database::run("DELETE FROM partner_contacts WHERE id=?",&[json!(clean_id)]).map_err(|e| e.to_string())?;
    if changes.changes == 0 {
        return Err(format!("未找到联系人: {}（可能已被其所属合作伙伴删除）", contact_id));
    }
    Ok(json!({"success":true}))
}

/// 关联联系人到项目（主映射方式）
#[tauri::command]
pub async fn contact_link_project(contact_id: String, project_id: String) -> Result<Value, String> {
    // WRITE 1: contact_project_mapping
    database::run(
        "INSERT OR IGNORE INTO contact_project_mapping(contact_id, project_id) VALUES(?, ?)",
        &[json!(&contact_id), json!(&project_id)]
    ).ok();
    // WRITE 2: business_relations
    crate::services::business_relations::upsert("CONTACT", &contact_id, "PROJECT", &project_id, "belongs_to", "{}");
    crate::services::knowledge_lifecycle::notify_changed("CONTACT", &contact_id);
    Ok(json!({"success":true}))
}

/// 取消联系人与项目的关联
#[tauri::command]
pub async fn contact_unlink_project(contact_id: String, project_id: String) -> Result<Value, String> {
    // DELETE 1: contact_project_mapping
    database::run(
        "DELETE FROM contact_project_mapping WHERE contact_id=? AND project_id=?",
        &[json!(&contact_id), json!(&project_id)]
    ).ok();
    // DELETE 2: business_relations
    crate::services::business_relations::delete("CONTACT", &contact_id, "PROJECT", &project_id, "belongs_to");
    Ok(json!({"success":true}))
}

// ═══════════════════════════════════════════════════════════════════
// 全局列表（通讯录用，不过滤 project_id）
// ═══════════════════════════════════════════════════════════════════

/// 获取联系人总数
#[tauri::command]
pub async fn contacts_count() -> Result<i64, String> {
    let row = database::get("SELECT COUNT(*) as cnt FROM partner_contacts", &[])
        .map_err(|e| e.to_string())?;
    Ok(row.and_then(|r| r["cnt"].as_i64()).unwrap_or(0))
}

/// 列出所有合作伙伴（可选按类型过滤）
#[tauri::command]
pub async fn contacts_list_all(partner_type: Option<String>) -> Result<Vec<Value>, String> {
    let (sql, params): (String, Vec<Value>) = if let Some(ref pt) = partner_type {
        ("SELECT * FROM partners WHERE type = ?1 ORDER BY name".into(), vec![json!(pt)])
    } else {
        ("SELECT * FROM partners ORDER BY type, name".into(), vec![])
    };
    let partners = database::all(&sql, &params).map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    for p in &partners {
        let pid = p["id"].as_str().unwrap_or("");
        let contact_ids: Vec<String> = database::all(
            "SELECT DISTINCT contact_id FROM partner_contact_mapping WHERE partner_id=?",
            &[json!(pid)]
        ).unwrap_or_default().iter()
            .filter_map(|r| r["contact_id"].as_str().map(String::from))
            .collect();
        let contacts = if contact_ids.is_empty() {
            vec![]
        } else {
            let ph: String = contact_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let params: Vec<Value> = contact_ids.iter().map(|id| json!(id)).collect();
            database::all(&format!("SELECT DISTINCT * FROM partner_contacts WHERE id IN ({})", ph), &params).unwrap_or_default()
        };
        let mut obj = p.clone();
        obj["contacts"] = json!(contacts);
        result.push(obj);
    }
    Ok(result)
}

// ═══════════════════════════════════════════════════════════════════
// Event Bus helpers
// ═══════════════════════════════════════════════════════════════════

fn emit_for_project(pid: &str) {
    crate::services::object_changed::object_changed(crate::services::object_changed::ObjectType::Project, pid, crate::services::object_changed::Action::Updated);
}

fn emit_for_partner(pid: &str) {
    // 从 partner_project_mapping 读取
    if let Ok(rows) = database::all(
        "SELECT project_id FROM partner_project_mapping WHERE partner_id=?",
        &[json!(pid)]
    ) {
        for r in rows {
            if let Some(proj_id) = r["project_id"].as_str() {
                emit_for_project(proj_id);
            }
        }
    }
    // 从 contact_project_mapping 读取（通过联系人）
    if let Ok(rows) = database::all(
        "SELECT DISTINCT cpm.project_id FROM contact_project_mapping cpm INNER JOIN partner_contact_mapping pcm ON cpm.contact_id = pcm.contact_id WHERE pcm.partner_id=?",
        &[json!(pid)]
    ) {
        for r in rows {
            if let Some(proj_id) = r["project_id"].as_str() {
                emit_for_project(proj_id);
            }
        }
    }
}

fn emit_for_partner_contact(cid: &str) {
    if let Ok(Some(r)) = database::get(
        "SELECT project_id FROM contact_project_mapping WHERE contact_id=?",
        &[json!(cid)]
    ) {
        if let Some(pid) = r["project_id"].as_str() {
            emit_for_project(pid);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// 向后兼容：旧命令别名
// ═══════════════════════════════════════════════════════════════════

// 渠道
#[tauri::command] pub async fn channel_list(project_id: String) -> Result<Vec<Value>, String> { partner_list(project_id, Some("CHANNEL".into())).await }
#[tauri::command] pub async fn channel_create(data: Value) -> Result<Value, String> { let mut d = data; d["type"] = json!("CHANNEL"); partner_create(d).await }
#[tauri::command] pub async fn channel_update(data: Value) -> Result<Value, String> { partner_update(data).await }
#[tauri::command] pub async fn channel_delete(channel_id: String) -> Result<Value, String> { partner_delete(channel_id).await }
#[tauri::command] pub async fn channel_link_project(channel_id: String, project_id: String, contact_id: Option<String>, remark: Option<String>) -> Result<Value, String> { partner_link_project(channel_id, project_id, contact_id, remark).await }
#[tauri::command] pub async fn channel_unlink_project(channel_id: String, project_id: String) -> Result<Value, String> { partner_unlink_project(channel_id, project_id).await }
#[tauri::command] pub async fn channel_contact_create(data: Value) -> Result<Value, String> { partner_contact_create(data).await }
#[tauri::command] pub async fn channel_contact_update(data: Value) -> Result<Value, String> { partner_contact_update(data).await }
#[tauri::command] pub async fn channel_contact_delete(contact_id: String) -> Result<Value, String> { partner_contact_delete(contact_id).await }
#[tauri::command] pub async fn channel_update_remark(channel_id: String, project_id: String, remark: Option<String>) -> Result<Value, String> { partner_update_remark(channel_id, project_id, remark).await }

// 厂商
#[tauri::command] pub async fn vendor_list(project_id: String) -> Result<Vec<Value>, String> { partner_list(project_id, Some("VENDOR".into())).await }
#[tauri::command] pub async fn vendor_create(data: Value) -> Result<Value, String> { let mut d = data; d["type"] = json!("VENDOR"); partner_create(d).await }
#[tauri::command] pub async fn vendor_update(data: Value) -> Result<Value, String> { partner_update(data).await }
#[tauri::command] pub async fn vendor_delete(vendor_id: String) -> Result<Value, String> { partner_delete(vendor_id).await }
#[tauri::command] pub async fn vendor_link_project(vendor_id: String, project_id: String, contact_id: Option<String>, remark: Option<String>) -> Result<Value, String> { partner_link_project(vendor_id, project_id, contact_id, remark).await }
#[tauri::command] pub async fn vendor_unlink_project(vendor_id: String, project_id: String) -> Result<Value, String> { partner_unlink_project(vendor_id, project_id).await }
#[tauri::command] pub async fn vendor_contact_create(data: Value) -> Result<Value, String> { partner_contact_create(data).await }
#[tauri::command] pub async fn vendor_contact_update(data: Value) -> Result<Value, String> { partner_contact_update(data).await }
#[tauri::command] pub async fn vendor_contact_delete(contact_id: String) -> Result<Value, String> { partner_contact_delete(contact_id).await }
#[tauri::command] pub async fn vendor_update_remark(vendor_id: String, project_id: String, remark: Option<String>) -> Result<Value, String> { partner_update_remark(vendor_id, project_id, remark).await }

// 客户
#[tauri::command] pub async fn customer_list() -> Result<Vec<Value>, String> { partner_list("".into(), Some("CUSTOMER".into())).await }
#[tauri::command] pub async fn customer_create(data: Value) -> Result<Value, String> { let mut d = data; d["type"] = json!("CUSTOMER"); partner_create(d).await }
#[tauri::command] pub async fn customer_update(data: Value) -> Result<Value, String> { partner_update(data).await }
#[tauri::command] pub async fn customer_delete(customer_id: String) -> Result<Value, String> { partner_delete(customer_id).await }
#[tauri::command] pub async fn customer_link_project(customer_id: String, project_id: String) -> Result<Value, String> { partner_link_project(customer_id, project_id, None, None).await }
#[tauri::command] pub async fn customer_unlink_project(customer_id: String, project_id: String) -> Result<Value, String> { partner_unlink_project(customer_id, project_id).await }
#[tauri::command] pub async fn customer_contact_create(data: Value) -> Result<Value, String> { partner_contact_create(data).await }
#[tauri::command] pub async fn customer_contact_update(data: Value) -> Result<Value, String> { partner_contact_update(data).await }
#[tauri::command] pub async fn customer_contact_delete(contact_id: String) -> Result<Value, String> { partner_contact_delete(contact_id).await }

// 全局列表
#[tauri::command] pub async fn contacts_list_channels() -> Result<Vec<Value>, String> { contacts_list_all(Some("CHANNEL".into())).await }
#[tauri::command] pub async fn contacts_list_vendors() -> Result<Vec<Value>, String> { contacts_list_all(Some("VENDOR".into())).await }
