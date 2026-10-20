//! DT1 tile sets in the native format.
//!
//! A tile set holds the floor, wall, roof and shadow tiles a level is built
//! from. Each tile is made of small blocks: 32 × 15 diamond blocks for
//! floors and 32 × 32 run-length blocks for upright parts, each placed at
//! its own offset inside the tile.
//!
//! The native form is `P.toml`, with every tile and block field, and one
//! indexed PNG per tile at `P.d/<n>.png`. Normally the PNG is the tile as
//! it looks: the bounding box of all its blocks drawn in place
//! (`layout = "assembled"`). When that picture would not read back to the
//! very same blocks (blocks that overlap, or a tile spread too wide), the
//! PNG is a strip instead, block `k` in the 32 × 32 cell at `(0, 32k)`
//! (`layout = "blocks"`).

use d2_formats::dt1::{Dt1, Dt1Block, Dt1Tile, ISO_HEIGHT, ISO_WIDTH, RLE_HEIGHT, RLE_WIDTH};

use crate::kind::{
    diff_fields, need, pixel_diff, Difference, FileStore, NativeError, NativeFile, NativeKind,
    ViewPalette,
};
use crate::native_toml::{array, hex2, hex4, Fields};
use crate::png;

/// The shape of a diamond (floor) block, row by row: how many pixels each
/// of its 15 rows skips from the left and how many it then draws.
const ISO_SKIP: [usize; ISO_HEIGHT] = [14, 12, 10, 8, 6, 4, 2, 0, 2, 4, 6, 8, 10, 12, 14];
const ISO_RUN: [usize; ISO_HEIGHT] = [4, 8, 12, 16, 20, 24, 28, 32, 28, 24, 20, 16, 12, 8, 4];

/// Largest assembled image written; a tile spread wider takes the strip.
/// Real tiles span a few hundred pixels, so this only guards against
/// hostile input.
const MAX_ASSEMBLED_PIXELS: u64 = 1 << 24;

fn toml_path(p: &str) -> String {
    format!("{p}.toml")
}
fn tile_png(p: &str, n: usize) -> String {
    format!("{p}.d/{n}.png")
}

fn block_size(b: &Dt1Block) -> (usize, usize) {
    b.size()
}

fn check_block(file: &str, n: usize, k: usize, b: &Dt1Block) -> Result<(), NativeError> {
    let (w, h) = block_size(b);
    if b.pixels.len() != w * h {
        return Err(NativeError::new(
            file,
            format!(
                "tile {n} block {k}: {} pixels, format {} needs {}",
                b.pixels.len(),
                b.format,
                w * h
            ),
        ));
    }
    Ok(())
}

/// Draws a block at `(ox, oy)`: the diamond for format 1, the non-zero
/// pixels otherwise.
fn draw(img: &mut [u8], iw: usize, ox: usize, oy: usize, b: &Dt1Block) {
    if b.is_iso() {
        for row in 0..ISO_HEIGHT {
            let (s, r) = (ISO_SKIP[row], ISO_RUN[row]);
            let dst = (oy + row) * iw + ox + s;
            img[dst..dst + r].copy_from_slice(&b.pixels[row * ISO_WIDTH + s..][..r]);
        }
    } else {
        for row in 0..RLE_HEIGHT {
            for c in 0..RLE_WIDTH {
                let v = b.pixels[row * RLE_WIDTH + c];
                if v != 0 {
                    img[(oy + row) * iw + ox + c] = v;
                }
            }
        }
    }
}

/// The block's pixels taken from the same rectangle and shape.
fn undraw(img: &[u8], iw: usize, ox: usize, oy: usize, iso: bool) -> Vec<u8> {
    if iso {
        let mut px = vec![0u8; ISO_WIDTH * ISO_HEIGHT];
        for row in 0..ISO_HEIGHT {
            let (s, r) = (ISO_SKIP[row], ISO_RUN[row]);
            let src = (oy + row) * iw + ox + s;
            px[row * ISO_WIDTH + s..][..r].copy_from_slice(&img[src..src + r]);
        }
        px
    } else {
        let mut px = Vec::with_capacity(RLE_WIDTH * RLE_HEIGHT);
        for row in 0..RLE_HEIGHT {
            let at = (oy + row) * iw + ox;
            px.extend_from_slice(&img[at..at + RLE_WIDTH]);
        }
        px
    }
}

