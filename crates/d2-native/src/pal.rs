//! Palettes and palette transforms as native kinds.
//!
//! - A palette (`pal.dat`, 256 colours, one per act) becomes `P.pal`, a
//!   JASC-PAL text file most paint programs open directly.
//! - A palette transform (`.pl2`) is the act's set of 256-entry colour
//!   maps: light levels, inventory colour variations, the blending tables
//!   (alpha, additive, multiplicative), hue and tone shifts, text colours.
//!   It becomes `P.toml` + `P.png`: each colour map is one PNG row of 256
//!   indices, in the field order of `Pl2`, and the PNG's `PLTE` is the
//!   transform's base palette, so the picture shows the maps' colours.

use d2_formats::palette::{ColorMap, Palette, Pl2, Rgb};

use crate::kind::{
    diff_fields, need, pixel_diff, Difference, FileStore, NativeError, NativeFile, NativeKind,
    ViewPalette,
};
use crate::native_toml::{array, Fields};
use crate::png;

// ---- pal.dat → JASC-PAL --------------------------------------------------

fn pal_path(p: &str) -> String {
    format!("{p}.pal")
}

impl NativeKind for Palette {
    const KIND: &'static str = "pal";

    fn write(&self, p: &str, _view: &ViewPalette) -> Result<Vec<NativeFile>, NativeError> {
        let mut t = String::from("JASC-PAL\n0100\n256\n");
        for c in &self.colors {
            t += &format!("{} {} {}\n", c.r, c.g, c.b);
        }
        Ok(vec![NativeFile {
            path: pal_path(p),
            bytes: t.into_bytes(),
        }])
    }

    fn read(p: &str, store: &dyn FileStore) -> Result<Self, NativeError> {
        let path = pal_path(p);
        let text = String::from_utf8(need(store, &path)?)
            .map_err(|_| NativeError::new(&path, "not UTF-8"))?;
        let err = |m: String| NativeError::new(&path, m);
        // Editors on Windows write CRLF; a final newline is optional.
        let mut lines = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l));
        for (i, want) in ["JASC-PAL", "0100", "256"].into_iter().enumerate() {
            match lines.next() {
                Some(l) if l == want => {}
                other => {
                    return Err(err(format!(
                        "line {}: expected {want:?}, got {other:?}",
                        i + 1
                    )))
                }
            }
        }
        let mut colors = [Rgb::default(); 256];
        for (i, c) in colors.iter_mut().enumerate() {
            let line = lines
                .next()
                .ok_or_else(|| err(format!("line {}: missing colour {i}", i + 4)))?;
            let v: Vec<u8> = line
                .split(' ')
                .map(|s| {
                    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
                        None
                    } else {
                        s.parse().ok()
                    }
                })
                .collect::<Option<_>>()
                .ok_or_else(|| {
                    err(format!(
                        "line {}: expected `R G B` (0-255), got {line:?}",
                        i + 4
                    ))
                })?;
            let [r, g, b] = v[..] else {
                return Err(err(format!(
                    "line {}: expected `R G B`, got {line:?}",
                    i + 4
                )));
            };
            *c = Rgb { r, g, b };
        }
        if lines.any(|l| !l.is_empty()) {
            return Err(err("data after the 256 colours".into()));
        }
        Ok(Palette { colors })
    }

    fn first_difference(&self, native: &Self, p: &str) -> Option<Difference> {
        let i = self
            .colors
            .iter()
            .zip(&native.colors)
            .position(|(a, b)| a != b)?;
        Some(Difference {
            file: pal_path(p),
            detail: format!("colour {i}: {:?} != {:?}", self.colors[i], native.colors[i]),
        })
    }
}

// ---- PL2 -----------------------------------------------------------------

fn toml_path(p: &str) -> String {
    format!("{p}.toml")
}
fn png_path(p: &str) -> String {
    format!("{p}.png")
}

fn rgb_text(c: Rgb) -> String {
    format!("\"#{:02x}{:02x}{:02x}\"", c.r, c.g, c.b)
}

fn parse_rgb(s: &str) -> Option<Rgb> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6
        || !h
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(Rgb {
        r: (v >> 16) as u8,
        g: (v >> 8) as u8,
        b: v as u8,
    })
}

