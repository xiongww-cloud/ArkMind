use serde_json::{json, Value}; use crate::database;
#[tauri::command] pub async fn channel_list(project_id: String) -> Result<Vec<Value>, String> {
    let partners = database::all("SELECT * FROM channel_partners WHERE project_id=?",&[json!(&project_id)]).map_err(|e| e.to_string())?;
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
#[tauri::command] pub async fn channel_create(data: Value) -> Result<Value, String> { let id = uuid::Uuid::new_v4().to_string(); let name = data["name"].as_str().unwrap_or("").to_string(); let remark = data["remark"].as_str().unwrap_or("").to_string(); let pid = data["projectId"].as_str().unwrap_or("").to_string(); database::run("INSERT INTO channel_partners(id,project_id,name,phone,company,remark) VALUES(?,?,?,?,?,?)",&[json!(id),json!(&pid),json!(&name),json!(data["phone"].as_str().unwrap_or("")),json!(data["company"].as_str().unwrap_or("")),json!(&remark)]).map_err(|e| e.to_string())?; emit_for_project(&pid); Ok(json!({"success":true,"id":id,"name":name,"remark":remark})) }
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
#[tauri::command] pub async fn channel_delete(channel_id: String) -> Result<Value, String> { emit_for_channel(&channel_id); database::run("DELETE FROM channel_contacts WHERE channel_id=?",&[json!(&channel_id)]).ok(); database::run("DELETE FROM channel_partners WHERE id=?",&[json!(channel_id)]).map_err(|e| e.to_string())?; Ok(json!({"success":true})) }
#[tauri::command] pub async fn channel_contact_create(data: Value) -> Result<Value, String> { let id = uuid::Uuid::new_v4().to_string(); let name = data["name"].as_str().unwrap_or("").to_string(); let phone = data["phone"].as_str().unwrap_or("").to_string(); let remark = data["remark"].as_str().unwrap_or(data["email"].as_str().unwrap_or("")).to_string(); let cid = data["channelId"].as_str().unwrap_or("").to_string(); database::run("INSERT INTO channel_contacts(id,channel_id,name,phone,remark) VALUES(?,?,?,?,?)",&[json!(id),json!(&cid),json!(&name),json!(&phone),json!(&remark)]).map_err(|e|e.to_string())?; emit_for_channel(&cid); Ok(json!({"success":true,"id":id,"name":name,"phone":phone,"remark":remark})) }
#[tauri::command] pub async fn channel_contact_update(data: Value) -> Result<Value, String> {
    let id = data["id"].as_str().ok_or("missing id")?;
    let mut fields: Vec<String> = Vec::new(); let mut values: Vec<Value> = Vec::new();
    if let Some(v) = data["name"].as_str() { fields.push("name=?".into()); values.push(json!(v)); }
    if let Some(v) = data["phone"].as_str() { fields.push("phone=?".into()); values.push(json!(v)); }
    if let Some(v) = data["email"].as_str() { fields.push("email=?".into()); values.push(json!(v)); }
    if let Some(v) = data["remark"].as_str() { fields.push("remark=?".into()); values.push(json!(v)); }
    if fields.is_empty() { return Ok(json!({"success":true})); }
    values.push(json!(id));
    database::run(&format!("UPDATE channel_contacts SET {} WHERE id=?", fields.join(",")), &values).map_err(|e| e.to_string())?;
    emit_for_channel_contact(id);
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn channel_contact_delete(contact_id: String) -> Result<Value, String> { emit_for_channel_contact(&contact_id); database::run("DELETE FROM channel_contacts WHERE id=?",&[json!(&contact_id)]).map_err(|e| e.to_string())?; Ok(json!({"success":true})) }
#[tauri::command] pub async fn vendor_list(project_id: String) -> Result<Vec<Value>, String> {
    let partners = database::all("SELECT * FROM vendor_partners WHERE project_id=?",&[json!(&project_id)]).map_err(|e| e.to_string())?;
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
#[tauri::command] pub async fn vendor_create(data: Value) -> Result<Value, String> { let id = uuid::Uuid::new_v4().to_string(); let name = data["name"].as_str().unwrap_or("").to_string(); let pid = data["projectId"].as_str().unwrap_or("").to_string(); database::run("INSERT INTO vendor_partners(id,project_id,name,phone,company,remark) VALUES(?,?,?,?,?,?)",&[json!(id),json!(&pid),json!(&name),json!(data["phone"].as_str().unwrap_or("")),json!(data["company"].as_str().unwrap_or("")),json!(data["remark"].as_str().unwrap_or(""))]).map_err(|e| e.to_string())?; emit_for_project(&pid); Ok(json!({"success":true,"id":id,"name":name})) }
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
#[tauri::command] pub async fn vendor_delete(vendor_id: String) -> Result<Value, String> { emit_for_vendor(&vendor_id); database::run("DELETE FROM vendor_contacts WHERE vendor_id=?",&[json!(&vendor_id)]).ok(); database::run("DELETE FROM vendor_partners WHERE id=?",&[json!(vendor_id)]).map_err(|e| e.to_string())?; Ok(json!({"success":true})) }
#[tauri::command] pub async fn vendor_contact_create(data: Value) -> Result<Value, String> { let id = uuid::Uuid::new_v4().to_string(); let name = data["name"].as_str().unwrap_or("").to_string(); let phone = data["phone"].as_str().unwrap_or("").to_string(); let remark = data["remark"].as_str().unwrap_or(data["email"].as_str().unwrap_or("")).to_string(); let vid = data["vendorId"].as_str().unwrap_or("").to_string(); database::run("INSERT INTO vendor_contacts(id,vendor_id,name,phone,remark) VALUES(?,?,?,?,?)",&[json!(id),json!(&vid),json!(&name),json!(&phone),json!(&remark)]).map_err(|e|e.to_string())?; emit_for_vendor(&vid); Ok(json!({"success":true,"id":id,"name":name,"phone":phone,"remark":remark})) }
#[tauri::command] pub async fn vendor_contact_update(data: Value) -> Result<Value, String> {
    let id = data["id"].as_str().ok_or("missing id")?;
    let mut fields: Vec<String> = Vec::new(); let mut values: Vec<Value> = Vec::new();
    if let Some(v) = data["name"].as_str() { fields.push("name=?".into()); values.push(json!(v)); }
    if let Some(v) = data["phone"].as_str() { fields.push("phone=?".into()); values.push(json!(v)); }
    if let Some(v) = data["email"].as_str() { fields.push("email=?".into()); values.push(json!(v)); }
    if let Some(v) = data["remark"].as_str() { fields.push("remark=?".into()); values.push(json!(v)); }
    if fields.is_empty() { return Ok(json!({"success":true})); }
    values.push(json!(id));
    database::run(&format!("UPDATE vendor_contacts SET {} WHERE id=?", fields.join(",")), &values).map_err(|e| e.to_string())?;
    emit_for_vendor_contact(id);
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn vendor_contact_delete(contact_id: String) -> Result<Value, String> { database::run("DELETE FROM vendor_contacts WHERE id=?",&[json!(&contact_id)]).map_err(|e| e.to_string())?; emit_for_vendor_contact(&contact_id); Ok(json!({"success":true})) }