/// Where a tile's blocks sit in its image.
enum Plan {
    Assembled { origin: (i32, i32) },
    Strip,
}

/// `(origin, width, height)` of the assembled bounding box.
fn bounding_box(blocks: &[Dt1Block]) -> ((i32, i32), u32, u32) {
    let x0 = blocks.iter().map(|b| i32::from(b.x)).min().unwrap_or(0);
    let y0 = blocks.iter().map(|b| i32::from(b.y)).min().unwrap_or(0);
    let right = blocks
        .iter()
        .map(|b| i32::from(b.x) + RLE_WIDTH as i32)
        .max()
        .unwrap_or(0);
    let bottom = blocks
        .iter()
        .map(|b| i32::from(b.y) + block_size(b).1 as i32)
        .max()
        .unwrap_or(0);
    ((x0, y0), (right - x0) as u32, (bottom - y0) as u32)
}

fn positions(plan: &Plan, blocks: &[Dt1Block]) -> Vec<(usize, usize)> {
    match plan {
        Plan::Assembled { origin } => blocks
            .iter()
            .map(|b| {
                (
                    (i32::from(b.x) - origin.0) as usize,
                    (i32::from(b.y) - origin.1) as usize,
                )
            })
            .collect(),
        Plan::Strip => (0..blocks.len()).map(|k| (0, RLE_HEIGHT * k)).collect(),
    }
}

fn dims(plan: &Plan, blocks: &[Dt1Block]) -> (u32, u32) {
    match plan {
        Plan::Assembled { .. } => {
            let (_, w, h) = bounding_box(blocks);
            (w, h)
        }
        Plan::Strip => (RLE_WIDTH as u32, (RLE_HEIGHT * blocks.len()) as u32),
    }
}

/// Draws the blocks and checks the image reads back to them.
fn try_plan(plan: &Plan, blocks: &[Dt1Block]) -> Option<(u32, u32, Vec<u8>)> {
    let (w, h) = dims(plan, blocks);
    if u64::from(w) * u64::from(h) > MAX_ASSEMBLED_PIXELS {
        return None;
    }
    let mut img = vec![0u8; w as usize * h as usize];
    let pos = positions(plan, blocks);
    for (b, &(x, y)) in blocks.iter().zip(&pos) {
        draw(&mut img, w as usize, x, y, b);
    }
    let ok = blocks
        .iter()
        .zip(&pos)
        .all(|(b, &(x, y))| undraw(&img, w as usize, x, y, b.is_iso()) == b.pixels);
    ok.then_some((w, h, img))
}

impl NativeKind for Dt1 {
    const KIND: &'static str = "dt1";

    fn write(&self, p: &str, view: &ViewPalette) -> Result<Vec<NativeFile>, NativeError> {
        let tp = toml_path(p);
        let mut out = Vec::new();
        let mut t = String::new();
        t += "native = \"dt1\"\nnative_version = 1\n";
        t += &format!(
            "version = {}\nminor_version = {}\n",
            self.version, self.minor_version
        );
        for (n, tile) in self.tiles.iter().enumerate() {
            for (k, b) in tile.blocks.iter().enumerate() {
                check_block(&tp, n, k, b)?;
            }
            t += "\n[[tile]]\n";
            t += &format!("light_direction = {}\n", tile.light_direction);
            t += &format!("roof_height = {}\n", tile.roof_height);
            t += &format!("material_flags = {}\n", hex4(tile.material_flags));
            t += &format!("height = {}\nwidth = {}\n", tile.height, tile.width);
            t += &format!("unknown_height = {}\n", tile.unknown_height);
            t += &format!("orientation = {}\n", tile.orientation);
            t += &format!(
                "main_index = {}\nsub_index = {}\n",
                tile.main_index, tile.sub_index
            );
            t += &format!("rarity = {}\n", tile.rarity);
            t += &format!("unknown_color = {}\n", tile.unknown_color);
            t += &format!(
                "subtile_flags = {}\n",
                array(tile.subtile_flags.iter().map(|&f| hex2(f)))
            );
            t += &format!("unknown_58 = {}\n", tile.unknown_58);
            t += &format!("cache_index = {}\n", tile.cache_index);
            t += &format!("unknown_5c = {}\n", tile.unknown_5c);
            if !tile.blocks.is_empty() {
                let assembled = Plan::Assembled {
                    origin: bounding_box(&tile.blocks).0,
                };
                let (plan, name, (w, h, img)) = if let Some(r) = try_plan(&assembled, &tile.blocks)
                {
                    (assembled, "assembled", r)
                } else if let Some(r) = try_plan(&Plan::Strip, &tile.blocks) {
                    (Plan::Strip, "blocks", r)
                } else {
                    return Err(NativeError::new(
                        tile_png(p, n),
                        format!(
                            "tile {n}: block pixels are not representable (pixels outside the block shape)"
                        ),
                    ));
                };
                t += &format!("image = \"{n}.png\"\nlayout = \"{name}\"\n");
                if let Plan::Assembled { origin } = plan {
                    t += &format!("origin = [{}, {}]\n", origin.0, origin.1);
                }
                let path = tile_png(p, n);
                let bytes = png::write_indexed(&path, w, h, &img, view)?;
                out.push(NativeFile { path, bytes });
            }
            for b in &tile.blocks {
                t += "\n[[tile.block]]\n";
                t += &format!("x = {}\ny = {}\n", b.x, b.y);
                t += &format!("unknown1 = {}\n", b.unknown1);
                t += &format!("grid_x = {}\ngrid_y = {}\n", b.grid_x, b.grid_y);
                t += &format!("format = {}\nunknown2 = {}\n", b.format, b.unknown2);
            }
        }
        out.push(NativeFile {
            path: tp,
            bytes: t.into_bytes(),
        });
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }

