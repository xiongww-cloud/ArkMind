//! License validation — Ed25519 signature + machine code binding + expiry check
//!
//! License key format: Arkmind-{base64_json_payload}.{base64_signature}
//!
//! Premium features: FTS5, AI sync/tag, KnowledgePipeline, Agent RAG, LanceDB
//! Basic features: Core CRUD, manual tags, solution BOM, file management

use base64::Engine;
use ed25519_dalek::{Verifier, VerifyingKey, Signature};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const PUBLIC_KEY_B64: &str = "KnOxev79d1jGqD+8puSDci84W1P0tXBtNKYYtQRrrKM=";

#[derive(Debug, Deserialize, Serialize)]
struct LicensePayload {
    product: String,
    machine_code: String,
    expires_at: String,
    issued_at: String,
}

pub fn get_machine_hash() -> String {
    let id = get_hardware_id();
    let mut h = Sha256::new(); h.update(id.as_bytes());
    hex::encode(&h.finalize()[..8])
}

#[cfg(target_os = "windows")]
fn get_hardware_id() -> String {
    use std::process::Command;
    // Try motherboard UUID first (most stable across Windows reinstalls)
    if let Ok(output) = Command::new("wmic").args(["csproduct", "get", "uuid"]).output() {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let t = line.trim();
            if !t.is_empty() && !t.eq_ignore_ascii_case("UUID") && t.len() > 8 {
                return t.to_string();
            }
        }
    }
    // Fallback: hostname
    hostname::get().unwrap_or_default().to_string_lossy().to_string()
}

#[cfg(target_os = "macos")]
fn get_hardware_id() -> String {
    std::process::Command::new("ioreg").args(["-rd1","-c","IOPlatformExpertDevice"]).output()
        .ok().and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout); s.lines().find_map(|l| {
                let t = l.trim(); if t.contains("IOPlatformUUID") { t.split('"').nth(3).map(|s| s.to_string()) } else { None }
            })
        }).unwrap_or_else(|| hostname::get().unwrap_or_default().to_string_lossy().to_string())
}

#[cfg(target_os = "linux")]
fn get_hardware_id() -> String {
    std::fs::read_to_string("/etc/machine-id").unwrap_or_else(|_| hostname::get().unwrap_or_default().to_string_lossy().to_string())
}

fn get_public_key() -> Result<VerifyingKey, String> {
    let vk_bytes = base64::engine::general_purpose::STANDARD.decode(PUBLIC_KEY_B64).map_err(|e| format!("pk decode: {}", e))?;
    let vk_arr: [u8; 32] = vk_bytes.try_into().map_err(|_| "pk must be 32 bytes".to_string())?;
    VerifyingKey::from_bytes(&vk_arr).map_err(|e| format!("pk parse: {}", e))
}

/// 应用启动时扫描并更新过期许可证状态
pub fn cleanup_expired_licenses() {
    let today = chrono::Local::now().date_naive().format("%Y-%m-%d").to_string();
    match crate::database::run(
        "UPDATE license SET status='inactive' WHERE status='active' AND expires_at IS NOT NULL AND expires_at != '2099-12-31' AND expires_at < ?1",
        &[serde_json::json!(today)],
    ) {
        Ok(r) if r.changes > 0 => {
            eprintln!("[许可证] 已清理 {} 条过期许可证", r.changes);
        }
        _ => {}
    }
}

pub fn get_machine_code() -> String { get_hardware_id() }

pub fn activate_license(key: &str) -> Result<(String, String), String> {
    let (product, expires_at) = verify_license_key_and_get_expiry(key)?;
    let machine = get_machine_hash();
    let expires_value = if expires_at.is_empty() { serde_json::json!("2099-12-31") } else { serde_json::json!(expires_at) };
    crate::database::run("UPDATE license SET status='inactive' WHERE status='active'", &[]).ok();
    crate::database::run(
        "INSERT INTO license (key_hash,license_key,status,product,activated_at,expires_at,machine_code) \
         VALUES(?,?,'active',?,datetime('now', 'localtime'),?,?)",
        &[
            serde_json::json!(hex::encode(Sha256::digest(key.as_bytes()))),
            serde_json::json!(key),
            serde_json::json!(product),
            expires_value,
            serde_json::json!(machine),
        ],
    ).map_err(|e| format!("db: {}", e))?;
    crate::database::save().map_err(|e| e.to_string())?;
    // 激活后启动知识管线
    crate::services::knowledge_pipeline::start();
    Ok((product, expires_at))
}

fn verify_license_key_and_get_expiry(key: &str) -> Result<(String, String), String> {
    let stripped = key.strip_prefix("Arkmind-").ok_or("缺少 Arkmind- 前缀")?;
    let dot_pos = stripped.rfind('.').ok_or("缺少签名分隔符")?;
    let payload_b64 = &stripped[..dot_pos];
    let sig_b64 = &stripped[dot_pos + 1..];

    let payload_bytes = base64::engine::general_purpose::STANDARD.decode(payload_b64).map_err(|e| format!("payload decode: {}", e))?;
    let payload_json = String::from_utf8(payload_bytes).map_err(|_| "invalid utf8".to_string())?;
    let payload: LicensePayload = serde_json::from_str(&payload_json).map_err(|e| format!("json parse: {}", e))?;

    let public_key = get_public_key()?;
    let message = format!("Arkmind:{}", payload_b64);
    let sig_bytes = base64::engine::general_purpose::STANDARD.decode(sig_b64).map_err(|e| format!("sig decode: {}", e))?;
    let sig_arr: [u8; 64] = sig_bytes.try_into().map_err(|_| "sig must be 64 bytes".to_string())?;
    public_key.verify(message.as_bytes(), &Signature::from_bytes(&sig_arr)).map_err(|_| "签名验证失败".to_string())?;

    let current = get_machine_hash();
    eprintln!("[许可] 期望={} 实际={}", payload.machine_code, current);
    if payload.machine_code != current { return Err(format!("机器码不匹配：{} vs {}", payload.machine_code, current)); }

    if !payload.expires_at.is_empty() && payload.expires_at != "2099-12-31" {
        if let Ok(exp) = chrono::NaiveDate::parse_from_str(&payload.expires_at, "%Y-%m-%d") {
            if exp < chrono::Local::now().date_naive() { return Err(format!("已过期：{}", payload.expires_at)); }
        }
    }
    println!("[许可] ✓ 有效: 产品={}, 有效期={}", payload.product, payload.expires_at);
    Ok((payload.product, payload.expires_at))
}

/// 保留旧签名以供 license-builder 等工具使用
pub fn verify_license_key(key: &str) -> Result<String, String> {
    verify_license_key_and_get_expiry(key).map(|(p, _)| p)
}
