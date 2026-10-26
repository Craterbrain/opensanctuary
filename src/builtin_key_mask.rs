//! Masks the YouVersion key that release builds carry, so it is not stored as readable text in the
//! binary. Shared by `build.rs` (which masks it) and `api::builtin_key` (which unmasks it at run time).
//!
//! This is obfuscation, not secrecy: the app has to send the key to YouVersion, so anyone who runs it
//! can capture it. The point is that `strings`, a search of the installer, the plugin zip or a git
//! clone does not find it. A leaked key is rotated in the YouVersion portal (docs/LICENSED_SCRIPTURE.md).

use sha2::{Digest, Sha256};

/// A keystream of `len` bytes from SHA-256 over `pad || block counter`.
fn keystream(pad: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + 32);
    let mut counter: u32 = 0;
    while out.len() < len {
        let mut h = Sha256::new();
        h.update(pad);
        h.update(counter.to_le_bytes());
        out.extend_from_slice(&h.finalize());
        counter += 1;
    }
    out.truncate(len);
    out
}

/// `data` XOR the keystream of `pad`: the same call masks and unmasks.
pub fn xor_with_pad(data: &[u8], pad: &[u8]) -> Vec<u8> {
    data.iter().zip(keystream(pad, data.len())).map(|(a, b)| a ^ b).collect()
}