    fn read(p: &str, store: &dyn FileStore) -> Result<Self, NativeError> {
        let tp = toml_path(p);
        let text =
            String::from_utf8(need(store, &tp)?).map_err(|_| NativeError::new(&tp, "not UTF-8"))?;
        let mut f = Fields::parse(&tp, &text)?;
        f.header("dt1", 1)?;
        let version = f.i32("version")?;
        let minor_version = f.i32("minor_version")?;
        let mut tiles = Vec::new();
        for (n, t) in f.tables("tile")?.iter_mut().enumerate() {
            let light_direction = t.u32("light_direction")?;
            let roof_height = t.u16("roof_height")?;
            let material_flags = t.hex_u16("material_flags")?;
            let height = t.i32("height")?;
            let width = t.i32("width")?;
            let unknown_height = t.i32("unknown_height")?;
            let orientation = t.u32("orientation")?;
            let main_index = t.u32("main_index")?;
            let sub_index = t.u32("sub_index")?;
            let rarity = t.u32("rarity")?;
            let unknown_color = t.u32("unknown_color")?;
            let flags = t.strs("subtile_flags")?;
            if flags.len() != 25 {
                return Err(t.err("`subtile_flags` must have 25 entries"));
            }
            let mut subtile_flags = [0u8; 25];
            for (o, s) in subtile_flags.iter_mut().zip(&flags) {
                *o = match crate::native_toml::parse_hex(s, 2) {
                    Some(v) => v as u8,
                    None => {
                        return Err(t.err(format!(
                        "`subtile_flags` entry {s:?}: expected \"0x\" and 2 lowercase hex digits"
                    )))
                    }
                };
            }
            let unknown_58 = t.u16("unknown_58")?;
            let cache_index = t.u16("cache_index")?;
            let unknown_5c = t.u32("unknown_5c")?;
            let mut blocks = Vec::new();
            let mut raw = t.tables("block")?;
            for b in raw.iter_mut() {
                blocks.push(Dt1Block {
                    x: b.i16("x")?,
                    y: b.i16("y")?,
                    unknown1: b.u16("unknown1")?,
                    grid_x: b.u8("grid_x")?,
                    grid_y: b.u8("grid_y")?,
                    format: b.u16("format")?,
                    unknown2: b.u16("unknown2")?,
                    pixels: Vec::new(),
                });
                b.finish()?;
            }
            if blocks.is_empty() {
                for key in ["image", "layout", "origin"] {
                    if t.has(key) {
                        return Err(t.err(format!("tile {n} has no blocks but a `{key}`")));
                    }
                }
            } else {
                let image = t.str("image")?;
                if image != format!("{n}.png") {
                    return Err(t.err(format!("tile {n}: image {image:?}, expected \"{n}.png\"")));
                }
                let layout = t.str("layout")?;
                let plan = match layout.as_str() {
                    "assembled" => {
                        let o = t.ints("origin")?;
                        let want = bounding_box(&blocks).0;
                        if o != [i64::from(want.0), i64::from(want.1)] {
                            return Err(t.err(format!(
                                "tile {n}: origin {o:?} does not match the blocks ({}, {})",
                                want.0, want.1
                            )));
                        }
                        Plan::Assembled { origin: want }
                    }
                    "blocks" => Plan::Strip,
                    other => return Err(t.err(format!("tile {n}: unknown layout {other:?}"))),
                };
                let path = tile_png(p, n);
                let (w, h, img) = png::read_indexed(&path, &need(store, &path)?)?;
                if (w, h) != dims(&plan, &blocks) {
                    return Err(NativeError::new(
                        &path,
                        format!(
                            "image is {w} x {h}, the sidecar's blocks need {} x {}",
                            dims(&plan, &blocks).0,
                            dims(&plan, &blocks).1
                        ),
                    ));
                }
                let pos = positions(&plan, &blocks);
                for (b, &(x, y)) in blocks.iter_mut().zip(&pos) {
                    b.pixels = undraw(&img, w as usize, x, y, b.is_iso());
                }
            }
            t.finish()?;
            tiles.push(Dt1Tile {
                light_direction,
                roof_height,
                material_flags,
                height,
                width,
                unknown_height,
                orientation,
                main_index,
                sub_index,
                rarity,
                unknown_color,
                subtile_flags,
                unknown_58,
                cache_index,
                unknown_5c,
                blocks,
            });
        }
        f.finish()?;
        Ok(Dt1 {
            version,
            minor_version,
            tiles,
        })
    }

