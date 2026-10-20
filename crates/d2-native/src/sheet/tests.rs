//! Sheet layout, round trips, perturbations and strict-reader checks of
//! the DC6 and DCC sprite kinds.

use d2_formats::dc6::{Dc6, Dc6Frame, Dc6Header};
use d2_formats::dcc::{Dcc, DccDirection, DccFrame};

use crate::kind::{
    check_files, grey_palette, round_trip, CheckError, MemStore, NativeFile, NativeKind,
};
use crate::png;

const P: &str = "data/global/ui/test.dc6";

fn frame(w: u32, h: u32, seed: u8) -> Dc6Frame {
    Dc6Frame {
        flip: 0,
        width: w,
        height: h,
        offset_x: -3,
        offset_y: 7,
        unknown: 1,
        next_block: 99,
        pixels: (0..w * h).map(|i| (i as u8).wrapping_add(seed)).collect(),
    }
}

fn dc6(sizes: &[(u32, u32)], directions: u32) -> Dc6 {
    let n = sizes.len() as u32 / directions;
    Dc6 {
        header: Dc6Header {
            version: 6,
            flags: 1,
            encoding: 0,
            termination: [0xee; 4],
            directions,
            frames_per_direction: n,
        },
        frames: sizes
            .iter()
            .enumerate()
            .map(|(i, &(w, h))| frame(w, h, i as u8 * 40 + 1))
            .collect(),
    }
}

fn vector() -> Dc6 {
    dc6(&[(3, 2), (0, 0), (4, 1), (2, 3)], 2)
}

fn toml_of(files: &[NativeFile]) -> String {
    let f = files.iter().find(|f| f.path.ends_with(".toml")).unwrap();
    String::from_utf8(f.bytes.clone()).unwrap()
}

#[test]
fn dc6_test_vector_layout_and_round_trip() {
    let d = vector();
    let files = round_trip(&d, P, &grey_palette()).unwrap();
    let png = files.iter().find(|f| f.path == format!("{P}.png")).unwrap();
    let (w, h, _) = png::read_indexed("x", &png.bytes).unwrap();
    assert_eq!((w, h), (6, 5));
    let t = toml_of(&files);
    for r in [
        "[0, 0, 3, 2]",
        "[3, 0, 0, 0]",
        "[0, 2, 4, 1]",
        "[4, 2, 2, 3]",
    ] {
        assert!(t.contains(&format!("rect = {r}")), "{r} in\n{t}");
    }
    assert!(t.contains("[[direction.frame]]"));
}

#[test]
fn png_is_indexed_8bit_with_trns_and_plte() {
    let files = vector().write(P, &grey_palette()).unwrap();
    let png = files.iter().find(|f| f.path.ends_with(".png")).unwrap();
    let dec = png::read_indexed("x", &png.bytes).unwrap();
    assert_eq!(dec.0, 6);
    // chunk order IHDR, PLTE, tRNS, IDAT..., IEND
    let b = &png.bytes;
    let mut at = 8;
    let mut names = Vec::new();
    while at < b.len() {
        let len = u32::from_be_bytes(b[at..at + 4].try_into().unwrap()) as usize;
        let n = String::from_utf8(b[at + 4..at + 8].to_vec()).unwrap();
        if n == "tRNS" {
            assert_eq!(len, 256);
            assert_eq!((b[at + 8], b[at + 9], b[at + 263]), (0, 255, 255));
        }
        if n == "IHDR" {
            assert_eq!((b[at + 16], b[at + 17], b[at + 20]), (8, 3, 0));
        }
        if names.last() != Some(&n) {
            names.push(n);
        }
        at += 12 + len;
    }
    assert_eq!(names, ["IHDR", "PLTE", "tRNS", "IDAT", "IEND"]);
}

#[test]
fn dc6_without_pixels_has_no_png() {
    let d = dc6(&[(0, 0), (0, 0)], 1);
    let files = round_trip(&d, P, &grey_palette()).unwrap();
    assert_eq!(files.len(), 1);
    assert!(toml_of(&files).contains("sheet = false"));
    // a frame 5 x 0 still takes its width in the layout and has no pixels
    let d = dc6(&[(5, 0), (0, 0)], 1);
    let files = round_trip(&d, P, &grey_palette()).unwrap();
    assert_eq!(files.len(), 1);
}

#[test]
fn dc6_empty_file_and_row_of_empty_frames() {
    let empty = dc6(&[], 1);
    round_trip(&empty, P, &grey_palette()).unwrap();
    // row 0 all 0 x 0 (height 0), row 1 real
    let d = dc6(&[(0, 0), (2, 2)], 2);
    let files = round_trip(&d, P, &grey_palette()).unwrap();
    let png = files.iter().find(|f| f.path.ends_with(".png")).unwrap();
    let (_, h, _) = png::read_indexed("x", &png.bytes).unwrap();
    assert_eq!(h, 2);
}