/// The map groups of a `Pl2` in PNG row order: `(name, first row, rows)`.
/// The counts are fixed by the format except the text colour shifts, one
/// per text colour (`palette.md`).
fn groups(text_colors: usize) -> Vec<(&'static str, usize, usize)> {
    let counts = [
        ("light_levels", 32),
        ("inventory_variations", 16),
        ("selected_unit_shift", 1),
        ("alpha_blend", 3 * 256),
        ("additive_blend", 256),
        ("multiplicative_blend", 256),
        ("hue_variations", 111),
        ("red_tones", 1),
        ("green_tones", 1),
        ("blue_tones", 1),
        ("unknown_variations", 14),
        ("max_component_blend", 256),
        ("darkened_shift", 1),
        ("text_color_shifts", text_colors),
    ];
    let mut at = 0;
    counts
        .into_iter()
        .map(|(n, c)| {
            let g = (n, at, c);
            at += c;
            g
        })
        .collect()
}

fn rows_of(pl: &Pl2) -> Vec<&ColorMap> {
    let mut v: Vec<&ColorMap> = Vec::new();
    v.extend(&pl.light_levels);
    v.extend(&pl.inventory_variations);
    v.push(&pl.selected_unit_shift);
    v.extend(pl.alpha_blend.iter().flatten());
    v.extend(&pl.additive_blend);
    v.extend(&pl.multiplicative_blend);
    v.extend(&pl.hue_variations);
    v.push(&pl.red_tones);
    v.push(&pl.green_tones);
    v.push(&pl.blue_tones);
    v.extend(&pl.unknown_variations);
    v.extend(&pl.max_component_blend);
    v.push(&pl.darkened_shift);
    v.extend(&pl.text_color_shifts);
    v
}

fn base_view(pl: &Pl2) -> ViewPalette {
    pl.base_palette.colors
}

impl NativeKind for Pl2 {
    const KIND: &'static str = "pl2";

    fn write(&self, p: &str, _view: &ViewPalette) -> Result<Vec<NativeFile>, NativeError> {
        let (tp, pp) = (toml_path(p), png_path(p));
        if self.text_colors.len() != self.text_color_shifts.len() {
            return Err(NativeError::new(
                &tp,
                "text_colors and text_color_shifts differ in length",
            ));
        }
        let g = groups(self.text_colors.len());
        let rows = rows_of(self);
        let total = g.last().map_or(0, |x| x.1 + x.2);
        let ok = self.light_levels.len() == 32
            && self.inventory_variations.len() == 16
            && self.alpha_blend.len() == 3
            && self.alpha_blend.iter().all(|l| l.len() == 256)
            && self.additive_blend.len() == 256
            && self.multiplicative_blend.len() == 256
            && self.hue_variations.len() == 111
            && self.unknown_variations.len() == 14
            && self.max_component_blend.len() == 256
            && rows.len() == total;
        if !ok {
            return Err(NativeError::new(
                &tp,
                "map group sizes differ from the format",
            ));
        }
        let img: Vec<u8> = rows.iter().flat_map(|m| m.iter().copied()).collect();
        let bytes = png::write_indexed(&pp, 256, total as u32, &img, &base_view(self))?;
        let mut t = String::from("native = \"pl2\"\nnative_version = 1\n");
        t += &format!(
            "base_palette = {}\n",
            array(self.base_palette.colors.iter().map(|&c| rgb_text(c)))
        );
        t += &format!(
            "text_colors = {}\n",
            array(self.text_colors.iter().map(|&c| rgb_text(c)))
        );
        for (name, start, count) in g {
            t += &format!("\n[[group]]\nname = \"{name}\"\nstart = {start}\ncount = {count}\n");
        }
        Ok(vec![
            NativeFile { path: pp, bytes },
            NativeFile {
                path: tp,
                bytes: t.into_bytes(),
            },
        ])
    }