    fn first_difference(&self, native: &Self, p: &str) -> Option<Difference> {
        let tp = toml_path(p);
        diff_fields!(tp, "file", self, native, [version, minor_version]);
        if self.tiles.len() != native.tiles.len() {
            return Some(Difference {
                file: tp,
                detail: format!("{} tiles != {}", self.tiles.len(), native.tiles.len()),
            });
        }
        for (n, (a, b)) in self.tiles.iter().zip(&native.tiles).enumerate() {
            let at = format!("tile {n}");
            diff_fields!(
                tp,
                at,
                a,
                b,
                [
                    light_direction,
                    roof_height,
                    material_flags,
                    height,
                    width,
                    unknown_height,
                    orientation,
                    main_index,
                    sub_index,
                    rarity,
                    unknown_color,
                    subtile_flags,
                    unknown_58,
                    cache_index,
                    unknown_5c
                ]
            );
            if a.blocks.len() != b.blocks.len() {
                return Some(Difference {
                    file: tp,
                    detail: format!("{at}: {} blocks != {}", a.blocks.len(), b.blocks.len()),
                });
            }
            for (k, (ba, bb)) in a.blocks.iter().zip(&b.blocks).enumerate() {
                let at = format!("tile {n} block {k}");
                diff_fields!(
                    tp,
                    at,
                    ba,
                    bb,
                    [x, y, unknown1, grid_x, grid_y, format, unknown2]
                );
                if let Some(d) = pixel_diff(&ba.pixels, &bb.pixels, RLE_WIDTH) {
                    return Some(Difference {
                        file: tile_png(p, n),
                        detail: format!("{at} {d}"),
                    });
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::{check_files, grey_palette, round_trip, CheckError, MemStore};

    const P: &str = "data/global/tiles/act1/town/floor.dt1";

    fn tile(blocks: Vec<Dt1Block>) -> Dt1Tile {
        Dt1Tile {
            light_direction: 2,
            roof_height: 3,
            material_flags: 0x0a0b,
            height: -80,
            width: 160,
            unknown_height: 4,
            orientation: 1,
            main_index: 5,
            sub_index: 6,
            rarity: 7,
            unknown_color: 8,
            subtile_flags: std::array::from_fn(|i| i as u8 * 3),
            unknown_58: 9,
            cache_index: 10,
            unknown_5c: 11,
            blocks,
        }
    }

    fn iso(x: i16, y: i16, seed: u8) -> Dt1Block {
        let mut pixels = vec![0u8; ISO_WIDTH * ISO_HEIGHT];
        let mut k = seed;
        for r in 0..ISO_HEIGHT {
            for c in ISO_SKIP[r]..ISO_SKIP[r] + ISO_RUN[r] {
                pixels[r * ISO_WIDTH + c] = k;
                k = k.wrapping_add(1).max(1);
            }
        }
        block(x, y, 1, pixels)
    }

    fn block(x: i16, y: i16, format: u16, pixels: Vec<u8>) -> Dt1Block {
        Dt1Block {
            x,
            y,
            unknown1: 12,
            grid_x: 1,
            grid_y: 2,
            format,
            unknown2: 13,
            pixels,
        }
    }

    /// An RLE block whose only pixels are `(column, value)` in row 0.
    fn rle(x: i16, y: i16, px: &[(usize, u8)]) -> Dt1Block {
        let mut pixels = vec![0u8; RLE_WIDTH * RLE_HEIGHT];
        for &(c, v) in px {
            pixels[c] = v;
        }
        block(x, y, 0, pixels)
    }

    fn file(tiles: Vec<Dt1Tile>) -> Dt1 {
        Dt1 {
            version: 7,
            minor_version: 6,
            tiles,
        }
    }

    fn text(files: &[NativeFile]) -> String {
        let f = files.iter().find(|f| f.path.ends_with(".toml")).unwrap();
        String::from_utf8(f.bytes.clone()).unwrap()
    }

    fn image(files: &[NativeFile], n: usize) -> (u32, u32, Vec<u8>) {
        let f = files
            .iter()
            .find(|f| f.path == format!("{P}.d/{n}.png"))
            .unwrap();
        png::read_indexed("x", &f.bytes).unwrap()
    }

    #[test]
    fn iso_block_vector_is_32_by_15_with_origin_zero() {
        let d = file(vec![tile(vec![iso(0, 0, 1)])]);
        let files = round_trip(&d, P, &grey_palette()).unwrap();
        let t = text(&files);
        assert!(
            t.contains("layout = \"assembled\"") && t.contains("origin = [0, 0]"),
            "{t}"
        );
        let (w, h, px) = image(&files, 0);
        assert_eq!((w, h), (32, 15));
        assert_eq!(px, d.tiles[0].blocks[0].pixels);
        assert_eq!(px.iter().filter(|&&v| v != 0).count(), 256);
    }

    #[test]
    fn assembled_image_covers_the_bounding_box_of_negative_origins() {
        let d = file(vec![tile(vec![
            rle(-32, -10, &[(0, 7), (31, 8)]),
            iso(0, 20, 1),
        ])]);
        let files = round_trip(&d, P, &grey_palette()).unwrap();
        let t = text(&files);
        assert!(t.contains("origin = [-32, -10]"), "{t}");
        let (w, h, _) = image(&files, 0);
        // right = max(x + 32) = 32; bottom = max(y + h) = max(22, 35)
        assert_eq!((w, h), (64, 45));
    }

    #[test]
    fn overlapping_rle_blocks_take_the_strip_layout() {
        let d = file(vec![tile(vec![
            rle(0, 0, &[(20, 5)]),
            rle(16, 0, &[(4, 6)]),
        ])]);
        let files = round_trip(&d, P, &grey_palette()).unwrap();
        assert!(text(&files).contains("layout = \"blocks\""));
        let (w, h, _) = image(&files, 0);
        assert_eq!((w, h), (32, 64));
        // same index in the shared pixel: not a conflict, reads back, assembled
        let d = file(vec![tile(vec![
            rle(0, 0, &[(20, 5)]),
            rle(16, 0, &[(4, 5)]),
        ])]);
        let files = round_trip(&d, P, &grey_palette()).unwrap();
        assert!(text(&files).contains("layout = \"assembled\""));
        // a neighbour's pixel inside an RLE block's rectangle: strip
        let d = file(vec![tile(vec![
            rle(0, 0, &[(25, 5)]),
            rle(16, 0, &[(0, 6)]),
        ])]);
        let files = round_trip(&d, P, &grey_palette()).unwrap();
        assert!(text(&files).contains("layout = \"blocks\""));
    }

    #[test]
    fn tile_without_blocks_has_no_image_and_empty_file_round_trips() {
        let d = file(vec![tile(vec![]), tile(vec![iso(0, 0, 3)]), tile(vec![])]);
        let files = round_trip(&d, P, &grey_palette()).unwrap();
        let png: Vec<_> = files.iter().filter(|f| f.path.ends_with(".png")).collect();
        assert_eq!(png.len(), 1);
        assert_eq!(png[0].path, format!("{P}.d/1.png"));
        round_trip(&file(vec![]), P, &grey_palette()).unwrap();
    }

    #[test]
    fn disjoint_rle_blocks_assemble() {
        let d = file(vec![tile(vec![
            rle(0, 0, &[(1, 5)]),
            rle(32, 0, &[(2, 6)]),
        ])]);
        let files = round_trip(&d, P, &grey_palette()).unwrap();
        assert!(text(&files).contains("layout = \"assembled\""));
        assert_eq!(image(&files, 0).0, 64);
    }

    #[test]
    fn pixels_outside_the_block_shape_are_refused() {
        let mut b = iso(0, 0, 1);
        b.pixels[0] = 9; // outside the diamond
        let e = file(vec![tile(vec![b])])
            .write(P, &grey_palette())
            .unwrap_err();
        assert!(e.message.contains("not representable"), "{e}");
        let mut b = iso(0, 0, 1);
        b.pixels.pop();
        assert!(file(vec![tile(vec![b])]).write(P, &grey_palette()).is_err());
    }

    #[test]
    fn writing_twice_gives_identical_bytes() {
        let d = file(vec![tile(vec![iso(0, 0, 1), rle(32, 0, &[(3, 4)])])]);
        assert_eq!(
            d.write(P, &grey_palette()).unwrap(),
            d.write(P, &grey_palette()).unwrap()
        );
    }

    #[test]
    fn perturbations_name_tile_block_pixel_and_fields() {
        let d = file(vec![
            tile(vec![iso(0, 0, 1)]),
            tile(vec![iso(0, 0, 1), iso(0, 15, 2)]),
        ]);
        let files = d.write(P, &grey_palette()).unwrap();

        // pixel (14, 0) is in the diamond of row 0 (skip 14); block 1 of tile 1 is at y = 15
        let mut f = files.clone();
        let img = f
            .iter_mut()
            .find(|f| f.path == format!("{P}.d/1.png"))
            .unwrap();
        let (w, h, mut px) = png::read_indexed("x", &img.bytes).unwrap();
        px[15 * w as usize + 14] ^= 0x40;
        img.bytes = png::write_indexed("x", w, h, &px, &grey_palette()).unwrap();
        let CheckError::Mismatch(diff) = check_files(&d, P, &f).unwrap_err() else {
            panic!()
        };
        assert_eq!(diff.file, format!("{P}.d/1.png"));
        assert!(
            diff.detail
                .starts_with("tile 1 block 1 pixel (14, 0): index 2 != "),
            "{diff}"
        );

        let mut f = files.clone();
        let t = f.iter_mut().find(|f| f.path.ends_with(".toml")).unwrap();
        t.bytes = String::from_utf8(t.bytes.clone())
            .unwrap()
            .replacen("rarity = 7", "rarity = 70", 1)
            .into_bytes();
        let CheckError::Mismatch(diff) = check_files(&d, P, &f).unwrap_err() else {
            panic!()
        };
        assert_eq!(diff.file, format!("{P}.toml"));
        assert!(diff.detail.starts_with("tile 0: rarity 7 != 70"), "{diff}");
    }

    #[test]
    fn strict_reader_rejects_bad_spellings_and_mismatched_images() {
        let d = file(vec![tile(vec![iso(0, 0, 1)])]);
        let files = d.write(P, &grey_palette()).unwrap();
        let with = |from: &str, to: &str| {
            let mut f = files.clone();
            let t = f.iter_mut().find(|f| f.path.ends_with(".toml")).unwrap();
            t.bytes = String::from_utf8(t.bytes.clone())
                .unwrap()
                .replacen(from, to, 1)
                .into_bytes();
            Dt1::read(P, &MemStore::from_files(&f)).unwrap_err()
        };
        assert!(with("\"0x0a0b\"", "\"0x0A0B\"")
            .message
            .contains("material_flags"));
        assert!(with("\"0x00\"", "\"0\"").message.contains("subtile_flags"));
        assert!(with("origin = [0, 0]", "origin = [1, 0]")
            .message
            .contains("origin"));
        assert!(with("layout = \"assembled\"", "layout = \"blocks\"")
            .message
            .contains("image is"));
        let mut f = files.clone();
        f.retain(|f| f.path.ends_with(".toml"));
        let e = Dt1::read(P, &MemStore::from_files(&f)).unwrap_err();
        assert_eq!(e.file, format!("{P}.d/0.png"));
    }
}
