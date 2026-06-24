use crate::{database, license};
use serde_json::{json, Value};

#[tauri::command]
pub fn license_status() -> Result<Value, String> {
    let row = database::get(
        "SELECT status, product, activated_at, expires_at, machine_code \
         FROM license ORDER BY id DESC LIMIT 1",
        &[],
    )
    .map_err(|e| e.to_string())?;

    match row {
        Some(r) => {
            let is_active = r["status"].as_str() == Some("active");
            let product = r["product"].as_str().unwrap_or("");
            Ok(json!({
                "activated": is_active,
                "status": r["status"].as_str().unwrap_or("inactive"),
                "product": product,
                "activatedAt": r["activated_at"].as_str().unwrap_or(""),
                "expiresAt": r["expires_at"].as_str().unwrap_or(""),
                "machineCode": r["machine_code"].as_str().unwrap_or(""),
            }))
        }
        None => Ok(json!({
            "activated": false,
            "status": "inactive"
        })),
    }
}

#[tauri::command]
pub fn machine_code() -> Result<String, String> {
    Ok(license::get_machine_code())
}

#[tauri::command]
pub fn license_activate(key: String) -> Result<Value, String> {
    match license::activate_license(&key) {
        Ok((product, expires_at)) => {
            Ok(json!({
                "success": true,
                "product": product,
                "expiresAt": expires_at
            }))
        }
        Err(e) => Ok(json!({
            "success": false,
            "error": e
        })),
    }
}