    fn read(p: &str, store: &dyn FileStore) -> Result<Self, NativeError> {
        let (tp, pp) = (toml_path(p), png_path(p));
        let text =
            String::from_utf8(need(store, &tp)?).map_err(|_| NativeError::new(&tp, "not UTF-8"))?;
        let mut f = Fields::parse(&tp, &text)?;
        f.header("pl2", 1)?;
        let rgbs = |f: &mut Fields, key: &str| -> Result<Vec<Rgb>, NativeError> {
            f.strs(key)?
                .iter()
                .map(|s| {
                    parse_rgb(s).ok_or_else(|| {
                        f.err(format!(
                            "`{key}` entry {s:?}: expected \"#rrggbb\" lowercase"
                        ))
                    })
                })
                .collect()
        };
        let base = rgbs(&mut f, "base_palette")?;
        let text_colors = rgbs(&mut f, "text_colors")?;
        let Ok(colors) = <[Rgb; 256]>::try_from(base) else {
            return Err(f.err("`base_palette` must have 256 entries"));
        };
        let want = groups(text_colors.len());
        let mut got = f.tables("group")?;
        if got.len() != want.len() {
            return Err(f.err(format!("{} groups, expected {}", got.len(), want.len())));
        }
        for (g, &(name, start, count)) in got.iter_mut().zip(&want) {
            let n = g.str("name")?;
            let (s, c) = (g.int("start")?, g.int("count")?);
            if n != name || s != start as i64 || c != count as i64 {
                return Err(g.err(format!(
                    "group {n:?} {s}+{c}, expected {name:?} {start}+{count}"
                )));
            }
            g.finish()?;
        }
        f.finish()?;
        let total = want.last().map_or(0, |x| x.1 + x.2);
        let (w, h, img) = png::read_indexed(&pp, &need(store, &pp)?)?;
        if (w, h) != (256, total as u32) {
            return Err(NativeError::new(
                &pp,
                format!("image is {w} x {h}, expected 256 x {total}"),
            ));
        }
        let mut maps = img.as_chunks::<256>().0.iter().map(|c| {
            let mut m = [0u8; 256];
            m.copy_from_slice(c);
            m
        });
        let mut take = |n: usize| -> Vec<ColorMap> { maps.by_ref().take(n).collect() };
        let one = |v: Vec<ColorMap>| v[0];
        let light_levels = take(32);
        let inventory_variations = take(16);
        let selected_unit_shift = one(take(1));
        let alpha_blend = (0..3).map(|_| take(256)).collect();
        let additive_blend = take(256);
        let multiplicative_blend = take(256);
        let hue_variations = take(111);
        let red_tones = one(take(1));
        let green_tones = one(take(1));
        let blue_tones = one(take(1));
        let unknown_variations = take(14);
        let max_component_blend = take(256);
        let darkened_shift = one(take(1));
        let text_color_shifts = take(text_colors.len());
        Ok(Pl2 {
            base_palette: Palette { colors },
            light_levels,
            inventory_variations,
            selected_unit_shift,
            alpha_blend,
            additive_blend,
            multiplicative_blend,
            hue_variations,
            red_tones,
            green_tones,
            blue_tones,
            unknown_variations,
            max_component_blend,
            darkened_shift,
            text_colors,
            text_color_shifts,
        })
    }