#[test]
fn writing_twice_gives_identical_bytes() {
    let d = vector();
    assert_eq!(
        d.write(P, &grey_palette()).unwrap(),
        d.write(P, &grey_palette()).unwrap()
    );
}

fn perturb_png(files: &mut [NativeFile], at: usize) {
    let f = files.iter_mut().find(|f| f.path.ends_with(".png")).unwrap();
    let (w, h, mut px) = png::read_indexed("x", &f.bytes).unwrap();
    px[at] = px[at].wrapping_add(1);
    f.bytes = png::write_indexed("x", w, h, &px, &grey_palette()).unwrap();
}

#[test]
fn dc6_pixel_perturbation_names_file_direction_frame_pixel() {
    let d = vector();
    let mut files = d.write(P, &grey_palette()).unwrap();
    // sheet is 6 wide; frame (dir 1, frame 1) is at [4, 2, 2, 3]: its pixel (1, 2)
    perturb_png(&mut files, (2 + 2) * 6 + 4 + 1);
    let e = check_files(&d, P, &files).unwrap_err();
    let CheckError::Mismatch(diff) = e else {
        panic!("{e:?}")
    };
    assert_eq!(diff.file, format!("{P}.png"));
    assert_eq!(
        diff.detail,
        "direction 1 frame 1 pixel (1, 2): index 126 != 127"
    );
}

#[test]
fn dc6_field_perturbation_names_sidecar_and_field() {
    let d = vector();
    let mut files = d.write(P, &grey_palette()).unwrap();
    let t = files
        .iter_mut()
        .find(|f| f.path.ends_with(".toml"))
        .unwrap();
    let s = String::from_utf8(t.bytes.clone()).unwrap();
    t.bytes = s.replacen("offset_y = 7", "offset_y = 8", 1).into_bytes();
    let CheckError::Mismatch(diff) = check_files(&d, P, &files).unwrap_err() else {
        panic!()
    };
    assert_eq!(diff.file, format!("{P}.toml"));
    assert!(
        diff.detail
            .starts_with("direction 0 frame 0: offset_y 7 != 8"),
        "{diff}"
    );
}

#[test]
fn strict_readers_name_the_file() {
    let d = vector();
    let files = d.write(P, &grey_palette()).unwrap();
    let read = |files: &[NativeFile]| Dc6::read(P, &MemStore::from_files(files)).unwrap_err();

    // colour type 2 PNG
    let mut f = files.clone();
    let png = f.iter_mut().find(|f| f.path.ends_with(".png")).unwrap();
    let mut out = Vec::new();
    {
        let mut e = ::png::Encoder::new(&mut out, 6, 5);
        e.set_color(::png::ColorType::Rgb);
        e.set_depth(::png::BitDepth::Eight);
        let mut w = e.write_header().unwrap();
        w.write_image_data(&[0u8; 6 * 5 * 3]).unwrap();
    }
    png.bytes = out;
    let e = read(&f);
    assert_eq!(e.file, format!("{P}.png"));
    assert!(e.message.contains("indexed"), "{e}");

    // unknown native_version
    let mut f = files.clone();
    let t = f.iter_mut().find(|f| f.path.ends_with(".toml")).unwrap();
    t.bytes = String::from_utf8(t.bytes.clone())
        .unwrap()
        .replace("native_version = 1", "native_version = 2")
        .into_bytes();
    let e = read(&f);
    assert_eq!(e.file, format!("{P}.toml"));
    assert!(e.message.contains("native_version"), "{e}");

    // hex in the wrong spelling (upper case)
    let mut f = files.clone();
    let t = f.iter_mut().find(|f| f.path.ends_with(".toml")).unwrap();
    t.bytes = String::from_utf8(t.bytes.clone())
        .unwrap()
        .replace("0xeeeeeeee", "0xEEEEEEEE")
        .into_bytes();
    assert!(read(&f).message.contains("termination"));

    // unknown key
    let mut f = files.clone();
    let t = f.iter_mut().find(|f| f.path.ends_with(".toml")).unwrap();
    t.bytes.extend_from_slice(b"\nextra = 1\n");
    assert!(read(&f).message.contains("unknown key"));

    // a frame resized in the sidecar no longer matches the layout / PNG
    let mut f = files.clone();
    let t = f.iter_mut().find(|f| f.path.ends_with(".toml")).unwrap();
    t.bytes = String::from_utf8(t.bytes.clone())
        .unwrap()
        .replace("rect = [0, 0, 3, 2]", "rect = [0, 0, 4, 2]")
        .into_bytes();
    assert!(read(&f).message.contains("rect"));
}

