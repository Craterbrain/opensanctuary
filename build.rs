//! Rebuilds the frontend bundle (web/dist/) before every `cargo build`/
//! `cargo run` that could plausibly need it -- see web/tests/_pretest_build.ts
//! for the parallel fix on the test side, and why this class of bug exists:
//! nothing else regenerates web/dist/bundle.js when web/src/*.ts changes (no
//! other build.rs, no watcher running by default), so the compiled Rust
//! server keeps serving whatever JS was last built, silently, even after a
//! source fix lands. That's exactly what made a real, already-fixed bug look
//! unfixed earlier in this project's life -- the running app just hadn't
//! been told to rebuild its frontend.
//!
//! Gated by `cargo:rerun-if-changed` below, so this only actually re-invokes
//! `bun run build` when frontend source (or its own package manifest)
//! changed since the last build -- a Rust-only edit/rebuild loop never pays
//! this cost.
use std::path::Path;
use std::process::Command;

#[path = "src/builtin_key_mask.rs"]
mod mask;

/// 32 random bytes for the mask. /dev/urandom where there is one (every build host we use);
/// otherwise the clock and process id, which is enough for obfuscation.
fn random_pad() -> [u8; 32] {
    use std::io::Read;
    let mut pad = [0u8; 32];
    if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut pad)).is_ok() {
        return pad;
    }
    let seed = format!("{:?}{}", std::time::SystemTime::now(), std::process::id());
    use sha2::{Digest, Sha256};
    pad.copy_from_slice(&Sha256::digest(seed.as_bytes()));
    pad
}

/// Writes `$OUT_DIR/yvp_key.rs`: the YouVersion App Key from `YVP_APP_KEY` (a GitHub Actions secret in the
/// release workflow), masked, or nothing when it isn't set. The key itself is never printed or written
/// anywhere readable; `cargo:rerun-if-env-changed` only names the variable.
fn write_youversion_key() {
    println!("cargo:rerun-if-env-changed=YVP_APP_KEY");
    let out = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("yvp_key.rs");
    let key = std::env::var("YVP_APP_KEY").unwrap_or_default();
    let key = key.trim();
    let (pad, masked) = if key.is_empty() {
        (vec![0u8; 32], Vec::new())
    } else {
        let pad = random_pad();
        (pad.to_vec(), mask::xor_with_pad(key.as_bytes(), &pad))
    };
    let list = |b: &[u8]| b.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let src = format!("pub static YVP_PAD: [u8; 32] = [{}];\npub static YVP_MASKED: &[u8] = &[{}];\n", list(&pad), list(&masked));
    std::fs::write(out, src).expect("write yvp_key.rs");
}

fn main() {
    write_youversion_key();

    let web_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("web");

    println!("cargo:rerun-if-changed={}", web_dir.join("src").display());
    println!("cargo:rerun-if-changed={}", web_dir.join("index.html").display());
    println!("cargo:rerun-if-changed={}", web_dir.join("package.json").display());

    if std::env::var_os("OSNEXT_SKIP_FRONTEND_BUILD").is_some() {
        println!("cargo:warning=OSNEXT_SKIP_FRONTEND_BUILD set -- not rebuilding web/dist/; it may be stale.");
        return;
    }

    // A machine without bun installed at all (a Rust-only CI image, say)
    // shouldn't have its whole build fail over this -- warn and leave
    // whatever's already in web/dist/ alone rather than hard-failing.
    if Command::new("bun").arg("--version").output().is_err() {
        println!("cargo:warning=`bun` not found on PATH -- skipping frontend rebuild. web/dist/ may be stale; run `cd web && bun run build` manually before testing the app.");
        return;
    }

    let output = Command::new("bun")
        .args(["run", "build"])
        .current_dir(&web_dir)
        .output()
        .expect("failed to spawn `bun run build`");

    if !output.status.success() {
        // Cargo only surfaces a build script's output on failure (or under
        // --verbose) -- folding stdout/stderr into the panic message is what
        // actually guarantees the real bun/tsc error shows up here, not just
        // a bare non-zero exit code.
        panic!(
            "\n`bun run build` failed (exit code {:?}) -- refusing to build/run against a stale or broken web/dist/.\n\n--- stdout ---\n{}\n--- stderr ---\n{}\n",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}
