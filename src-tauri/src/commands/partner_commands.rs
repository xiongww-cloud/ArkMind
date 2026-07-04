use serde_json::{json, Value}; use crate::database;
#[tauri::command] pub async fn channel_list(project_id: String) -> Result<Vec<Value>, String> {
    let partners = database::all(
        "SELECT cp.*, pcm.selected_contact_id FROM channel_partners cp JOIN project_channel_mapping pcm ON cp.id = pcm.channel_id WHERE pcm.project_id = ?",
        &[json!(&project_id)]
    ).map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    for p in &partners {
        let cid = p["id"].as_str().unwrap_or("");
        let selected_id = p["selected_contact_id"].as_str();
        // 只返回选中的联系人（如果有的话），否则返回所有联系人（兼容旧数据）
        let contacts = if let Some(sid) = selected_id {
            database::all("SELECT * FROM channel_contacts WHERE id=?",&[json!(sid)]).unwrap_or_default()
        } else {
            database::all("SELECT * FROM channel_contacts WHERE channel_id=?",&[json!(cid)]).unwrap_or_default()
        };
        let mut obj = p.clone();
        obj["contacts"] = json!(contacts);
        result.push(obj);
    }
    Ok(result)
}
#[tauri::command] pub async fn channel_create(data: Value) -> Result<Value, String> { let id = uuid::Uuid::new_v4().to_string(); let name = data["name"].as_str().unwrap_or("").to_string(); let remark = data["remark"].as_str().unwrap_or("").to_string(); let pid = data["projectId"].as_str().unwrap_or("").to_string(); database::run("INSERT INTO channel_partners(id,project_id,name,phone,company,remark) VALUES(?,?,?,?,?,?)",&[json!(id),json!(&pid),json!(&name),json!(data["phone"].as_str().unwrap_or("")),json!(data["company"].as_str().unwrap_or("")),json!(&remark)]).map_err(|e| e.to_string())?; if !pid.is_empty() { crate::services::business_relations::upsert("CHANNEL", &id, "PROJECT", &pid, "belongs_to", "{}"); } emit_for_project(&pid); Ok(json!({"success":true,"id":id,"name":name,"remark":remark})) }
#[tauri::command] pub async fn channel_update(data: Value) -> Result<Value, String> {
    let id = data["id"].as_str().ok_or("missing id")?;
    let mut fields: Vec<String> = Vec::new(); let mut values: Vec<Value> = Vec::new();
    if let Some(v) = data["name"].as_str() { fields.push("name=?".into()); values.push(json!(v)); }
    if let Some(v) = data["phone"].as_str() { fields.push("phone=?".into()); values.push(json!(v)); }
    if let Some(v) = data["company"].as_str() { fields.push("company=?".into()); values.push(json!(v)); }
    if let Some(v) = data["remark"].as_str() { fields.push("remark=?".into()); values.push(json!(v)); }
    if fields.is_empty() { return Ok(json!({"success":true})); }
    values.push(json!(id));
    database::run(&format!("UPDATE channel_partners SET {} WHERE id=?", fields.join(",")), &values).map_err(|e| e.to_string())?;
    emit_for_channel(id);
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn channel_delete(channel_id: String) -> Result<Value, String> { crate::services::business_relations::delete_by_source("CHANNEL", &channel_id); emit_for_channel(&channel_id); database::run("DELETE FROM channel_contacts WHERE channel_id=?",&[json!(&channel_id)]).ok(); database::run("DELETE FROM project_channel_mapping WHERE channel_id=?",&[json!(&channel_id)]).ok(); database::run("DELETE FROM channel_partners WHERE id=?",&[json!(channel_id)]).map_err(|e| e.to_string())?; Ok(json!({"success":true})) }
#[tauri::command] pub async fn channel_link_project(channel_id: String, project_id: String, contact_id: Option<String>) -> Result<Value, String> { database::run("INSERT OR IGNORE INTO project_channel_mapping(project_id,channel_id,selected_contact_id) VALUES(?1,?2,?3)", &[json!(&project_id), json!(&channel_id), json!(&contact_id)]).map_err(|e| e.to_string())?; crate::services::business_relations::upsert("CHANNEL", &channel_id, "PROJECT", &project_id, "belongs_to", "{}"); if let Some(ref cid) = contact_id { if !cid.is_empty() { crate::services::business_relations::upsert("CONTACT", cid, "PROJECT", &project_id, "belongs_to", "{}"); } } Ok(json!({"success":true})) }
#[tauri::command] pub async fn channel_unlink_project(channel_id: String, project_id: String) -> Result<Value, String> { let selected = database::get("SELECT selected_contact_id FROM project_channel_mapping WHERE project_id=?1 AND channel_id=?2", &[json!(&project_id), json!(&channel_id)]).ok().flatten().and_then(|r| r["selected_contact_id"].as_str().map(String::from)); database::run("DELETE FROM project_channel_mapping WHERE project_id=?1 AND channel_id=?2", &[json!(&project_id), json!(&channel_id)]).map_err(|e| e.to_string())?; crate::services::business_relations::delete("CHANNEL", &channel_id, "PROJECT", &project_id, "belongs_to"); if let Some(cid) = selected { if !cid.is_empty() { crate::services::business_relations::delete("CONTACT", &cid, "PROJECT", &project_id, "belongs_to"); } } Ok(json!({"success":true})) }
#[tauri::command] pub async fn channel_contact_create(data: Value) -> Result<Value, String> { let id = uuid::Uuid::new_v4().to_string(); let name = data["name"].as_str().unwrap_or("").to_string(); let phone = data["phone"].as_str().unwrap_or("").to_string(); let email = data["email"].as_str().unwrap_or("").to_string(); let position = data["position"].as_str().unwrap_or("").to_string(); let remark = data["remark"].as_str().unwrap_or("").to_string(); let cid = data["channelId"].as_str().unwrap_or("").to_string(); database::run("INSERT INTO channel_contacts(id,channel_id,name,phone,email,position,remark) VALUES(?,?,?,?,?,?,?)",&[json!(id),json!(&cid),json!(&name),json!(&phone),json!(&email),json!(&position),json!(&remark)]).map_err(|e|e.to_string())?; crate::services::source_doc_sync::sync_entity("CONTACT", &id); crate::services::business_relations::upsert("CONTACT", &id, "CHANNEL", &cid, "employed_by", "{}"); emit_for_channel(&cid); Ok(json!({"success":true,"id":id,"name":name,"phone":phone,"email":email,"position":position,"remark":remark})) }
#[tauri::command] pub async fn channel_contact_update(data: Value) -> Result<Value, String> {
    let id = data["id"].as_str().ok_or("missing id")?;
    let mut fields: Vec<String> = Vec::new(); let mut values: Vec<Value> = Vec::new();
    if let Some(v) = data["name"].as_str() { fields.push("name=?".into()); values.push(json!(v)); }
    if let Some(v) = data["phone"].as_str() { fields.push("phone=?".into()); values.push(json!(v)); }
    if let Some(v) = data["email"].as_str() { fields.push("email=?".into()); values.push(json!(v)); }
    if let Some(v) = data["position"].as_str() { fields.push("position=?".into()); values.push(json!(v)); }
    if let Some(v) = data["remark"].as_str() { fields.push("remark=?".into()); values.push(json!(v)); }
    if fields.is_empty() { return Ok(json!({"success":true})); }
    values.push(json!(id));
    database::run(&format!("UPDATE channel_contacts SET {} WHERE id=?", fields.join(",")), &values).map_err(|e| e.to_string())?;
    crate::services::source_doc_sync::sync_entity("CONTACT", id);
    emit_for_channel_contact(id);
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn channel_contact_delete(contact_id: String) -> Result<Value, String> { crate::services::source_doc_sync::delete_entity_source_doc("CONTACT", &contact_id); crate::services::business_relations::delete_by_source("CONTACT", &contact_id); emit_for_channel_contact(&contact_id); database::run("DELETE FROM channel_contacts WHERE id=?",&[json!(&contact_id)]).map_err(|e| e.to_string())?; Ok(json!({"success":true})) }
#[tauri::command] pub async fn vendor_list(project_id: String) -> Result<Vec<Value>, String> {
    let partners = database::all(
        "SELECT vp.*, pvm.selected_contact_id FROM vendor_partners vp JOIN project_vendor_mapping pvm ON vp.id = pvm.vendor_id WHERE pvm.project_id = ?",
        &[json!(&project_id)]
    ).map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    for p in &partners {
        let vid = p["id"].as_str().unwrap_or("");
        let selected_id = p["selected_contact_id"].as_str();
        // 只返回选中的联系人（如果有的话），否则返回所有联系人（兼容旧数据）
        let contacts = if let Some(sid) = selected_id {
            database::all("SELECT * FROM vendor_contacts WHERE id=?",&[json!(sid)]).unwrap_or_default()
        } else {
            database::all("SELECT * FROM vendor_contacts WHERE vendor_id=?",&[json!(vid)]).unwrap_or_default()
        };
        let mut obj = p.clone();
        obj["contacts"] = json!(contacts);
        result.push(obj);
    }
    Ok(result)
}
#[tauri::command] pub async fn vendor_create(data: Value) -> Result<Value, String> { let id = uuid::Uuid::new_v4().to_string(); let name = data["name"].as_str().unwrap_or("").to_string(); let pid = data["projectId"].as_str().unwrap_or("").to_string(); database::run("INSERT INTO vendor_partners(id,project_id,name,phone,company,remark) VALUES(?,?,?,?,?,?)",&[json!(id),json!(&pid),json!(&name),json!(data["phone"].as_str().unwrap_or("")),json!(data["company"].as_str().unwrap_or("")),json!(data["remark"].as_str().unwrap_or(""))]).map_err(|e| e.to_string())?; if !pid.is_empty() { crate::services::business_relations::upsert("VENDOR", &id, "PROJECT", &pid, "belongs_to", "{}"); } emit_for_project(&pid); Ok(json!({"success":true,"id":id,"name":name})) }
#[tauri::command] pub async fn vendor_update(data: Value) -> Result<Value, String> {
    let id = data["id"].as_str().ok_or("missing id")?;
    let mut fields: Vec<String> = Vec::new(); let mut values: Vec<Value> = Vec::new();
    if let Some(v) = data["name"].as_str() { fields.push("name=?".into()); values.push(json!(v)); }
    if let Some(v) = data["phone"].as_str() { fields.push("phone=?".into()); values.push(json!(v)); }
    if let Some(v) = data["company"].as_str() { fields.push("company=?".into()); values.push(json!(v)); }
    if let Some(v) = data["remark"].as_str() { fields.push("remark=?".into()); values.push(json!(v)); }
    if fields.is_empty() { return Ok(json!({"success":true})); }
    values.push(json!(id));
    database::run(&format!("UPDATE vendor_partners SET {} WHERE id=?", fields.join(",")), &values).map_err(|e| e.to_string())?;
    emit_for_vendor(id);
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn vendor_delete(vendor_id: String) -> Result<Value, String> { crate::services::business_relations::delete_by_source("VENDOR", &vendor_id); emit_for_vendor(&vendor_id); database::run("DELETE FROM vendor_contacts WHERE vendor_id=?",&[json!(&vendor_id)]).ok(); database::run("DELETE FROM project_vendor_mapping WHERE vendor_id=?",&[json!(&vendor_id)]).ok(); database::run("DELETE FROM vendor_partners WHERE id=?",&[json!(vendor_id)]).map_err(|e| e.to_string())?; Ok(json!({"success":true})) }
#[tauri::command] pub async fn vendor_link_project(vendor_id: String, project_id: String, contact_id: Option<String>) -> Result<Value, String> { database::run("INSERT OR IGNORE INTO project_vendor_mapping(project_id,vendor_id,selected_contact_id) VALUES(?1,?2,?3)", &[json!(&project_id), json!(&vendor_id), json!(&contact_id)]).map_err(|e| e.to_string())?; crate::services::business_relations::upsert("VENDOR", &vendor_id, "PROJECT", &project_id, "belongs_to", "{}"); if let Some(ref cid) = contact_id { if !cid.is_empty() { crate::services::business_relations::upsert("CONTACT", cid, "PROJECT", &project_id, "belongs_to", "{}"); } } Ok(json!({"success":true})) }
#[tauri::command] pub async fn vendor_unlink_project(vendor_id: String, project_id: String) -> Result<Value, String> { let selected = database::get("SELECT selected_contact_id FROM project_vendor_mapping WHERE project_id=?1 AND vendor_id=?2", &[json!(&project_id), json!(&vendor_id)]).ok().flatten().and_then(|r| r["selected_contact_id"].as_str().map(String::from)); database::run("DELETE FROM project_vendor_mapping WHERE project_id=?1 AND vendor_id=?2", &[json!(&project_id), json!(&vendor_id)]).map_err(|e| e.to_string())?; crate::services::business_relations::delete("VENDOR", &vendor_id, "PROJECT", &project_id, "belongs_to"); if let Some(cid) = selected { if !cid.is_empty() { crate::services::business_relations::delete("CONTACT", &cid, "PROJECT", &project_id, "belongs_to"); } } Ok(json!({"success":true})) }
#[tauri::command] pub async fn vendor_contact_create(data: Value) -> Result<Value, String> { let id = uuid::Uuid::new_v4().to_string(); let name = data["name"].as_str().unwrap_or("").to_string(); let phone = data["phone"].as_str().unwrap_or("").to_string(); let email = data["email"].as_str().unwrap_or("").to_string(); let position = data["position"].as_str().unwrap_or("").to_string(); let remark = data["remark"].as_str().unwrap_or("").to_string(); let vid = data["vendorId"].as_str().unwrap_or("").to_string(); database::run("INSERT INTO vendor_contacts(id,vendor_id,name,phone,email,position,remark) VALUES(?,?,?,?,?,?,?)",&[json!(id),json!(&vid),json!(&name),json!(&phone),json!(&email),json!(&position),json!(&remark)]).map_err(|e|e.to_string())?; crate::services::source_doc_sync::sync_entity("CONTACT", &id); crate::services::business_relations::upsert("CONTACT", &id, "VENDOR", &vid, "employed_by", "{}"); emit_for_vendor(&vid); Ok(json!({"success":true,"id":id,"name":name,"phone":phone,"email":email,"position":position,"remark":remark})) }
#[tauri::command] pub async fn vendor_contact_update(data: Value) -> Result<Value, String> {
    let id = data["id"].as_str().ok_or("missing id")?;
    let mut fields: Vec<String> = Vec::new(); let mut values: Vec<Value> = Vec::new();
    if let Some(v) = data["name"].as_str() { fields.push("name=?".into()); values.push(json!(v)); }
    if let Some(v) = data["phone"].as_str() { fields.push("phone=?".into()); values.push(json!(v)); }
    if let Some(v) = data["email"].as_str() { fields.push("email=?".into()); values.push(json!(v)); }
    if let Some(v) = data["position"].as_str() { fields.push("position=?".into()); values.push(json!(v)); }
    if let Some(v) = data["remark"].as_str() { fields.push("remark=?".into()); values.push(json!(v)); }
    if fields.is_empty() { return Ok(json!({"success":true})); }
    values.push(json!(id));
    database::run(&format!("UPDATE vendor_contacts SET {} WHERE id=?", fields.join(",")), &values).map_err(|e| e.to_string())?;
    crate::services::source_doc_sync::sync_entity("CONTACT", id);
    emit_for_vendor_contact(id);
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn vendor_contact_delete(contact_id: String) -> Result<Value, String> { crate::services::source_doc_sync::delete_entity_source_doc("CONTACT", &contact_id); crate::services::business_relations::delete_by_source("CONTACT", &contact_id); database::run("DELETE FROM vendor_contacts WHERE id=?",&[json!(&contact_id)]).map_err(|e| e.to_string())?; emit_for_vendor_contact(&contact_id); Ok(json!({"success":true})) }

