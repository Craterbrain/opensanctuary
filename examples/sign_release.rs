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
        Some("sign-plugin") => {
            let (Some(sk_path), Some(folder)) = (args.get(2), args.get(3)) else {
                eprintln!("Usage: sign_release sign-plugin <secret-key-path> <plugin-folder> [password]");
                return ExitCode::FAILURE;
            };
            sign_plugin(sk_path, folder, args.get(4).cloned())
        }
        _ => {
            eprintln!("Usage:");
            eprintln!("  sign_release generate-keypair <prefix> [password]");
            eprintln!("  sign_release sign <secret-key-path> <file-to-sign> [password]");
            eprintln!("  sign_release sign-plugin <secret-key-path> <plugin-folder> [password]");
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

/// Signs a whole plugin folder with `sk_path`: lists every file's hash in `plugin.json`, sets `publisher_key` to the
/// key's public half, and writes `plugin.json.minisig`. Used by the release process for the bundled plugins
/// (docs/PLUGIN_SIGNING.md) and by tests with the throwaway keys in testdata/signing/.
fn sign_plugin(sk_path: &str, folder: &str, password: Option<String>) -> ExitCode {
    // Real release keys are encrypted; the throwaway test keys in testdata/signing/ are not.
    let loaded = match fs::read_to_string(sk_path).ok().and_then(|t| minisign::SecretKeyBox::from_string(&t).ok()) {
        Some(b) if password.is_none() => b.into_unencrypted_secret_key().or_else(|_| minisign::SecretKey::from_file(sk_path, None)),
        _ => minisign::SecretKey::from_file(sk_path, password),
    };
    let sk = match loaded {
        Ok(sk) => sk,
        Err(e) => {
            eprintln!("Failed to load secret key {}: {}", sk_path, e);
            return ExitCode::FAILURE;
        }
    };
    let pk = match minisign::PublicKey::from_secret_key(&sk) {
        Ok(pk) => pk,
        Err(e) => {
            eprintln!("Couldn't derive the public key: {}", e);
            return ExitCode::FAILURE;
        }
    };
    let folder = Path::new(folder);
    let count = match os_next::core::plugin_registry::write_file_list(folder, Some(&pk.to_base64())) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("{}", e);
            return ExitCode::FAILURE;
        }
    };
    let manifest = folder.join("plugin.json");
    let data = match fs::read(&manifest) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to read {}: {}", manifest.display(), e);
            return ExitCode::FAILURE;
        }
    };
    let sig = match minisign::sign(None, &sk, &data[..], None, None) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Signing failed: {}", e);
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = fs::write(folder.join("plugin.json.minisig"), sig.into_string()) {
        eprintln!("Failed to write the signature: {}", e);
        return ExitCode::FAILURE;
    }
    println!("Signed {} ({} files listed) with key {}", folder.display(), count, pk.to_base64());
    ExitCode::SUCCESS
}