// ── Event Bus helpers ──
fn emit_for_project(pid: &str) { crate::services::object_changed::object_changed(crate::services::object_changed::ObjectType::Project, pid, crate::services::object_changed::Action::Updated); }
fn emit_for_channel(cid: &str) { if let Ok(Some(r)) = database::get("SELECT project_id FROM channel_partners WHERE id=?", &[json!(cid)]) { if let Some(pid) = r["project_id"].as_str() { emit_for_project(pid); } } }
fn emit_for_vendor(vid: &str) { if let Ok(Some(r)) = database::get("SELECT project_id FROM vendor_partners WHERE id=?", &[json!(vid)]) { if let Some(pid) = r["project_id"].as_str() { emit_for_project(pid); } } }
fn emit_for_channel_contact(ccid: &str) { if let Ok(Some(r)) = database::get("SELECT cp.project_id FROM channel_contacts cc JOIN channel_partners cp ON cp.id=cc.channel_id WHERE cc.id=?", &[json!(ccid)]) { if let Some(pid) = r["project_id"].as_str() { emit_for_project(pid); } } }
fn emit_for_vendor_contact(vcid: &str) { if let Ok(Some(r)) = database::get("SELECT vp.project_id FROM vendor_contacts vc JOIN vendor_partners vp ON vp.id=vc.vendor_id WHERE vc.id=?", &[json!(vcid)]) { if let Some(pid) = r["project_id"].as_str() { emit_for_project(pid); } } }
