//! The expansion field (`expfield.d2`) as a native kind: a grid of cells,
//! each holding a value from 0 to 8.
//!
//! The grid becomes an indexed PNG of `width × height` pixels with the cell
//! value as the index, so it can be viewed and painted like a picture.
//! `P.toml` keeps `width`, `height` and the leading 16-bit `header` value
//! (266 in the shipped file), which nothing reads but the file carries.

use crate::kind::{
    need, pixel_diff, Difference, FileStore, NativeError, NativeFile, NativeKind, ViewPalette,
};
use crate::native_toml::Fields;
use crate::png;

/// The decoded file: header u16, rows `H`, columns `W`, then `H × W`
/// cells row-major. Equality is the exact comparison used by the round-trip check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpFieldData {
    /// The leading 16-bit value, kept but not used (266 in the shipped file).
    pub header: u16,
    /// Number of rows.
    pub height: u32,
    /// Number of columns.
    pub width: u32,
    /// `height × width` cell values (0–8), row-major.
    pub cells: Vec<u8>,
}

impl ExpFieldData {
    /// Parses the file bytes: u16 header, u32 height, u32 width, cells.
    /// Strict: the size must be exact.
    pub fn from_bytes(bytes: &[u8]) -> Result<ExpFieldData, NativeError> {
        let err = |m: String| NativeError::new("expfield.d2", m);
        if bytes.len() < 10 {
            return Err(err("shorter than its 10-byte header".into()));
        }
        let header = u16::from_le_bytes([bytes[0], bytes[1]]);
        let height = u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]);
        let width = u32::from_le_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]);
        let want = 10 + u64::from(height) * u64::from(width);
        if bytes.len() as u64 != want {
            return Err(err(format!("{} bytes, header says {want}", bytes.len())));
        }
        Ok(ExpFieldData {
            header,
            height,
            width,
            cells: bytes[10..].to_vec(),
        })
    }
}

fn toml_path(p: &str) -> String {
    format!("{p}.toml")
}
fn png_path(p: &str) -> String {
    format!("{p}.png")
}

impl NativeKind for ExpFieldData {
    const KIND: &'static str = "expfield";

    fn write(&self, p: &str, view: &ViewPalette) -> Result<Vec<NativeFile>, NativeError> {
        let (tp, pp) = (toml_path(p), png_path(p));
        if self.cells.len() as u64 != u64::from(self.width) * u64::from(self.height) {
            return Err(NativeError::new(
                &tp,
                format!(
                    "{} cells for {} x {}",
                    self.cells.len(),
                    self.width,
                    self.height
                ),
            ));
        }
        let t = format!(
            "native = \"expfield\"\nnative_version = 1\nheader = {}\nwidth = {}\nheight = {}\n",
            self.header, self.width, self.height
        );
        let mut out = vec![NativeFile {
            path: pp.clone(),
            bytes: if self.cells.is_empty() {
                Vec::new()
            } else {
                png::write_indexed(&pp, self.width, self.height, &self.cells, view)?
            },
        }];
        if self.cells.is_empty() {
            // No pixels: no PNG (sheets do the same).
            out.clear();
        }
        out.push(NativeFile {
            path: tp,
            bytes: t.into_bytes(),
        });
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }

    fn read(p: &str, store: &dyn FileStore) -> Result<Self, NativeError> {
        let (tp, pp) = (toml_path(p), png_path(p));
        let text =
            String::from_utf8(need(store, &tp)?).map_err(|_| NativeError::new(&tp, "not UTF-8"))?;
        let mut f = Fields::parse(&tp, &text)?;
        f.header("expfield", 1)?;
        let header = f.u16("header")?;
        let width = f.u32("width")?;
        let height = f.u32("height")?;
        f.finish()?;
        let cells = if width == 0 || height == 0 {
            Vec::new()
        } else {
            let (w, h, px) = png::read_indexed(&pp, &need(store, &pp)?)?;
            if (w, h) != (width, height) {
                return Err(NativeError::new(
                    &pp,
                    format!("image is {w} x {h}, the sidecar says {width} x {height}"),
                ));
            }
            px
        };
        Ok(ExpFieldData {
            header,
            height,
            width,
            cells,
        })
    }

