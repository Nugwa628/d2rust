//! `mpq-tool render <name> [palette] [out.png]`: draw a DC6/DCC/DT1 as a PNG
//! contact sheet for visual checking. Output goes under the gitignored
//! `game/` folder by default; it contains game graphics and must never be
//! committed.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use d2_formats::dc6::Dc6;
use d2_formats::dcc::Dcc;
use d2_formats::dt1::Dt1;
use d2_formats::mpq::Archive;
use d2_formats::palette::Palette;

/// An indexed image positioned at (x, y) in its own coordinate space.
struct Sprite {
    x: i32,
    y: i32,
    w: usize,
    h: usize,
    pixels: Vec<u8>,
}

fn find(archives: &[Archive], name: &str) -> Result<Vec<u8>> {
    for a in archives {
        if a.contains(name) {
            return a.read(name).with_context(|| format!("reading {name}"));
        }
    }
    bail!("{name} not found in any archive")
}

/// Lays sprites out in a grid of cells, each the union box of all sprites
/// (so relative offsets between frames stay visible).
fn sheet(sprites: &[Sprite], columns: usize) -> (usize, usize, Vec<u8>) {
    let x0 = sprites.iter().map(|s| s.x).min().unwrap_or(0);
    let y0 = sprites.iter().map(|s| s.y).min().unwrap_or(0);
    let x1 = sprites.iter().map(|s| s.x + s.w as i32).max().unwrap_or(1);
    let y1 = sprites.iter().map(|s| s.y + s.h as i32).max().unwrap_or(1);
    let (cw, ch) = ((x1 - x0) as usize + 2, (y1 - y0) as usize + 2);
    let cols = columns.min(sprites.len()).max(1);
    let rows = sprites.len().div_ceil(cols).max(1);
    let (w, h) = (cw * cols, ch * rows);
    let mut img = vec![0u8; w * h];
    for (i, s) in sprites.iter().enumerate() {
        let ox = (i % cols) * cw + 1 + (s.x - x0) as usize;
        let oy = (i / cols) * ch + 1 + (s.y - y0) as usize;
        for row in 0..s.h {
            for col in 0..s.w {
                let p = s.pixels[row * s.w + col];
                if p != 0 {
                    img[(oy + row) * w + ox + col] = p;
                }
            }
        }
    }
    (w, h, img)
}

