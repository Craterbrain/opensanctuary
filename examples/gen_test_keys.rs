//! Writes the throwaway key pairs in testdata/signing/ (unencrypted, public, worthless: see its README).
//!   cargo run --example gen_test_keys -- <output dir> <name>...
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = std::path::Path::new(&args[1]);
    for name in &args[2..] {
        let kp = minisign::KeyPair::generate_unencrypted_keypair().expect("generate");
        let sk = kp.sk.to_box(Some(&format!("OpenSanctuary TEST key ({name}): never use for anything real"))).expect("secret key");
        let pk = kp.pk.to_box().expect("public key");
        std::fs::write(dir.join(format!("{name}.key")), sk.into_string()).unwrap();
        std::fs::write(dir.join(format!("{name}.pub")), pk.into_string()).unwrap();
        println!("{name}: {}", kp.pk.to_base64());
    }
}
