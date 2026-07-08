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
            let mut status = r["status"].as_str().unwrap_or("inactive").to_string();
            let product = r["product"].as_str().unwrap_or("");
            let expires_at = r["expires_at"].as_str().unwrap_or("");

            // ★ 如果 status='active' 但 expires_at 已过期，强制返回 'expired'
            if status == "active" && !expires_at.is_empty() && expires_at != "2099-12-31" {
                if let Ok(expiry) = chrono::NaiveDate::parse_from_str(expires_at, "%Y-%m-%d") {
                    let today = chrono::Local::now().date_naive();
                    if expiry < today {
                        status = "expired".to_string();
                        // 同步更新数据库状态
                        let _ = database::run(
                            "UPDATE license SET status='inactive' WHERE status='active' AND expires_at<?1",
                            &[json!(expires_at)],
                        );
                    }
                }
            }

            let is_active = status == "active";
            Ok(json!({
                "activated": is_active,
                "status": status,
                "product": product,
                "activatedAt": r["activated_at"].as_str().unwrap_or(""),
                "expiresAt": expires_at,
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
