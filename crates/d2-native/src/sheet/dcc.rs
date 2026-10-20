//! DCC sprites: the directional animations of monsters, characters and
//! missiles. The sidecar keeps the file fields, per `[[direction]]` its
//! bounding box and compression flags, and per `[[direction.frame]]` the
//! frame's rectangle, offsets and orientation. The bit-stream details the
//! encoder needs to rebuild the file byte for byte (`final_dc6_size`,
//! `outsize_coded`, `pcd_leftover_bits`, `coded_bytes`, `optional_data`)
//! sit in `encoding` tables that a hand-made sprite may leave out.

use d2_formats::dcc::{Dcc, DccDirection, DccFrame};

use super::{
    build_sheet, files, load_pixels, read_rect, read_sidecar, rect_text, sheet_png, sheet_toml,
    SheetFrame,
};
use crate::kind::{
    diff_fields, pixel_diff, Difference, FileStore, NativeError, NativeFile, NativeKind,
    ViewPalette,
};
use crate::native_toml::hex2;
use crate::toml_kinds::{hex_bytes, unhex_bytes};

/// Bytes as a quoted string of lowercase hex pairs (`"00abff"`).
fn quoted_hex(b: &[u8]) -> String {
    format!("\"{}\"", hex_bytes(b))
}

impl NativeKind for Dcc {
    const KIND: &'static str = "dcc";

    fn write(&self, p: &str, view: &ViewPalette) -> Result<Vec<NativeFile>, NativeError> {
        let (tp, pp) = (sheet_toml(p), sheet_png(p));
        for (d, dir) in self.directions.iter().enumerate() {
            if dir.frames.len() != self.frames_per_direction as usize {
                return Err(NativeError::new(
                    &tp,
                    format!(
                        "direction {d}: {} frames, frames_per_direction = {}",
                        dir.frames.len(),
                        self.frames_per_direction
                    ),
                ));
            }
        }
        let dirs: Vec<Vec<SheetFrame<'_>>> = self
            .directions
            .iter()
            .map(|d| {
                d.frames
                    .iter()
                    .map(|fr| SheetFrame {
                        w: fr.width,
                        h: fr.height,
                        pixels: &fr.pixels,
                    })
                    .collect()
            })
            .collect();
        let (lay, png) = build_sheet(&pp, &dirs, view)?;
        let mut t = String::new();
        t += "native = \"dcc\"\nnative_version = 1\n";
        t += &format!("version = {}\n", self.version);
        t += &format!("frames_per_direction = {}\n", self.frames_per_direction);
        t += &format!("tag = {}\n", self.tag);
        t += &format!("sheet = {}\n\n", lay.has_pixels());
        t += &format!("[encoding]\nfinal_dc6_size = {}\n", self.final_dc6_size);
        for (di, d) in self.directions.iter().enumerate() {
            t += "\n[[direction]]\n";
            t += &format!("compression_flags = {}\n", hex2(d.compression_flags));
            t += &format!("x_min = {}\ny_min = {}\n", d.x_min, d.y_min);
            t += &format!("width = {}\nheight = {}\n", d.width, d.height);
            t += &format!(
                "[direction.encoding]\noutsize_coded = {}\npcd_leftover_bits = {}\n",
                d.outsize_coded, d.pcd_leftover_bits
            );
            for (fi, fr) in d.frames.iter().enumerate() {
                t += "\n[[direction.frame]]\n";
                t += &format!("rect = {}\n", rect_text(lay.rects[di][fi]));
                t += &format!("variable0 = {}\n", fr.variable0);
                t += &format!("x_offset = {}\ny_offset = {}\n", fr.x_offset, fr.y_offset);
                t += &format!("bottom_up = {}\n", fr.bottom_up);
                t += &format!("x_min = {}\ny_min = {}\n", fr.x_min, fr.y_min);
                t += &format!(
                    "[direction.frame.encoding]\ncoded_bytes = {}\noptional_data = {}\n",
                    fr.coded_bytes,
                    quoted_hex(&fr.optional_data)
                );
            }
        }
        Ok(files(p, t, png))
    }