pub fn run(dir: &Path, name: &str, palette: Option<&str>, out: Option<PathBuf>) -> Result<()> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mpq")))
        .collect();
    // Archive priority order: patch first.
    paths.sort_by_key(|p| {
        let n = p.file_name().unwrap().to_string_lossy().to_lowercase();
        [
            "patch_d2.mpq",
            "d2exp.mpq",
            "d2xmusic.mpq",
            "d2xtalk.mpq",
            "d2xvideo.mpq",
            "d2data.mpq",
            "d2char.mpq",
        ]
        .iter()
        .position(|&x| x == n)
        .unwrap_or(99)
    });
    let archives: Vec<Archive> = paths.iter().map(Archive::open).collect::<Result<_, _>>()?;

    let pal_name = palette.unwrap_or(r"data\global\palette\act1\pal.dat");
    let pal = Palette::parse(&find(&archives, pal_name)?)?;
    let bytes = find(&archives, name)?;
    let lower = name.to_ascii_lowercase();

    let (sprites, columns): (Vec<Sprite>, usize) = if lower.ends_with(".dcc") {
        let dcc = Dcc::parse(&bytes)?;
        let dir0 = dcc.directions.first().context("no directions")?;
        let s = dir0
            .frames
            .iter()
            .map(|f| Sprite {
                x: f.x_min,
                y: f.y_min,
                w: f.width as usize,
                h: f.height as usize,
                pixels: f.pixels.clone(),
            })
            .collect();
        (s, 8)
    } else if lower.ends_with(".dc6") {
        let dc6 = Dc6::parse(&bytes)?;
        let f = dc6.header.frames_per_direction as usize;
        let s = dc6.frames[..f.min(dc6.frames.len())]
            .iter()
            .map(|f| Sprite {
                x: 0,
                y: 0,
                w: f.width as usize,
                h: f.height as usize,
                pixels: f.pixels.clone(),
            })
            .collect();
        (s, 8)
    } else if lower.ends_with(".dt1") {
        // Each tile: its blocks placed at their (x, y).
        let dt1 = Dt1::parse(&bytes)?;
        let tiles: Vec<Sprite> = dt1
            .tiles
            .iter()
            .filter(|t| !t.blocks.is_empty())
            .take(24)
            .map(|t| {
                let parts: Vec<Sprite> = t
                    .blocks
                    .iter()
                    .map(|b| {
                        let (w, h) = b.size();
                        Sprite {
                            x: i32::from(b.x),
                            y: i32::from(b.y),
                            w,
                            h,
                            pixels: b.pixels.clone(),
                        }
                    })
                    .collect();
                compose(&parts)
            })
            .collect();
        (tiles, 6)
    } else {
        bail!("render supports .dcc, .dc6 and .dt1");
    };
    if sprites.is_empty() {
        bail!("nothing to draw");
    }

    let (w, h, img) = sheet(&sprites, columns);
    // Small sheets are scaled up so details are visible.
    let scale = if w < 400 { 3 } else { 1 };
    let (w, h, img) = upscale(w, h, &img, scale);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for &p in &img {
        let c = pal.colors[usize::from(p)];
        // Index 0 is transparent: draw it as a dark grey background.
        if p == 0 {
            rgba.extend_from_slice(&[32, 32, 40, 255]);
        } else {
            rgba.extend_from_slice(&[c.r, c.g, c.b, 255]);
        }
    }
    let out = out.unwrap_or_else(|| {
        let stem = Path::new(&name.replace('\\', "/"))
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "render".into());
        PathBuf::from("game/renders").join(format!("{stem}.png"))
    });
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, png(w as u32, h as u32, &rgba))?;
    println!(
        "wrote {} ({w}x{h}, {} images)",
        out.display(),
        sprites.len()
    );
    Ok(())
}

/// Nearest-neighbour upscale by an integer factor.
fn upscale(w: usize, h: usize, img: &[u8], s: usize) -> (usize, usize, Vec<u8>) {
    if s == 1 {
        return (w, h, img.to_vec());
    }
    let mut out = vec![0u8; w * s * h * s];
    for y in 0..h * s {
        for x in 0..w * s {
            out[y * w * s + x] = img[(y / s) * w + x / s];
        }
    }
    (w * s, h * s, out)
}

/// Places sprites in one canvas at their own coordinates.
fn compose(parts: &[Sprite]) -> Sprite {
    let x0 = parts.iter().map(|s| s.x).min().unwrap_or(0);
    let y0 = parts.iter().map(|s| s.y).min().unwrap_or(0);
    let x1 = parts.iter().map(|s| s.x + s.w as i32).max().unwrap_or(1);
    let y1 = parts.iter().map(|s| s.y + s.h as i32).max().unwrap_or(1);
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let mut pixels = vec![0u8; w * h];
    for s in parts {
        let (ox, oy) = ((s.x - x0) as usize, (s.y - y0) as usize);
        for row in 0..s.h {
            for col in 0..s.w {
                let p = s.pixels[row * s.w + col];
                if p != 0 {
                    pixels[(oy + row) * w + ox + col] = p;
                }
            }
        }
    }
    Sprite {
        x: 0,
        y: 0,
        w,
        h,
        pixels,
    }
}

// --- Minimal PNG writer (RGBA8, uncompressed deflate blocks) ---

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

fn png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(rgba.len() + h as usize);
    for row in rgba.chunks(w as usize * 4) {
        raw.push(0); // filter: none
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, block) in blocks.iter().enumerate() {
        z.push(u8::from(i + 1 == blocks.len()));
        let len = block.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(block);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums() {
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }
}
