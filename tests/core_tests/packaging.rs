//! The .deb and the generic tarball must ship the same web UI. The tarball (and the Windows
//! packages) copy `web/` with shell commands in packaging/; the .deb takes its files from the
//! `[package.metadata.deb]` asset lists in Cargo.toml instead. 0.3.1's .deb shipped without
//! `web/pattern-thumbs/` because only the scripts were updated.

fn read(rel: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)).unwrap()
}

#[test]
fn the_deb_ships_every_web_file_and_folder_the_tarball_does() {
    let script = read("packaging/build-deb-in-container.sh");
    let mut wanted: Vec<String> = Vec::new();
    for line in script.lines().map(str::trim) {
        if let Some(files) = line.strip_prefix("for f in ").and_then(|l| l.strip_suffix("; do")) {
            wanted.extend(files.split_whitespace().map(|f| format!("\"web/{f}\"")));
        } else if let Some(rest) = line.strip_prefix("cp -r web/") {
            let dir = rest.split_whitespace().next().unwrap();
            wanted.push(format!("\"web/{dir}/"));
        }
    }
    assert!(wanted.len() > 5, "couldn't read the tarball's web files from the script: {wanted:?}");
    wanted.push("\"web/dist/".to_string());

    let toml = read("Cargo.toml");
    let main = toml.split("[package.metadata.deb]").nth(1).unwrap();
    let (main, variant) = main.split_once("[package.metadata.deb.variants.no-tv-apk]").unwrap();
    for (name, assets) in [("[package.metadata.deb]", main), ("variants.no-tv-apk", variant)] {
        let missing: Vec<_> = wanted.iter().filter(|w| !assets.contains(&format!("source = {w}"))).collect();
        assert!(missing.is_empty(), "{name} assets in Cargo.toml lack {missing:?}");
    }
}
