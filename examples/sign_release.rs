//! Release-signing tool (docs/update.md's "Release signing key" plan).
//!
//! Dev-only (the `minisign` crate is a dev-dependency, not shipped in the
//! app) -- this is what a release process runs to sign `checksums.txt`
//! before uploading it alongside the installer. The app itself only ever
//! *verifies* (via `minisign-verify`, a real dependency; see
//! `src/network/updater.rs`), never signs.
//!
//! Usage:
//!   cargo run --example sign_release -- generate-keypair <prefix> [password]
//!   cargo run --example sign_release -- sign <secret-key-path> <file-to-sign> [password]
//!
//! `generate-keypair foo` writes `foo.key` (secret, password-protected) and
//! `foo.pub` (public, safe to embed/publish). `sign` writes `<file>.minisig`.

use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("generate-keypair") => {
            let Some(prefix) = args.get(2) else {
                eprintln!("Usage: sign_release generate-keypair <prefix> [password]");
                return ExitCode::FAILURE;
            };
            let password = args.get(3).cloned();
            generate_keypair(prefix, password)
        }
        Some("sign") => {
            let (Some(sk_path), Some(file_path)) = (args.get(2), args.get(3)) else {
                eprintln!("Usage: sign_release sign <secret-key-path> <file-to-sign> [password]");
                return ExitCode::FAILURE;
            };
            let password = args.get(4).cloned();
            sign_file(sk_path, file_path, password)
        }
        _ => {
            eprintln!("Usage:");
            eprintln!("  sign_release generate-keypair <prefix> [password]");
            eprintln!("  sign_release sign <secret-key-path> <file-to-sign> [password]");
            ExitCode::FAILURE
        }
    }
}

fn generate_keypair(prefix: &str, password: Option<String>) -> ExitCode {
    let kp = match minisign::KeyPair::generate_encrypted_keypair(password) {
        Ok(kp) => kp,
        Err(e) => {
            eprintln!("Failed to generate keypair: {}", e);
            return ExitCode::FAILURE;
        }
    };

    let sk_path = format!("{}.key", prefix);
    let pk_path = format!("{}.pub", prefix);

    let sk_box = kp.sk.to_box(Some("release signing key")).expect("encode secret key");
    let pk_box = kp.pk.to_box().expect("encode public key");

    if let Err(e) = fs::write(&sk_path, sk_box.into_string()) {
        eprintln!("Failed to write {}: {}", sk_path, e);
        return ExitCode::FAILURE;
    }
    if let Err(e) = fs::write(&pk_path, pk_box.into_string()) {
        eprintln!("Failed to write {}: {}", pk_path, e);
        return ExitCode::FAILURE;
    }

    println!("Wrote {} (secret -- keep safe, see docs/update.md) and {} (public -- safe to embed/publish).", sk_path, pk_path);
    println!("Public key (base64, for embedding): {}", kp.pk.to_base64());
    ExitCode::SUCCESS
}

fn sign_file(sk_path: &str, file_path: &str, password: Option<String>) -> ExitCode {
    let sk = match minisign::SecretKey::from_file(sk_path, password) {
        Ok(sk) => sk,
        Err(e) => {
            eprintln!("Failed to load secret key {}: {}", sk_path, e);
            return ExitCode::FAILURE;
        }
    };
    let data_file = match fs::File::open(file_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Failed to open {}: {}", file_path, e);
            return ExitCode::FAILURE;
        }
    };

    let sig_box = match minisign::sign(None, &sk, data_file, None, None) {
        Ok(sig) => sig,
        Err(e) => {
            eprintln!("Signing failed: {}", e);
            return ExitCode::FAILURE;
        }
    };

    let sig_path = format!("{}.minisig", file_path);
    if let Err(e) = fs::write(&sig_path, sig_box.into_string()) {
        eprintln!("Failed to write {}: {}", sig_path, e);
        return ExitCode::FAILURE;
    }

    println!("Signed {} -> {}", Path::new(file_path).display(), sig_path);
    ExitCode::SUCCESS
}