    fn first_difference(&self, native: &Self, p: &str) -> Option<Difference> {
        let tp = toml_path(p);
        crate::kind::diff_fields!(tp, "expfield", self, native, [header, width, height]);
        pixel_diff(&self.cells, &native.cells, self.width as usize).map(|d| Difference {
            file: png_path(p),
            detail: format!("cell {d}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::{check_files, grey_palette, round_trip, CheckError, MemStore};

    const P: &str = "data/global/expfield.d2";

    fn field(w: u32, h: u32) -> ExpFieldData {
        ExpFieldData {
            header: 266,
            height: h,
            width: w,
            cells: (0..w * h).map(|i| (i % 9) as u8).collect(),
        }
    }

    #[test]
    fn round_trip_keeps_header_dimensions_and_cells() {
        let d = field(256, 256);
        let files = round_trip(&d, P, &grey_palette()).unwrap();
        let t = String::from_utf8(
            files
                .iter()
                .find(|f| f.path.ends_with(".toml"))
                .unwrap()
                .bytes
                .clone(),
        )
        .unwrap();
        assert!(
            t.contains("header = 266") && t.contains("width = 256") && t.contains("height = 256")
        );
        let png = files.iter().find(|f| f.path.ends_with(".png")).unwrap();
        let (w, h, px) = png::read_indexed("x", &png.bytes).unwrap();
        assert_eq!((w, h), (256, 256));
        assert_eq!(px, d.cells);
        round_trip(&field(3, 5), P, &grey_palette()).unwrap();
        round_trip(&field(0, 0), P, &grey_palette()).unwrap();
    }

    #[test]
    fn from_bytes_is_strict_about_size() {
        let mut b = vec![0x0a, 0x01, 2, 0, 0, 0, 3, 0, 0, 0];
        b.extend([8u8; 6]);
        let f = ExpFieldData::from_bytes(&b).unwrap();
        assert_eq!((f.header, f.height, f.width, f.cells.len()), (266, 2, 3, 6));
        b.pop();
        assert!(ExpFieldData::from_bytes(&b).is_err());
        assert!(ExpFieldData::from_bytes(&b[..9]).is_err());
    }

    #[test]
    fn perturbation_names_the_cell() {
        let d = field(4, 3);
        let mut files = d.write(P, &grey_palette()).unwrap();
        let f = files.iter_mut().find(|f| f.path.ends_with(".png")).unwrap();
        let (w, h, mut px) = png::read_indexed("x", &f.bytes).unwrap();
        px[2 * 4 + 1] = 8;
        f.bytes = png::write_indexed("x", w, h, &px, &grey_palette()).unwrap();
        let CheckError::Mismatch(diff) = check_files(&d, P, &files).unwrap_err() else {
            panic!()
        };
        assert_eq!(diff.file, format!("{P}.png"));
        assert_eq!(diff.detail, "cell pixel (1, 2): index 0 != 8");
        // header
        let mut files = d.write(P, &grey_palette()).unwrap();
        let t = files
            .iter_mut()
            .find(|f| f.path.ends_with(".toml"))
            .unwrap();
        t.bytes = String::from_utf8(t.bytes.clone())
            .unwrap()
            .replace("header = 266", "header = 267")
            .into_bytes();
        let CheckError::Mismatch(diff) = check_files(&d, P, &files).unwrap_err() else {
            panic!()
        };
        assert!(diff.detail.contains("header 266 != 267"));
        // a missing PNG is a load error naming it
        files.retain(|f| f.path.ends_with(".toml"));
        assert_eq!(
            ExpFieldData::read(P, &MemStore::from_files(&files))
                .unwrap_err()
                .file,
            format!("{P}.png")
        );
    }
}
