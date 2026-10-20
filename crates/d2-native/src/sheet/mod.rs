//! Sprite sheets: DC6 and DCC sprites as one indexed PNG (`P.png`) plus a
//! TOML sidecar (`P.toml`).
//!
//! A sprite is a set of directions, each a run of frames of their own
//! sizes. The sheet puts them on one picture a person can open and paint
//! on: row `d` of the sheet is direction `d`; its frames go left to right
//! at their own widths, packed with no gap, top edges on the row's top.
//! A row is as tall as its tallest frame, the sheet as wide as its widest
//! row, and every pixel no frame covers is index 0 (transparent). A sprite
//! with no pixels at all (every frame 0 wide or 0 high) has no PNG, and
//! its sidecar says `sheet = false`.
//!
//! The sidecar carries everything else: the header fields, and per frame
//! its rectangle on the sheet, offsets and flags. Fields that only matter
//! for re-encoding the original file sit in `encoding` tables; a sprite a
//! modder writes from scratch may leave those out and gets zeros.
//!
//! The reader is strict about the layout: the rectangles in the sidecar
//! must be exactly the layout the frame sizes give, and the PNG exactly the
//! sheet size, so a modder who resizes a frame also fixes the sidecar.
//!
//! The two sprite kinds share the layout and the PNG half here; their
//! sidecars are in [`dc6`] and [`dcc`].

mod dc6;
mod dcc;
#[cfg(test)]
mod tests;

use crate::kind::{need, FileStore, NativeError, NativeFile, ViewPalette};
use crate::native_toml::{array, Fields};
use crate::png;

/// Largest sheet, in pixels, either way: keeps a hostile sidecar from
/// asking for gigabytes.
const MAX_SHEET_PIXELS: u64 = 1 << 28;

/// One frame as the sheet sees it.
struct SheetFrame<'a> {
    w: u32,
    h: u32,
    pixels: &'a [u8],
}

/// A frame's rectangle on the sheet: `[x, y, width, height]`.
type Rect = [u32; 4];

/// The place of every frame in the sheet and the sheet's size.
#[derive(Debug, PartialEq, Eq)]
struct Layout {
    rects: Vec<Vec<Rect>>,
    width: u32,
    height: u32,
}

impl Layout {
    fn has_pixels(&self) -> bool {
        self.width > 0 && self.height > 0
    }
}

/// The layout of frames of the given `(w, h)` sizes, per direction.
fn layout(sizes: &[Vec<(u32, u32)>]) -> Result<Layout, String> {
    let (mut width, mut y) = (0u64, 0u64);
    let mut rects = Vec::with_capacity(sizes.len());
    for dir in sizes {
        let (mut x, mut row_h) = (0u64, 0u64);
        let mut row = Vec::with_capacity(dir.len());
        for &(w, h) in dir {
            row.push([
                u32::try_from(x).map_err(|_| "sheet too wide")?,
                u32::try_from(y).map_err(|_| "sheet too tall")?,
                w,
                h,
            ]);
            x += u64::from(w);
            row_h = row_h.max(u64::from(h));
        }
        width = width.max(x);
        y += row_h;
        rects.push(row);
    }
    if width.saturating_mul(y) > MAX_SHEET_PIXELS {
        return Err(format!("sheet {width} x {y} too large"));
    }
    Ok(Layout {
        rects,
        width: u32::try_from(width).map_err(|_| "sheet too wide")?,
        height: u32::try_from(y).map_err(|_| "sheet too tall")?,
    })
}