#[test]
fn mod_authored_file_omits_encoding_and_gets_defaults() {
    let d = vector();
    let mut files = d.write(P, &grey_palette()).unwrap();
    let t = files
        .iter_mut()
        .find(|f| f.path.ends_with(".toml"))
        .unwrap();
    let s = String::from_utf8(t.bytes.clone()).unwrap();
    let s: String = s
        .lines()
        .filter(|l| !l.starts_with("next_block") && !l.contains("encoding]"))
        .map(|l| format!("{l}\n"))
        .collect();
    t.bytes = s.into_bytes();
    let back = Dc6::read(P, &MemStore::from_files(&files)).unwrap();
    assert!(back.frames.iter().all(|f| f.next_block == 0));
    assert_eq!(back.frames[0].pixels, d.frames[0].pixels);
}

fn dcc_frame(w: u32, h: u32, seed: u8) -> DccFrame {
    DccFrame {
        variable0: 0,
        width: w,
        height: h,
        x_offset: -5,
        y_offset: 9,
        coded_bytes: 77,
        bottom_up: seed.is_multiple_of(2),
        optional_data: vec![0xab, 0x01],
        x_min: -4,
        y_min: -8,
        pixels: (0..w * h)
            .map(|i| (i as u8).wrapping_mul(3).wrapping_add(seed))
            .collect(),
    }
}

fn dcc() -> Dcc {
    let dir = |sizes: &[(u32, u32)], s: u8| DccDirection {
        outsize_coded: 1234,
        compression_flags: 0x03,
        x_min: -10,
        y_min: -20,
        width: 30,
        height: 40,
        frames: sizes
            .iter()
            .enumerate()
            .map(|(i, &(w, h))| dcc_frame(w, h, s + i as u8))
            .collect(),
        pcd_leftover_bits: 5,
    };
    Dcc {
        version: 6,
        frames_per_direction: 2,
        tag: 1,
        final_dc6_size: 5555,
        directions: vec![dir(&[(4, 3), (2, 2)], 1), dir(&[(0, 0), (5, 1)], 9)],
    }
}

#[test]
fn dcc_round_trip_with_encoding_fields() {
    let d = dcc();
    let files = round_trip(&d, "data/global/chars/x.dcc", &grey_palette()).unwrap();
    let t = toml_of(&files);
    assert!(t.contains("compression_flags = \"0x03\""));
    assert!(t.contains("optional_data = \"ab01\""));
    // an empty direction (no frames) and one with only empty frames
    let mut e = d.clone();
    e.frames_per_direction = 0;
    for dir in &mut e.directions {
        dir.frames.clear();
    }
    round_trip(&e, "data/global/chars/e.dcc", &grey_palette()).unwrap();
    let mut z = d;
    for dir in &mut z.directions {
        for f in &mut dir.frames {
            f.width = 0;
            f.height = 0;
            f.pixels.clear();
        }
    }
    let files = round_trip(&z, "data/global/chars/z.dcc", &grey_palette()).unwrap();
    assert_eq!(files.len(), 1);
}

#[test]
fn dcc_perturbations_report_the_first_difference() {
    let d = dcc();
    let p = "data/global/chars/x.dcc";
    let mut files = d.write(p, &grey_palette()).unwrap();
    // sheet row 0 is 6 wide x 3; frame (0,1) at [4,0,2,2]; pixel (1,1)
    perturb_png(&mut files, 6 + 4 + 1);
    let CheckError::Mismatch(diff) = check_files(&d, p, &files).unwrap_err() else {
        panic!()
    };
    assert_eq!(diff.file, format!("{p}.png"));
    assert!(
        diff.detail.starts_with("direction 0 frame 1 pixel (1, 1)"),
        "{diff}"
    );

    // an encoding-only field is compared too
    let mut files = d.write(p, &grey_palette()).unwrap();
    let t = files
        .iter_mut()
        .find(|f| f.path.ends_with(".toml"))
        .unwrap();
    t.bytes = String::from_utf8(t.bytes.clone())
        .unwrap()
        .replacen("pcd_leftover_bits = 5", "pcd_leftover_bits = 6", 1)
        .into_bytes();
    let CheckError::Mismatch(diff) = check_files(&d, p, &files).unwrap_err() else {
        panic!()
    };
    assert!(diff.detail.contains("direction 0") && diff.detail.contains("pcd_leftover_bits"));
}
