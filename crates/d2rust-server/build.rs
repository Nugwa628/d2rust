//! The server's game version: the game's own (the `d2-client` crate's
//! version) plus the build number `$D2RUST_BUILD` (else 0), the same four
//! parts the game puts in its hello. Client and server must match exactly,
//! so both read the one place the version is written.

fn main() {
    let manifest = "../d2-client/Cargo.toml";
    println!("cargo:rerun-if-changed={manifest}");
    println!("cargo:rerun-if-env-changed=D2RUST_BUILD");
    println!("cargo:rerun-if-env-changed=D2RS_BUILD");
    let text = std::fs::read_to_string(manifest).expect("the game crate's manifest");
    let version = text
        .lines()
        .find_map(|l| {
            let v = l.strip_prefix("version")?.trim().strip_prefix('=')?.trim();
            Some(v.trim_matches('"').to_owned())
        })
        .expect("the game crate's version");
    let build: u16 = std::env::var("D2RUST_BUILD")
        .or_else(|_| std::env::var("D2RS_BUILD"))
        .ok()
        .and_then(|b| b.trim().parse().ok())
        .unwrap_or(0);
    println!("cargo:rustc-env=D2RUST_VERSION={version}.{build}");
}