// ── Event Bus helpers ──
fn emit_for_project(pid: &str) { crate::services::object_changed::object_changed(crate::services::object_changed::ObjectType::Project, pid, crate::services::object_changed::Action::Updated); }
fn emit_for_channel(cid: &str) { if let Ok(Some(r)) = database::get("SELECT project_id FROM channel_partners WHERE id=?", &[json!(cid)]) { if let Some(pid) = r["project_id"].as_str() { emit_for_project(pid); } } }
fn emit_for_vendor(vid: &str) { if let Ok(Some(r)) = database::get("SELECT project_id FROM vendor_partners WHERE id=?", &[json!(vid)]) { if let Some(pid) = r["project_id"].as_str() { emit_for_project(pid); } } }
fn emit_for_channel_contact(ccid: &str) { if let Ok(Some(r)) = database::get("SELECT cp.project_id FROM channel_contacts cc JOIN channel_partners cp ON cp.id=cc.channel_id WHERE cc.id=?", &[json!(ccid)]) { if let Some(pid) = r["project_id"].as_str() { emit_for_project(pid); } } }
fn emit_for_vendor_contact(vcid: &str) { if let Ok(Some(r)) = database::get("SELECT vp.project_id FROM vendor_contacts vc JOIN vendor_partners vp ON vp.id=vc.vendor_id WHERE vc.id=?", &[json!(vcid)]) { if let Some(pid) = r["project_id"].as_str() { emit_for_project(pid); } } }

