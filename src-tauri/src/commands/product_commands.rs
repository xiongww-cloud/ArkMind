use crate::database;
use serde_json::{json, Value};

#[tauri::command] pub async fn product_series_list() -> Result<Vec<Value>, String> { database::all("SELECT * FROM product_series ORDER BY sort_order ASC, name ASC", &[]).map_err(|e| e.to_string()) }
#[tauri::command] pub async fn product_series_create(data: Value) -> Result<Value, String> { let id = uuid::Uuid::new_v4().to_string(); let inner = data.get("data").unwrap_or(&data); let name = inner["name"].as_str().unwrap_or(""); let parent_id = inner["parentId"].as_str().or_else(|| inner["parent_id"].as_str()).map(|s| json!(s)); database::run("INSERT INTO product_series(id,name,parent_id) VALUES(?,?,?)",&[json!(id),json!(name),parent_id.unwrap_or(Value::Null)]).map_err(|e| e.to_string())?; database::get("SELECT * FROM product_series WHERE id=?",&[json!(id)]).map_err(|e| e.to_string()).map(|r| { r.unwrap_or(Value::Null) }) }
#[tauri::command] pub async fn product_series_update(id: String, data: Value) -> Result<Value, String> { if let Some(n) = data["name"].as_str() { database::run("UPDATE product_series SET name=?,updated_at=datetime('now', 'localtime') WHERE id=?",&[json!(n),json!(id)]).map_err(|e| e.to_string())?; } if let Some(ref pv) = data.get("parentId").or(data.get("parent_id")) { if pv.is_null() || pv.as_str() == Some("") { database::run("UPDATE product_series SET parent_id=NULL WHERE id=?",&[json!(&id)]).map_err(|e| e.to_string())?; } else if let Some(pid) = pv.as_str() { database::run("UPDATE product_series SET parent_id=? WHERE id=?",&[json!(pid),json!(id)]).map_err(|e| e.to_string())?; } } if let Some(ref so) = data.get("sortOrder").or(data.get("sort_order")) { if let Some(so_num) = so.as_i64() { database::run("UPDATE product_series SET sort_order=?,updated_at=datetime('now', 'localtime') WHERE id=?",&[json!(so_num),json!(id)]).map_err(|e| e.to_string())?; } } Ok(json!({"success":true})) }
#[tauri::command] pub async fn product_series_delete(id: String) -> Result<Value, String> {
    let subs: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM product_series WHERE parent_id=?", rusqlite::params![&id], |r| r.get(0))).unwrap_or(0);
    let prods: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM products WHERE series_id=?", rusqlite::params![&id], |r| r.get(0))).unwrap_or(0);
    if subs > 0 || prods > 0 { return Ok(json!({"success":false,"error":"目录不为空，请先移动所有产品和子目录"})); }
    database::run("DELETE FROM product_series WHERE id=?",&[json!(&id)]).map_err(|e| e.to_string())?;
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn product_series_update_sort_order(data: Vec<Value>) -> Result<Value, String> {
    for item in &data {
        let id = item["id"].as_str().unwrap_or("");
        let so = item["sortOrder"].as_i64().unwrap_or(0);
        if !id.is_empty() { database::run("UPDATE product_series SET sort_order=? WHERE id=?",&[json!(so),json!(id)]).ok(); }
    }
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn product_list(series_id: Option<String>) -> Result<Vec<Value>, String> { if let Some(sid) = series_id { database::all("SELECT * FROM products WHERE series_id=? ORDER BY sort_order ASC, name ASC",&[json!(sid)]).map_err(|e| e.to_string()) } else { database::all("SELECT * FROM products ORDER BY sort_order ASC, name ASC",&[]).map_err(|e| e.to_string()) } }
#[tauri::command]
pub async fn product_create(data: Value) -> Result<Value, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let name = data["name"].as_str().unwrap_or("新产品").to_string();
    let desc = data["description"].as_str().unwrap_or("").to_string();
    let specs = data["fullSpecs"].as_str().unwrap_or("").to_string();
    let series_id = data["seriesId"].as_str().map(|s| json!(s));
    database::run("INSERT INTO products(id,name,series_id,description,price,full_specs) VALUES(?,?,?,?,?,?)",
    &[json!(id),json!(&name),series_id.unwrap_or(Value::Null),json!(&desc),json!(data["price"].as_f64().unwrap_or(0.0)),json!(&specs)]).map_err(|e| e.to_string())?;
    crate::services::knowledge_lifecycle::notify_changed("PRODUCT", &id);
    database::get("SELECT * FROM products WHERE id=?",&[json!(id)]).map_err(|e| e.to_string()).map(|r| r.unwrap_or(Value::Null))
}
#[tauri::command] pub async fn product_update(id: String, data: Value) -> Result<(), String> {
    let mut fields: Vec<String>=Vec::new(); let mut values: Vec<Value>=Vec::new();
    if let Some(v)=data["name"].as_str(){fields.push("name=?".into());values.push(json!(v));}
    if data.get("seriesId").is_some() {
        let v = data["seriesId"].as_str().map(|s| json!(s)).unwrap_or(json!(null));
        fields.push("series_id=?".into()); values.push(v);
    }
    if let Some(v)=data["description"].as_str(){fields.push("description=?".into());values.push(json!(v));}
    if let Some(v)=data["fullSpecs"].as_str(){fields.push("full_specs=?".into());values.push(json!(v));}
    if fields.is_empty(){return Ok(());}
    fields.push("updated_at=datetime('now', 'localtime')".into());
    values.push(json!(id));
    database::run(&format!("UPDATE products SET {} WHERE id=?",fields.join(",")),&values).map_err(|e|e.to_string())?;
    crate::services::knowledge_lifecycle::notify_changed("PRODUCT", &id);
    Ok(())
}
#[tauri::command] pub async fn product_update_sort_orders(data: Vec<Value>) -> Result<(), String> {
    for item in &data {
        let id = item["id"].as_str().unwrap_or("");
        let so = item["sortOrder"].as_i64().unwrap_or(0);
        if !id.is_empty() { database::run("UPDATE products SET sort_order=? WHERE id=?",&[json!(so),json!(id)]).ok(); }
    }
    Ok(())
}
#[tauri::command] pub async fn product_delete(id: String) -> Result<Value, String> {
    let product_assets = crate::commands::file_commands::collect_entity_asset_ids("product", std::slice::from_ref(&id));
    database::run("DELETE FROM file_relations WHERE entity_type='product' AND entity_id=?",&[json!(&id)]).map_err(|e| e.to_string())?;
    crate::services::business_relations::delete_by_source("PRODUCT", &id);
    // M2: 清理目标边（EQUIPMENT_ITEM-instance_of、PROJECT-belongs_to 等以 PRODUCT 为目标的关联）
    crate::services::business_relations::delete_by_target("PRODUCT", &id);
    database::run("DELETE FROM products WHERE id=?",&[json!(&id)]).map_err(|e| e.to_string())?;
    for aid in &product_assets { crate::commands::file_commands::release_asset_if_orphan(aid); }
    crate::services::knowledge_lifecycle::notify_deleted("PRODUCT", &id);
    database::save().map_err(|e| e.to_string())?;
    Ok(json!({"success":true}))
}
