//! Arkmind License Generator — Ed25519 asymmetric key signing
//!
//! License key format: Arkmind-{base64_payload}.{base64_signature}
//!
//! Payload (JSON): { product, machine_code, expires_at, issued_at }
//!
//! Workflow:
//!   1. Generate signing keypair (once) → save private key securely
//!   2. For each customer: sign { product, machine_code, expires } with private key
//!   3. Customer pastes license key → app verifies with embedded public key

use base64::Engine;
use clap::{Parser, Subcommand};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

fn gen_machine_hash(code: &str) -> String {
    let mut h = Sha256::new();
    h.update(code.as_bytes());
    hex::encode(&h.finalize()[..8])
}

#[derive(Debug, Serialize, Deserialize)]
struct LicensePayload {
    product: String,
    machine_code: String,
    expires_at: String,
    issued_at: String,
}

#[derive(Parser)]
#[command(name = "license-gen", about = "Arkmind License Key Generator")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate a new Ed25519 keypair (one-time setup)
    Keygen,
    /// Sign a license for a customer
    Sign {
        /// License tier: basic | advanced
        #[arg(short, long, default_value = "basic")]
        product: String,
        /// Machine code from the customer's app
        #[arg(short, long)]
        machine: String,
        /// Expiry date (YYYY-MM-DD), default: 2099-12-31
        #[arg(short, long, default_value = "2099-12-31")]
        expires: String,
        /// Output private key file (default: private.key)
        #[arg(long, default_value = "private.key")]
        key: String,
    },
    /// Show the public key (embed in source code)
    ShowPubkey {
        #[arg(long, default_value = "private.key")]
        key: String,
    },
    /// Validate a license key
    Verify {
        /// License key string
        license: String,
        /// Machine code
        #[arg(short, long)]
        machine: String,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Keygen => {
            // Generate fresh keypair
            let mut rng = rand_core::OsRng;
            let signing_key = SigningKey::generate(&mut rng);
            let verifying_key = signing_key.verifying_key();

            // Save private key
            let pk_bytes = signing_key.to_bytes();
            let pk_b64 = base64::engine::general_purpose::STANDARD.encode(pk_bytes);
            std::fs::write("private.key", &pk_b64).expect("failed to write private.key");

            // Also save public key
            let vk_bytes = verifying_key.as_bytes();
            let vk_b64 = base64::engine::general_purpose::STANDARD.encode(vk_bytes);
            std::fs::write("public.key", &vk_b64).expect("failed to write public.key");

            // Print public key for embedding
            println!("✅ Keypair generated!");
            println!("   Private key saved to: private.key (KEEP SECRET!)");
            println!("   Public key saved to:  public.key");
            println!();
            println!("   Public key (base64) for code embedding:");
            println!("   {}", vk_b64);
            println!();
            println!("   Rust code snippet:");
            println!("   const PUBLIC_KEY_B64: &str = \"{}\";", vk_b64);
        }
        Commands::Sign { product, machine, expires, key } => {
            // Read private key
            let pk_b64 = std::fs::read_to_string(&key).expect("failed to read private key file");
            let pk_b64 = pk_b64.trim();
            let pk_bytes = base64::engine::general_purpose::STANDARD
                .decode(pk_b64)
                .expect("invalid base64 private key");

            let signing_key = SigningKey::from_bytes(&pk_bytes.try_into().expect("private key must be 32 bytes"));

            // Build payload
            let issued = chrono::Local::now().format("%Y-%m-%d").to_string();
            let machine_hash = gen_machine_hash(&machine);
            let payload = LicensePayload {
                product,
                machine_code: machine_hash,
                expires_at: expires,
                issued_at: issued,
            };
            let payload_json = serde_json::to_string(&payload).expect("serialize");
            let payload_b64 = base64::engine::general_purpose::STANDARD.encode(payload_json.as_bytes());

            // Sign
            let message = format!("Arkmind:{}", payload_b64);
            let signature = signing_key.sign(message.as_bytes());

            // Encode
            let sig_b64 = base64::engine::general_purpose::STANDARD.encode(signature.to_bytes());

            let license = format!("Arkmind-{}.{}", payload_b64, sig_b64);
            println!("✅ License generated!");
            println!();
            println!("   Product:   {}", payload.product);
            println!("   Machine:   {} → hash: {}", machine, machine_hash);
            println!("   Expires:   {}", payload.expires_at);
            println!("   Issued:    {}", payload.issued_at);
            println!();
            println!("   License Key:");
            println!("   {}", license);
            println!();
            println!("   Copy this entire line to the customer.");
        }
        Commands::ShowPubkey { key } => {
            let pk_b64 = std::fs::read_to_string(&key).expect("failed to read private key file");
            let pk_b64 = pk_b64.trim();
            let pk_bytes = base64::engine::general_purpose::STANDARD
                .decode(pk_b64)
                .expect("invalid base64 private key");
            let signing_key = SigningKey::from_bytes(&pk_bytes.try_into().expect("32 bytes"));
            let vk_bytes = signing_key.verifying_key().as_bytes();
            let vk_b64 = base64::engine::general_purpose::STANDARD.encode(vk_bytes);
            println!("Public key (base64): {}", vk_b64);
        }
        Commands::Verify { license, machine } => {
            // Parse the license key
            let stripped = license.strip_prefix("Arkmind-").unwrap_or(&license);
            let dot_pos = stripped.rfind('.').expect("invalid license format");
            let payload_b64 = &stripped[..dot_pos];
            let sig_b64 = &stripped[dot_pos + 1..];

            let payload_bytes = base64::engine::general_purpose::STANDARD
                .decode(payload_b64)
                .expect("invalid payload base64");
            let payload_json = String::from_utf8(payload_bytes).expect("invalid utf8");
            let payload: LicensePayload = serde_json::from_str(&payload_json).expect("invalid payload json");

            let machine_hash = gen_machine_hash(&machine);
            println!("License details:");
            println!("  Product:    {}", payload.product);
            println!("  Machine:    {} (current: {})", payload.machine_code, machine_hash);
            println!("  Expires:    {}", payload.expires_at);
            println!("  Issued:     {}", payload.issued_at);
            println!("  Machine match: {}", payload.machine_code == machine_hash);
        }
    }
}
