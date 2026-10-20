//! Counts the DT1 tiles the native writer stores as `layout = "blocks"`,
//! the fallback layout used when a tile cannot be stored as one assembled
//! image. A development tool for watching how often the fallback happens.
//!
//! Reads `<canonical path>\t<file>` lines on stdin (one DT1 per line, the
//! winning copy of each name), writes each through the converter's DT1
//! writer and prints one line per file with fallback tiles, then the
//! totals. Files that do not parse (the unused version-4 DT1s) are
//! skipped.
//!
//! `cargo run --release -p d2-native --example dt1_layouts < list.tsv`

use d2_native::kind::{grey_palette, NativeKind};

fn main() {
    let pal = grey_palette();
    let (mut files, mut fb_files, mut assembled, mut blocks) = (0, 0, 0, 0);
    for line in std::io::stdin().lines() {
        let line = line.expect("stdin");
        let Some((rel, path)) = line.split_once('\t') else {
            continue;
        };
        let bytes = std::fs::read(path).expect("read DT1");
        let Ok(dt1) = d2_formats::dt1::Dt1::parse(&bytes) else {
            continue;
        };
        let out = dt1.write(rel, &pal).expect("native write");
        let toml = out
            .iter()
            .find(|f| f.path.ends_with(".toml"))
            .expect("sidecar");
        let text = String::from_utf8_lossy(&toml.bytes);
        let b = text.matches("layout = \"blocks\"").count();
        files += 1;
        assembled += text.matches("layout = \"assembled\"").count();
        blocks += b;
        if b > 0 {
            fb_files += 1;
            println!("{rel}\t{b}");
        }
    }
    println!(
        "files {files} with_fallback {fb_files} image_tiles {} assembled {assembled} blocks {blocks}",
        assembled + blocks
    );
}