/// Draws the frames into the sheet buffer.
fn render(lay: &Layout, dirs: &[Vec<SheetFrame<'_>>]) -> Vec<u8> {
    let sw = lay.width as usize;
    let mut sheet = vec![0u8; sw * lay.height as usize];
    for (rects, frames) in lay.rects.iter().zip(dirs) {
        for (r, f) in rects.iter().zip(frames) {
            let (x, y, w) = (r[0] as usize, r[1] as usize, f.w as usize);
            for row in 0..f.h as usize {
                let dst = (y + row) * sw + x;
                sheet[dst..dst + w].copy_from_slice(&f.pixels[row * w..(row + 1) * w]);
            }
        }
    }
    sheet
}

/// A frame's pixels cut out of the sheet.
fn extract(sheet: &[u8], sw: usize, r: Rect) -> Vec<u8> {
    let (x, y, w, h) = (r[0] as usize, r[1] as usize, r[2] as usize, r[3] as usize);
    let mut out = Vec::with_capacity(w * h);
    for row in 0..h {
        let at = (y + row) * sw + x;
        out.extend_from_slice(&sheet[at..at + w]);
    }
    out
}

/// A frame must hold exactly `w × h` pixels.
fn check_frame_pixels(file: &str, what: &str, f: &SheetFrame<'_>) -> Result<(), NativeError> {
    if f.pixels.len() as u64 != u64::from(f.w) * u64::from(f.h) {
        return Err(NativeError::new(
            file,
            format!(
                "{what}: {} pixels for a {} x {} frame",
                f.pixels.len(),
                f.w,
                f.h
            ),
        ));
    }
    Ok(())
}

/// Lays out, checks and encodes the frames. Returns the layout and the PNG,
/// if the sprite has any pixels.
fn build_sheet(
    png_path: &str,
    dirs: &[Vec<SheetFrame<'_>>],
    view: &ViewPalette,
) -> Result<(Layout, Option<Vec<u8>>), NativeError> {
    for (d, dir) in dirs.iter().enumerate() {
        for (f, fr) in dir.iter().enumerate() {
            check_frame_pixels(png_path, &format!("direction {d} frame {f}"), fr)?;
        }
    }
    let sizes: Vec<Vec<(u32, u32)>> = dirs
        .iter()
        .map(|d| d.iter().map(|f| (f.w, f.h)).collect())
        .collect();
    let lay = layout(&sizes).map_err(|m| NativeError::new(png_path, m))?;
    if !lay.has_pixels() {
        return Ok((lay, None));
    }
    let bytes = png::write_indexed(png_path, lay.width, lay.height, &render(&lay, dirs), view)?;
    Ok((lay, Some(bytes)))
}

/// A frame's `rect = [x, y, width, height]`.
fn read_rect(f: &mut Fields) -> Result<Rect, NativeError> {
    let v = f.ints("rect")?;
    if v.len() != 4 {
        return Err(f.err("`rect` must have 4 entries"));
    }
    let mut rect = [0u32; 4];
    for (o, i) in rect.iter_mut().zip(&v) {
        *o = u32::try_from(*i).map_err(|_| f.err("`rect` entry out of range"))?;
    }
    Ok(rect)
}

/// Cuts every frame out of the sheet, after checking the sidecar's
/// rectangles against the layout of the frame sizes and the PNG against
/// the layout's size.
fn load_pixels(
    toml_path: &str,
    png_path: &str,
    sheet: bool,
    rects: &[Vec<Rect>],
    store: &dyn FileStore,
) -> Result<Vec<Vec<Vec<u8>>>, NativeError> {
    let sizes: Vec<Vec<(u32, u32)>> = rects
        .iter()
        .map(|d| d.iter().map(|r| (r[2], r[3])).collect())
        .collect();
    let lay = layout(&sizes).map_err(|m| NativeError::new(toml_path, m))?;
    for (d, (a, b)) in lay.rects.iter().zip(rects).enumerate() {
        for (f, (want, got)) in a.iter().zip(b).enumerate() {
            if want != got {
                return Err(NativeError::new(
                    toml_path,
                    format!(
                        "direction {d} frame {f}: rect {got:?} does not match the sheet layout {want:?}"
                    ),
                ));
            }
        }
    }
    if sheet != lay.has_pixels() {
        return Err(NativeError::new(
            toml_path,
            format!(
                "sheet = {sheet} but the frames {} pixels",
                if lay.has_pixels() { "have" } else { "have no" }
            ),
        ));
    }
    let data = if sheet {
        let bytes = need(store, png_path)?;
        let (w, h, px) = png::read_indexed(png_path, &bytes)?;
        if (w, h) != (lay.width, lay.height) {
            return Err(NativeError::new(
                png_path,
                format!(
                    "sheet is {w} x {h}, the sidecar's frames need {} x {}",
                    lay.width, lay.height
                ),
            ));
        }
        px
    } else {
        Vec::new()
    };
    Ok(lay
        .rects
        .iter()
        .map(|d| {
            d.iter()
                .map(|r| extract(&data, lay.width as usize, *r))
                .collect()
        })
        .collect())
}

fn rect_text(r: Rect) -> String {
    array(r.iter().map(|v| v.to_string()))
}

fn sheet_png(p: &str) -> String {
    format!("{p}.png")
}

fn sheet_toml(p: &str) -> String {
    format!("{p}.toml")
}

/// The sidecar of `p`, parsed.
fn read_sidecar(store: &dyn FileStore, toml_path: &str) -> Result<Fields, NativeError> {
    let text = String::from_utf8(need(store, toml_path)?)
        .map_err(|_| NativeError::new(toml_path, "not UTF-8"))?;
    Fields::parse(toml_path, &text)
}

/// The written files of a sprite, sorted by path: the sidecar, and the
/// PNG when there is one.
fn files(p: &str, toml: String, png: Option<Vec<u8>>) -> Vec<NativeFile> {
    let mut v = vec![NativeFile {
        path: sheet_toml(p),
        bytes: toml.into_bytes(),
    }];
    if let Some(bytes) = png {
        v.push(NativeFile {
            path: sheet_png(p),
            bytes,
        });
    }
    v.sort_by(|a, b| a.path.cmp(&b.path));
    v
}
