//! AES-256-GCM encryption for sensitive data (API keys, tokens)
//!
//! Key derived from machine hash to bind encrypted data to the device.

use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
use aes_gcm::aead::OsRng;
use base64::Engine;
use sha2::{Digest, Sha256};

fn derive_key(machine_hash: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(machine_hash.as_bytes());
    hasher.update(b"arkmind-ai-config-v1");
    let result = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    key
}

pub fn encrypt_api_key(plaintext: &str, machine_hash: &str) -> String {
    if plaintext.is_empty() {
        return String::new();
    }
    let key_bytes = derive_key(machine_hash);
    let cipher = Aes256Gcm::new_from_slice(&key_bytes).expect("valid key length");

    let mut nonce_bytes = [0u8; 12];
    use rand::Rng;
    OsRng.fill(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .expect("encryption failed");

    let mut combined = nonce_bytes.to_vec();
    combined.extend_from_slice(&ciphertext);
    format!("enc:{}", base64::engine::general_purpose::STANDARD.encode(&combined))
}

pub fn decrypt_api_key(encrypted: &str, machine_hash: &str) -> String {
    if encrypted.is_empty() || !encrypted.starts_with("enc:") {
        return encrypted.to_string();
    }

    let b64 = &encrypted[4..];
    let combined = match base64::engine::general_purpose::STANDARD.decode(b64) {
        Ok(data) => data,
        Err(_) => return encrypted.to_string(),
    };

    if combined.len() < 12 {
        return encrypted.to_string();
    }

    let nonce_bytes = &combined[..12];
    let ciphertext = &combined[12..];

    let key_bytes = derive_key(machine_hash);
    let cipher = match Aes256Gcm::new_from_slice(&key_bytes) {
        Ok(c) => c,
        Err(_) => return encrypted.to_string(),
    };

    let nonce = Nonce::from_slice(nonce_bytes);
    match cipher.decrypt(nonce, ciphertext) {
        Ok(plaintext) => String::from_utf8(plaintext).unwrap_or_else(|_| encrypted.to_string()),
        Err(_) => encrypted.to_string(),
    }
}

pub fn is_encrypted(value: &str) -> bool {
    value.starts_with("enc:")
}

/// 展示用脱敏：保留首尾少量字符便于识别，中段以 **** 替代；短值全遮蔽
pub fn mask_secret(value: &str) -> String {
    if value.is_empty() { return String::new(); }
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 8 { return "****".to_string(); }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len()-4..].iter().collect();
    format!("{}****{}", head, tail)
}

/// 检测前端回传的脱敏占位值（原样提交打码串会覆盖真实密钥，需跳过）
pub fn is_masked_placeholder(value: &str) -> bool {
    value.contains("****")
}

pub fn encrypt_config_api_keys(config: &serde_json::Value, machine_hash: &str) -> serde_json::Value {
    let mut config = config.clone();

    if let Some(obj) = config.as_object_mut() {
        for key in &["providers", "visionProviders"] {
            if let Some(providers) = obj.get_mut(*key).and_then(|v| v.as_array_mut()) {
                for provider in providers.iter_mut() {
                    if let Some(api_key) = provider.get("apiKey").and_then(|v| v.as_str()) {
                        if !api_key.is_empty() && !is_encrypted(api_key) {
                            let encrypted = encrypt_api_key(api_key, machine_hash);
                            provider["apiKey"] = serde_json::json!(encrypted);
                        }
                    }
                }
            }
        }

        if let Some(embedding) = obj.get_mut("embedding") {
            if let Some(api_key) = embedding.get("apiKey").and_then(|v| v.as_str()) {
                if !api_key.is_empty() && !is_encrypted(api_key) {
                    let encrypted = encrypt_api_key(api_key, machine_hash);
                    embedding["apiKey"] = serde_json::json!(encrypted);
                }
            }
        }

        if let Some(web) = obj.get_mut("webSearch") {
            if let Some(api_key) = web.get("apiKey").and_then(|v| v.as_str()) {
                if !api_key.is_empty() && !is_encrypted(api_key) {
                    let encrypted = encrypt_api_key(api_key, machine_hash);
                    web["apiKey"] = serde_json::json!(encrypted);
                }
            }
        }
    }

    config
}

pub fn decrypt_config_api_keys(config: &serde_json::Value, machine_hash: &str) -> serde_json::Value {
    let mut config = config.clone();

    if let Some(obj) = config.as_object_mut() {
        for key in &["providers", "visionProviders"] {
            if let Some(providers) = obj.get_mut(*key).and_then(|v| v.as_array_mut()) {
                for provider in providers.iter_mut() {
                    if let Some(api_key) = provider.get("apiKey").and_then(|v| v.as_str()) {
                        if is_encrypted(api_key) {
                            let decrypted = decrypt_api_key(api_key, machine_hash);
                            provider["apiKey"] = serde_json::json!(decrypted);
                        }
                    }
                }
            }
        }

        if let Some(embedding) = obj.get_mut("embedding") {
            if let Some(api_key) = embedding.get("apiKey").and_then(|v| v.as_str()) {
                if is_encrypted(api_key) {
                    let decrypted = decrypt_api_key(api_key, machine_hash);
                    embedding["apiKey"] = serde_json::json!(decrypted);
                }
            }
        }

        if let Some(web) = obj.get_mut("webSearch") {
            if let Some(api_key) = web.get("apiKey").and_then(|v| v.as_str()) {
                if is_encrypted(api_key) {
                    let decrypted = decrypt_api_key(api_key, machine_hash);
                    web["apiKey"] = serde_json::json!(decrypted);
                }
            }
        }
    }

    config
}