// ── Customers CRUD ──
#[tauri::command] pub async fn customer_list() -> Result<Vec<Value>, String> {
    let customers = database::all("SELECT c.*, GROUP_CONCAT(p.name, ', ') as project_names FROM customers c LEFT JOIN project_customer_mapping pcm ON c.id=pcm.customer_id LEFT JOIN projects p ON p.id=pcm.project_id GROUP BY c.id ORDER BY c.updated_at DESC", &[]).map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    for c in &customers {
        let cid = c["id"].as_str().unwrap_or("");
        let contacts = database::all("SELECT * FROM customer_contacts WHERE customer_id=?",&[json!(cid)]).unwrap_or_default();
        let mut obj = c.clone();
        obj["contacts"] = json!(contacts);
        result.push(obj);
    }
    Ok(result)
}
#[tauri::command] pub async fn customer_create(data: Value) -> Result<Value, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let name = data["name"].as_str().unwrap_or("").to_string();
    let contact = data["contact_person"].as_str().unwrap_or("").to_string();
    let phone = data["contact_phone"].as_str().unwrap_or("").to_string();
    let remark = data["remark"].as_str().unwrap_or("").to_string();
    database::run("INSERT INTO customers(id,name,contact_person,contact_phone,remark) VALUES(?1,?2,?3,?4,?5)", &[json!(&id), json!(&name), json!(&contact), json!(&phone), json!(&remark)]).map_err(|e| e.to_string())?;
    Ok(json!({"success":true,"id":id,"name":name}))
}
#[tauri::command] pub async fn customer_update(data: Value) -> Result<Value, String> {
    let id = data["id"].as_str().ok_or("missing id")?;
    let mut fields: Vec<String> = Vec::new(); let mut values: Vec<Value> = Vec::new();
    if let Some(v) = data["name"].as_str() { fields.push("name=?".into()); values.push(json!(v)); }
    if let Some(v) = data["contact_person"].as_str() { fields.push("contact_person=?".into()); values.push(json!(v)); }
    if let Some(v) = data["contact_phone"].as_str() { fields.push("contact_phone=?".into()); values.push(json!(v)); }
    if let Some(v) = data["remark"].as_str() { fields.push("remark=?".into()); values.push(json!(v)); }
    if fields.is_empty() { return Ok(json!({"success":true})); }
    values.push(json!(id));
    database::run(&format!("UPDATE customers SET {} WHERE id=?", fields.join(",")), &values).map_err(|e| e.to_string())?;
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn customer_delete(customer_id: String) -> Result<Value, String> { crate::services::business_relations::delete_by_source("CUSTOMER", &customer_id); database::run("DELETE FROM customer_contacts WHERE customer_id=?", &[json!(&customer_id)]).ok(); database::run("DELETE FROM project_customer_mapping WHERE customer_id=?1", &[json!(&customer_id)]).ok(); database::run("DELETE FROM customers WHERE id=?1", &[json!(&customer_id)]).map_err(|e| e.to_string())?; Ok(json!({"success":true})) }
#[tauri::command] pub async fn customer_contact_create(data: Value) -> Result<Value, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let name = data["name"].as_str().unwrap_or("").to_string();
    let phone = data["phone"].as_str().unwrap_or("").to_string();
    let email = data["email"].as_str().unwrap_or("").to_string();
    let position = data["position"].as_str().unwrap_or("").to_string();
    let remark = data["remark"].as_str().unwrap_or("").to_string();
    let cid = data["customerId"].as_str().unwrap_or("").to_string();
    database::run("INSERT INTO customer_contacts(id,customer_id,name,phone,email,position,remark) VALUES(?,?,?,?,?,?,?)",&[json!(id),json!(&cid),json!(&name),json!(&phone),json!(&email),json!(&position),json!(&remark)]).map_err(|e|e.to_string())?;
    crate::services::source_doc_sync::sync_entity("CONTACT", &id);
    crate::services::business_relations::upsert("CONTACT", &id, "CUSTOMER", &cid, "employed_by", "{}");
    Ok(json!({"success":true,"id":id,"name":name,"phone":phone,"email":email,"position":position,"remark":remark}))
}
#[tauri::command] pub async fn customer_contact_update(data: Value) -> Result<Value, String> {
    let id = data["id"].as_str().ok_or("missing id")?;
    let mut fields: Vec<String> = Vec::new(); let mut values: Vec<Value> = Vec::new();
    if let Some(v) = data["name"].as_str() { fields.push("name=?".into()); values.push(json!(v)); }
    if let Some(v) = data["phone"].as_str() { fields.push("phone=?".into()); values.push(json!(v)); }
    if let Some(v) = data["email"].as_str() { fields.push("email=?".into()); values.push(json!(v)); }
    if let Some(v) = data["position"].as_str() { fields.push("position=?".into()); values.push(json!(v)); }
    if let Some(v) = data["remark"].as_str() { fields.push("remark=?".into()); values.push(json!(v)); }
    if fields.is_empty() { return Ok(json!({"success":true})); }
    values.push(json!(id));
    database::run(&format!("UPDATE customer_contacts SET {} WHERE id=?", fields.join(",")), &values).map_err(|e| e.to_string())?;
    crate::services::source_doc_sync::sync_entity("CONTACT", id);
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn customer_contact_delete(contact_id: String) -> Result<Value, String> {
    crate::services::source_doc_sync::delete_entity_source_doc("CONTACT", &contact_id);
    crate::services::business_relations::delete_by_source("CONTACT", &contact_id);
    database::run("DELETE FROM customer_contacts WHERE id=?",&[json!(&contact_id)]).map_err(|e| e.to_string())?;
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn customer_link_project(customer_id: String, project_id: String) -> Result<Value, String> {
    database::run("INSERT OR IGNORE INTO project_customer_mapping(project_id,customer_id) VALUES(?1,?2)", &[json!(&project_id), json!(&customer_id)]).map_err(|e| e.to_string())?;
    crate::services::business_relations::upsert("CUSTOMER", &customer_id, "PROJECT", &project_id, "belongs_to", "{}");
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn customer_unlink_project(customer_id: String, project_id: String) -> Result<Value, String> {
    database::run("DELETE FROM project_customer_mapping WHERE project_id=?1 AND customer_id=?2", &[json!(&project_id), json!(&customer_id)]).map_err(|e| e.to_string())?;
    crate::services::business_relations::delete("CUSTOMER", &customer_id, "PROJECT", &project_id, "belongs_to");
    Ok(json!({"success":true}))
}

// ── 全局列表（通讯录用，不过滤 project_id）──
#[tauri::command] pub async fn contacts_list_channels() -> Result<Vec<Value>, String> {
    let partners = database::all("SELECT * FROM channel_partners ORDER BY name", &[]).map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    for p in &partners {
        let cid = p["id"].as_str().unwrap_or("");
        let contacts = database::all("SELECT * FROM channel_contacts WHERE channel_id=?",&[json!(cid)]).unwrap_or_default();
        let mut obj = p.clone();
        obj["contacts"] = json!(contacts);
        result.push(obj);
    }
    Ok(result)
}
#[tauri::command] pub async fn contacts_list_vendors() -> Result<Vec<Value>, String> {
    let partners = database::all("SELECT * FROM vendor_partners ORDER BY name", &[]).map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    for p in &partners {
        let vid = p["id"].as_str().unwrap_or("");
        let contacts = database::all("SELECT * FROM vendor_contacts WHERE vendor_id=?",&[json!(vid)]).unwrap_or_default();
        let mut obj = p.clone();
        obj["contacts"] = json!(contacts);
        result.push(obj);
    }
    Ok(result)
}
