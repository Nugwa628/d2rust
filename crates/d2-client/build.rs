//! Windows: bakes the game's icon and version resource into
//! D2RUST-Game.exe, so Explorer, the taskbar and Apps & Features show
//! "D2RUST" and the version. The icon is drawn here, the same gold-trimmed
//! red tile as the launcher's, until real branding art lands.
//!
//! The version is the crate's (0.1.1) plus a 4th part, the build number:
//! `$D2RUST_BUILD` (set by the release build; the older `$D2RS_BUILD` is
//! read too, legacy-name), else 0.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=D2RUST_BUILD");
    println!("cargo:rerun-if-env-changed=D2RS_BUILD");
    let build: u16 = std::env::var("D2RUST_BUILD")
        .or_else(|_| std::env::var("D2RS_BUILD"))
        .ok()
        .and_then(|b| b.trim().parse().ok())
        .unwrap_or(0);
    // The full version, for `--version` and the crash log.
    let version = format!("{}.{build}", env!("CARGO_PKG_VERSION"));
    println!("cargo:rustc-env=D2RUST_VERSION={version}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico = out.join("d2rust-game.ico");
    std::fs::write(&ico, icon_file(&[16, 32, 48, 256])).unwrap();

    let mut parts = env!("CARGO_PKG_VERSION")
        .split('.')
        .map(|p| p.parse::<u16>().unwrap_or(0));
    let [major, minor, patch] = [(); 3].map(|_| parts.next().unwrap_or(0));
    let rc = out.join("d2rust-game.rc");
    std::fs::write(
        &rc,
        format!(
            r#"1 ICON "{ico}"
1 VERSIONINFO
FILEVERSION {major},{minor},{patch},{build}
PRODUCTVERSION {major},{minor},{patch},{build}
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "WerdeGames"
      VALUE "FileDescription", "D2RUST"
      VALUE "FileVersion", "{version}"
      VALUE "ProductName", "D2RUST"
      VALUE "ProductVersion", "{version}"
      VALUE "InternalName", "D2RUST-Game"
      VALUE "OriginalFilename", "D2RUST-Game.exe"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
            ico = ico.display().to_string().replace('\\', "/"),
        ),
    )
    .unwrap();
    embed_resource::compile(&rc, embed_resource::NONE)
        .manifest_required()
        .unwrap();
}

/// RGBA pixel of the `n` x `n` icon at (x, y): a dark edge, a gold trim,
/// a red field.
fn pixel(x: usize, y: usize, n: usize) -> [u8; 4] {
    let edge = x.min(y).min(n - 1 - x).min(n - 1 - y);
    let border = (n / 32).max(1);
    if edge < border {
        [9, 8, 7, 255]
    } else if edge < border * 3 {
        [199, 179, 119, 255]
    } else {
        [122, 22, 14, 255]
    }
}

/// An .ico holding one 32-bit BMP image per size.
fn icon_file(sizes: &[usize]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&n| bmp_image(n)).collect();
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len();
    for (&n, img) in sizes.iter().zip(&images) {
        let dim = if n >= 256 { 0 } else { n as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0, 1, 0, 32, 0]);
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += img.len();
    }
    for img in images {
        out.extend_from_slice(&img);
    }
    out
}

/// One icon image: a BITMAPINFOHEADER, the BGRA rows bottom-up, then an
/// all-zero AND mask (the alpha channel does the masking).
fn bmp_image(n: usize) -> Vec<u8> {
    let mask_row = n.div_ceil(32) * 4;
    let mut out = Vec::new();
    // The height counts the colour and mask images together.
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(n as i32).to_le_bytes());
    out.extend_from_slice(&(2 * n as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for y in (0..n).rev() {
        for x in 0..n {
            let [r, g, b, a] = pixel(x, y, n);
            out.extend_from_slice(&[b, g, r, a]);
        }
    }
    out.extend(std::iter::repeat_n(0u8, mask_row * n));
    out
}