    fn read(p: &str, store: &dyn FileStore) -> Result<Self, NativeError> {
        let (tp, pp) = (sheet_toml(p), sheet_png(p));
        let mut f = read_sidecar(store, &tp)?;
        f.header("dcc", 1)?;
        let version = f.u8("version")?;
        let frames_per_direction = f.u32("frames_per_direction")?;
        let tag = f.u32("tag")?;
        let sheet = f.bool("sheet")?;
        let final_dc6_size = match f.opt_table("encoding")? {
            Some(mut e) => {
                let v = e.u32("final_dc6_size")?;
                e.finish()?;
                v
            }
            None => 0,
        };
        let mut dirs = f.tables("direction")?;
        let mut rects = Vec::new();
        let mut shells = Vec::new();
        for (di, d) in dirs.iter_mut().enumerate() {
            let compression_flags = d.hex_u8("compression_flags")?;
            let x_min = d.i32("x_min")?;
            let y_min = d.i32("y_min")?;
            let width = d.u32("width")?;
            let height = d.u32("height")?;
            let (outsize_coded, pcd_leftover_bits) = match d.opt_table("encoding")? {
                Some(mut e) => {
                    let a = e.u32("outsize_coded")?;
                    let b = e.int("pcd_leftover_bits")?;
                    let b = usize::try_from(b)
                        .map_err(|_| e.err("`pcd_leftover_bits` out of range"))?;
                    e.finish()?;
                    (a, b)
                }
                None => (0, 0),
            };
            let mut frames = d.tables("frame")?;
            if frames.len() != frames_per_direction as usize {
                return Err(f.err(format!(
                    "direction {di}: {} frames, frames_per_direction = {frames_per_direction}",
                    frames.len()
                )));
            }
            let mut rr = Vec::new();
            let mut fs = Vec::new();
            for fr in frames.iter_mut() {
                let rect = read_rect(fr)?;
                let variable0 = fr.u32("variable0")?;
                let x_offset = fr.i32("x_offset")?;
                let y_offset = fr.i32("y_offset")?;
                let bottom_up = fr.bool("bottom_up")?;
                let fx_min = fr.i32("x_min")?;
                let fy_min = fr.i32("y_min")?;
                let (coded_bytes, optional_data) = match fr.opt_table("encoding")? {
                    Some(mut e) => {
                        let a = e.u32("coded_bytes")?;
                        let s = e.str("optional_data")?;
                        let b = unhex_bytes(&s).ok_or_else(|| {
                            e.err("`optional_data` must be lowercase hex digit pairs")
                        })?;
                        e.finish()?;
                        (a, b)
                    }
                    None => (0, Vec::new()),
                };
                fr.finish()?;
                fs.push(DccFrame {
                    variable0,
                    width: rect[2],
                    height: rect[3],
                    x_offset,
                    y_offset,
                    coded_bytes,
                    bottom_up,
                    optional_data,
                    x_min: fx_min,
                    y_min: fy_min,
                    pixels: Vec::new(),
                });
                rr.push(rect);
            }
            d.finish()?;
            rects.push(rr);
            shells.push(DccDirection {
                outsize_coded,
                compression_flags,
                x_min,
                y_min,
                width,
                height,
                frames: fs,
                pcd_leftover_bits,
            });
        }
        f.finish()?;
        let px = load_pixels(&tp, &pp, sheet, &rects, store)?;
        for (d, pd) in shells.iter_mut().zip(px) {
            for (fr, pixels) in d.frames.iter_mut().zip(pd) {
                fr.pixels = pixels;
            }
        }
        Ok(Dcc {
            version,
            frames_per_direction,
            tag,
            final_dc6_size,
            directions: shells,
        })
    }

    fn first_difference(&self, native: &Self, p: &str) -> Option<Difference> {
        let (tp, pp) = (sheet_toml(p), sheet_png(p));
        diff_fields!(
            tp,
            "file",
            self,
            native,
            [version, frames_per_direction, tag, final_dc6_size]
        );
        if self.directions.len() != native.directions.len() {
            return Some(Difference {
                file: tp,
                detail: format!(
                    "{} directions != {}",
                    self.directions.len(),
                    native.directions.len()
                ),
            });
        }
        for (di, (a, b)) in self.directions.iter().zip(&native.directions).enumerate() {
            let at = format!("direction {di}");
            diff_fields!(
                tp,
                at,
                a,
                b,
                [
                    outsize_coded,
                    compression_flags,
                    x_min,
                    y_min,
                    width,
                    height,
                    pcd_leftover_bits
                ]
            );
            if a.frames.len() != b.frames.len() {
                return Some(Difference {
                    file: tp,
                    detail: format!("{at}: {} frames != {}", a.frames.len(), b.frames.len()),
                });
            }
            for (fi, (fa, fb)) in a.frames.iter().zip(&b.frames).enumerate() {
                let at = format!("direction {di} frame {fi}");
                diff_fields!(
                    tp,
                    at,
                    fa,
                    fb,
                    [
                        variable0,
                        width,
                        height,
                        x_offset,
                        y_offset,
                        coded_bytes,
                        bottom_up,
                        optional_data,
                        x_min,
                        y_min
                    ]
                );
                if let Some(d) = pixel_diff(&fa.pixels, &fb.pixels, fa.width as usize) {
                    return Some(Difference {
                        file: pp,
                        detail: format!("{at} {d}"),
                    });
                }
            }
        }
        None
    }
}