    fn first_difference(&self, native: &Self, p: &str) -> Option<Difference> {
        let tp = toml_path(p);
        diff_fields!(tp, "pl2", self, native, [base_palette, text_colors]);
        let (a, b) = (rows_of(self), rows_of(native));
        if a.len() != b.len() {
            return Some(Difference {
                file: tp,
                detail: format!("{} maps != {}", a.len(), b.len()),
            });
        }
        let g = groups(self.text_colors.len());
        for (row, (x, y)) in a.iter().zip(&b).enumerate() {
            if let Some(d) = pixel_diff(&x[..], &y[..], 256) {
                let (name, start, _) = *g.iter().rev().find(|g| g.1 <= row).unwrap_or(&("?", 0, 0));
                return Some(Difference {
                    file: png_path(p),
                    detail: format!("row {row} ({name} map {}) {d}", row - start),
                });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::{check_files, grey_palette, round_trip, CheckError, MemStore};

    fn palette() -> Palette {
        let mut colors = [Rgb::default(); 256];
        for (i, c) in colors.iter_mut().enumerate() {
            *c = Rgb {
                r: i as u8,
                g: (i * 7) as u8,
                b: 255 - i as u8,
            };
        }
        Palette { colors }
    }

    fn pl2(text: usize) -> Pl2 {
        let map = |seed: usize| -> ColorMap { std::array::from_fn(|i| (i * 5 + seed) as u8) };
        let maps = |n: usize, s: usize| -> Vec<ColorMap> { (0..n).map(|i| map(s + i)).collect() };
        Pl2 {
            base_palette: palette(),
            light_levels: maps(32, 1),
            inventory_variations: maps(16, 2),
            selected_unit_shift: map(3),
            alpha_blend: (0..3).map(|l| maps(256, 10 + l)).collect(),
            additive_blend: maps(256, 4),
            multiplicative_blend: maps(256, 5),
            hue_variations: maps(111, 6),
            red_tones: map(7),
            green_tones: map(8),
            blue_tones: map(9),
            unknown_variations: maps(14, 11),
            max_component_blend: maps(256, 12),
            darkened_shift: map(13),
            text_colors: (0..text)
                .map(|i| Rgb {
                    r: i as u8,
                    g: 2,
                    b: 3,
                })
                .collect(),
            text_color_shifts: maps(text, 14),
        }
    }

    #[test]
    fn pal_is_jasc_text_and_round_trips() {
        let p = "data/global/palette/act1/pal.dat";
        let files = round_trip(&palette(), p, &grey_palette()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, format!("{p}.pal"));
        let t = String::from_utf8(files[0].bytes.clone()).unwrap();
        assert!(
            t.starts_with("JASC-PAL\n0100\n256\n0 0 255\n1 7 254\n"),
            "{t}"
        );
        assert_eq!(t.lines().count(), 259);
        // CRLF from a Windows editor and a missing final newline are fine
        let crlf = t.replace('\n', "\r\n");
        let mut f = files.clone();
        f[0].bytes = crlf.trim_end().as_bytes().to_vec();
        assert_eq!(
            Palette::read(p, &MemStore::from_files(&f)).unwrap(),
            palette()
        );
    }

    #[test]
    fn pal_strictness_and_perturbation() {
        let p = "pal.dat";
        let files = palette().write(p, &grey_palette()).unwrap();
        let bad = |from: &str, to: &str| {
            let mut f = files.clone();
            f[0].bytes = String::from_utf8(f[0].bytes.clone())
                .unwrap()
                .replacen(from, to, 1)
                .into_bytes();
            f
        };
        for (from, to) in [
            ("0100", "0200"),
            ("256\n0 0 255", "256\n0 0 256"),
            ("1 7 254", "1 7"),
            ("1 7 254", "1  7 254"),
        ] {
            let e = Palette::read(p, &MemStore::from_files(&bad(from, to))).unwrap_err();
            assert_eq!(e.file, "pal.dat.pal", "{from}");
        }
        let CheckError::Mismatch(d) =
            check_files(&palette(), p, &bad("1 7 254", "1 7 253")).unwrap_err()
        else {
            panic!()
        };
        assert_eq!(
            (d.file.as_str(), d.detail.starts_with("colour 1:")),
            ("pal.dat.pal", true)
        );
    }

    #[test]
    fn pl2_rows_groups_and_plte() {
        let p = "data/global/palette/act1/pal.pl2";
        let d = pl2(3);
        let files = round_trip(&d, p, &grey_palette()).unwrap();
        let png = files.iter().find(|f| f.path.ends_with(".png")).unwrap();
        let (w, h, px) = png::read_indexed("x", &png.bytes).unwrap();
        assert_eq!((w, h), (256, 1714 + 3));
        // row 0 = light_levels[0]; the last row = text_color_shifts[2]
        assert_eq!(&px[..256], &d.light_levels[0][..]);
        assert_eq!(&px[px.len() - 256..], &d.text_color_shifts[2][..]);
        // PLTE = base_palette
        let b = &png.bytes;
        let at = b.windows(4).position(|w| w == b"PLTE").unwrap();
        assert_eq!(&b[at + 4..at + 7], &[0, 0, 255]);
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
            t.contains("name = \"alpha_blend\"\nstart = 49\ncount = 768"),
            "{t}"
        );
        assert!(t.contains("\"#0000ff\"") && t.contains("\"#010203\""));
        round_trip(&pl2(0), p, &grey_palette()).unwrap();
    }

    #[test]
    fn pl2_perturbation_and_strictness() {
        let p = "act1.pl2";
        let d = pl2(2);
        let files = d.write(p, &grey_palette()).unwrap();

        let mut f = files.clone();
        let img = f.iter_mut().find(|f| f.path.ends_with(".png")).unwrap();
        let (w, h, mut px) = png::read_indexed("x", &img.bytes).unwrap();
        // row 49 + 256 + 4 is alpha_blend[1][4]? rows: alpha starts at 49 -> level 0 has 256 rows
        let row = 49 + 256 + 4;
        px[row * 256 + 9] ^= 1;
        img.bytes = png::write_indexed("x", w, h, &px, &grey_palette()).unwrap();
        let CheckError::Mismatch(diff) = check_files(&d, p, &f).unwrap_err() else {
            panic!()
        };
        assert_eq!(diff.file, "act1.pl2.png");
        assert!(
            diff.detail
                .starts_with("row 309 (alpha_blend map 260) pixel (9, 0)"),
            "{diff}"
        );

        let mut f = files.clone();
        let t = f.iter_mut().find(|f| f.path.ends_with(".toml")).unwrap();
        t.bytes = String::from_utf8(t.bytes.clone())
            .unwrap()
            .replacen("\"#0000ff\"", "\"#0000fG\"", 1)
            .into_bytes();
        assert!(Pl2::read(p, &MemStore::from_files(&f))
            .unwrap_err()
            .message
            .contains("#rrggbb"));

        let mut f = files.clone();
        let t = f.iter_mut().find(|f| f.path.ends_with(".toml")).unwrap();
        t.bytes = String::from_utf8(t.bytes.clone())
            .unwrap()
            .replacen("count = 32", "count = 31", 1)
            .into_bytes();
        assert!(Pl2::read(p, &MemStore::from_files(&f))
            .unwrap_err()
            .message
            .contains("light_levels"));
    }
}
