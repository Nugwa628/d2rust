//! DC6 sprites: the user interface art, item pictures and other sprites
//! stored frame by frame. The sidecar keeps the header (`[header]`) and,
//! per `[[direction]]`, one `[[direction.frame]]` table per frame with its
//! rectangle, flip flag, size, offsets and the encoder's `next_block`.

use d2_formats::dc6::{Dc6, Dc6Frame, Dc6Header};

use super::{
    build_sheet, files, load_pixels, read_rect, read_sidecar, rect_text, sheet_png, sheet_toml,
    SheetFrame,
};
use crate::kind::{
    diff_fields, pixel_diff, Difference, FileStore, NativeError, NativeFile, NativeKind,
    ViewPalette,
};
use crate::native_toml::hex8;

/// A frame's sidecar fields, read before its pixels are cut out.
struct FrameFields {
    flip: u32,
    width: u32,
    height: u32,
    offset_x: i32,
    offset_y: i32,
    unknown: u32,
    next_block: u32,
}

impl NativeKind for Dc6 {
    const KIND: &'static str = "dc6";

    fn write(&self, p: &str, view: &ViewPalette) -> Result<Vec<NativeFile>, NativeError> {
        let (tp, pp) = (sheet_toml(p), sheet_png(p));
        let h = &self.header;
        let (d, f) = (h.directions as usize, h.frames_per_direction as usize);
        if self.frames.len() != d * f {
            return Err(NativeError::new(
                &tp,
                format!(
                    "{} frames for {d} directions x {f} frames",
                    self.frames.len()
                ),
            ));
        }
        let dirs: Vec<Vec<SheetFrame<'_>>> = (0..d)
            .map(|di| {
                self.frames[di * f..(di + 1) * f]
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
        t += "native = \"dc6\"\nnative_version = 1\n";
        t += &format!("sheet = {}\n\n[header]\n", lay.has_pixels());
        t += &format!("version = {}\n", h.version);
        t += &format!("flags = {}\n", hex8(h.flags));
        t += &format!("encoding = {}\n", h.encoding);
        t += &format!(
            "termination = {}\n",
            hex8(u32::from_be_bytes(h.termination))
        );
        t += &format!("directions = {}\n", h.directions);
        t += &format!("frames_per_direction = {}\n", h.frames_per_direction);
        for (di, row) in lay.rects.iter().enumerate() {
            t += "\n[[direction]]\n";
            for (fi, r) in row.iter().enumerate() {
                let fr = &self.frames[di * f + fi];
                t += "\n[[direction.frame]]\n";
                t += &format!("rect = {}\n", rect_text(*r));
                t += &format!("flip = {}\n", fr.flip);
                t += &format!("width = {}\nheight = {}\n", fr.width, fr.height);
                t += &format!("offset_x = {}\noffset_y = {}\n", fr.offset_x, fr.offset_y);
                t += &format!("unknown = {}\n", fr.unknown);
                t += &format!(
                    "[direction.frame.encoding]\nnext_block = {}\n",
                    fr.next_block
                );
            }
        }
        Ok(files(p, t, png))
    }

    fn read(p: &str, store: &dyn FileStore) -> Result<Self, NativeError> {
        let (tp, pp) = (sheet_toml(p), sheet_png(p));
        let mut f = read_sidecar(store, &tp)?;
        f.header("dc6", 1)?;
        let sheet = f.bool("sheet")?;
        let mut h = f.table("header")?;
        let version = h.i32("version")?;
        let flags = h.hex_u32("flags")?;
        let encoding = h.u32("encoding")?;
        let termination = h.hex_u32("termination")?.to_be_bytes();
        let directions = h.u32("directions")?;
        let frames_per_direction = h.u32("frames_per_direction")?;
        h.finish()?;
        let mut dirs = f.tables("direction")?;
        if dirs.len() != directions as usize {
            return Err(f.err(format!(
                "directions = {directions} but {} [[direction]] tables",
                dirs.len()
            )));
        }
        let mut rects = Vec::new();
        let mut raws = Vec::new();
        for (di, d) in dirs.iter_mut().enumerate() {
            let mut frames = d.tables("frame")?;
            if frames.len() != frames_per_direction as usize {
                return Err(f.err(format!(
                    "direction {di}: {} frames, frames_per_direction = {frames_per_direction}",
                    frames.len()
                )));
            }
            let (mut rr, mut rw) = (Vec::new(), Vec::new());
            for (fi, fr) in frames.iter_mut().enumerate() {
                let rect = read_rect(fr)?;
                let raw = FrameFields {
                    flip: fr.u32("flip")?,
                    width: fr.u32("width")?,
                    height: fr.u32("height")?,
                    offset_x: fr.i32("offset_x")?,
                    offset_y: fr.i32("offset_y")?,
                    unknown: fr.u32("unknown")?,
                    next_block: match fr.opt_table("encoding")? {
                        Some(mut e) => {
                            let v = e.u32("next_block")?;
                            e.finish()?;
                            v
                        }
                        None => 0,
                    },
                };
                if (raw.width, raw.height) != (rect[2], rect[3]) {
                    return Err(f.err(format!(
                        "direction {di} frame {fi}: width / height differ from rect"
                    )));
                }
                fr.finish()?;
                rr.push(rect);
                rw.push(raw);
            }
            d.finish()?;
            rects.push(rr);
            raws.push(rw);
        }
        f.finish()?;
        let px = load_pixels(&tp, &pp, sheet, &rects, store)?;
        let frames = raws
            .into_iter()
            .zip(px)
            .flat_map(|(rw, pd)| rw.into_iter().zip(pd))
            .map(|(r, pixels)| Dc6Frame {
                flip: r.flip,
                width: r.width,
                height: r.height,
                offset_x: r.offset_x,
                offset_y: r.offset_y,
                unknown: r.unknown,
                next_block: r.next_block,
                pixels,
            })
            .collect();
        Ok(Dc6 {
            header: Dc6Header {
                version,
                flags,
                encoding,
                termination,
                directions,
                frames_per_direction,
            },
            frames,
        })
    }

    fn first_difference(&self, native: &Self, p: &str) -> Option<Difference> {
        let (tp, pp) = (sheet_toml(p), sheet_png(p));
        diff_fields!(
            tp,
            "header",
            self.header,
            native.header,
            [
                version,
                flags,
                encoding,
                termination,
                directions,
                frames_per_direction
            ]
        );
        if self.frames.len() != native.frames.len() {
            return Some(Difference {
                file: tp,
                detail: format!("{} frames != {}", self.frames.len(), native.frames.len()),
            });
        }
        let f = self.header.frames_per_direction.max(1) as usize;
        for (i, (a, b)) in self.frames.iter().zip(&native.frames).enumerate() {
            let at = format!("direction {} frame {}", i / f, i % f);
            diff_fields!(
                tp,
                at,
                a,
                b,
                [flip, width, height, offset_x, offset_y, unknown, next_block]
            );
            if let Some(d) = pixel_diff(&a.pixels, &b.pixels, a.width as usize) {
                return Some(Difference {
                    file: pp,
                    detail: format!("{at} {d}"),
                });
            }
        }
        None
    }
}
