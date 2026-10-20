use super::*;
use crate::frames::IndexFrame;
use crate::world_view::unit_rules::frame_pack::FrameKey;
use crate::world_view::unit_rules::generation_job::{generate_packed, hash_inputs, AnimationInput};
use crate::world_view::unit_rules::generator::Generator;
use d2_formats::palette::{Palette, Rgb};

fn key(token: [u8; 4]) -> FrameKey {
    FrameKey {
        kind: 0,
        token,
        mode: *b"A1  ",
        component: 0,
        component_token: *b"HD  ",
        armor_class: *b"LIT ",
        weapon_class: *b"HTH ",
        direction: 0,
        frame: 0,
        sub_frame: 0,
    }
}

fn input() -> AnimationInput {
    AnimationInput {
        key: key(*b"QA  "),
        directions: [(
            0u8,
            vec![
                IndexFrame::new(4, 4, 0, 0, vec![10; 16]).unwrap(),
                IndexFrame::new(4, 4, 2, 1, vec![30; 16]).unwrap(),
                IndexFrame::new(4, 4, 4, 2, vec![50; 16]).unwrap(),
            ],
        )]
        .into(),
        looping: true,
    }
}

fn palette() -> Palette {
    Palette {
        colors: std::array::from_fn(|i| Rgb {
            r: i as u8,
            g: (i * 3) as u8,
            b: (i * 7) as u8,
        }),
    }
}

#[test]
fn vectors_and_the_shared_lut_rebuild_exactly_the_generator_pictures() {
    let palette = palette();
    let ramps = [0u8; 256];
    let generator = Generator::new(&palette, &ramps);
    let tables = generator.tables();
    let input = input();
    let packed = generate_packed(&input, &generator, &tables, |_| {}).unwrap();
    assert!(packed.accepted > 0);
    let pack = BakedPack::from_pairs(packed.input_hash, input.looping, &packed.pairs).unwrap();
    let bytes = pack.encode().unwrap();
    let decoded = BakedPack::decode(&bytes).unwrap();
    assert_eq!(decoded, pack);
    assert_eq!(decoded.encode().unwrap(), bytes);
    let frames = decoded.decode_to_frames(&input, tables.bytes()).unwrap();
    assert_eq!(frames.frames.len(), packed.accepted as usize);
    for (frame_key, picture) in &frames.frames {
        let sources = &input.directions[&frame_key.direction];
        let a = &sources[usize::from(frame_key.frame)];
        let b = if frame_key.frame as usize + 1 == sources.len() {
            &sources[0]
        } else {
            &sources[usize::from(frame_key.frame) + 1]
        };
        let expected = generator.interpolate(a, b, frame_key.sub_frame);
        assert_eq!(expected.rejected, None);
        assert_eq!(&expected.picture, picture, "{frame_key:?}");
    }
}

#[test]
fn the_identity_is_the_source_content_not_the_archive_name() {
    let palette = palette();
    let ramps = [0u8; 256];
    let first = input();
    let mut second = input();
    second.key = key(*b"QB  ");
    assert_eq!(
        hash_inputs(std::slice::from_ref(&first), &palette, &ramps),
        hash_inputs(std::slice::from_ref(&second), &palette, &ramps)
    );
    second.directions.get_mut(&0).unwrap()[0].pixels[0] = 11;
    assert_ne!(
        hash_inputs(std::slice::from_ref(&first), &palette, &ramps),
        hash_inputs(std::slice::from_ref(&second), &palette, &ramps)
    );
}

#[test]
fn malformed_packs_are_rejected_before_decoding() {
    let palette = palette();
    let ramps = [0u8; 256];
    let generator = Generator::new(&palette, &ramps);
    let tables = generator.tables();
    let input = input();
    let packed = generate_packed(&input, &generator, &tables, |_| {}).unwrap();
    let pack = BakedPack::from_pairs(packed.input_hash, input.looping, &packed.pairs).unwrap();
    let bytes = pack.encode().unwrap();
    for end in 0..bytes.len() {
        assert!(BakedPack::decode(&bytes[..end]).is_err(), "end {end}");
    }
    let mut bad = bytes.clone();
    bad[8] = 1;
    assert_eq!(BakedPack::decode(&bad), Err(PackError::Version(1)));
    // The first pair's mask byte: bit 3 is not a fraction.
    let first_pair = 44 + 1 + 2;
    let mut bad = bytes.clone();
    bad[first_pair + 2] |= 0x08;
    assert!(BakedPack::decode(&bad).is_err());
    // Duplicate direction 0.
    let accepted = bytes[bytes.len() - 4..].to_vec();
    let entry = bytes[44..bytes.len() - 4].to_vec();
    let mut bad = bytes[..44].to_vec();
    bad[43] = 2;
    bad.extend_from_slice(&entry);
    bad.extend_from_slice(&entry);
    bad.extend_from_slice(&accepted);
    assert!(BakedPack::decode(&bad).is_err());
    // Trailing bytes and a wrong accepted count.
    let mut bad = bytes.clone();
    bad.push(0);
    assert!(BakedPack::decode(&bad).is_err());
    let mut bad = bytes.clone();
    let at = bad.len() - 4;
    bad[at..].copy_from_slice(&0u32.to_le_bytes());
    assert!(BakedPack::decode(&bad).is_err());
    // Source cross-checks: a changed frame count or direction.
    assert!(pack.matches(&input).is_ok());
    let mut changed = input.clone();
    changed
        .directions
        .get_mut(&0)
        .unwrap()
        .push(IndexFrame::new(4, 4, 0, 0, vec![1; 16]).unwrap());
    assert!(pack.matches(&changed).is_err());
    // A short blend table is refused.
    assert!(pack
        .decode_to_frames(&input, &tables.bytes()[..LUT_BYTES - 1])
        .is_err());
    // Pair vectors are stored in ascending frame order.
    let mut pairs = packed.pairs.clone();
    pairs.swap(0, 1);
    assert!(BakedPack::from_pairs(packed.input_hash, input.looping, &pairs).is_err());
}

/// Compares a v2 bake against a reference bake of the same game data, frame
/// for frame, and reports every difference (accepted-set changes, shapes,
/// pixel diffs). Set `D2_VERIFY_OLD` (reference bake folder), `D2_VERIFY_NEW`
/// (v2 bake folder) and `D2_GAME_DIR`; run with `--ignored --no-capture`.
/// Not a CI test: it needs both bakes and the game files. With the exact
/// motion search this reported zero differences; the two-stage search's
/// measured diff is in the PR.
#[test]
#[ignore = "needs a reference bake and the game files"]
fn v2_bake_diff_report_against_a_reference_bake() {
    use crate::world_view::unit_rules::animation_bake::{read_index, INDEX};
    use crate::world_view::unit_rules::animation_baked::{read_baked_pack, read_blend_lut};
    use crate::world_view::unit_rules::generation_job::read_bounded;
    use d2_formats::mpq::ArchiveSet;
    use std::collections::BTreeMap;
    use std::io::Read;

    let (Ok(old_dir), Ok(new_dir), Ok(game)) = (
        std::env::var("D2_VERIFY_OLD"),
        std::env::var("D2_VERIFY_NEW"),
        std::env::var("D2_GAME_DIR"),
    ) else {
        return;
    };
    let old = read_index(&std::path::Path::new(&old_dir).join(INDEX)).expect("old bake.json");
    let new = read_index(&std::path::Path::new(&new_dir).join(INDEX)).expect("new bake.json");
    let lut = read_blend_lut(std::path::Path::new(&new_dir), &new.blend_lut).expect("blend.lut");
    let source = ArchiveSet::open_dir(std::path::Path::new(&game)).expect("game archives");
    let old_by_source: BTreeMap<&str, &super::super::animation_bake::PackEntry> =
        old.packs.iter().map(|p| (p.source.as_str(), p)).collect();
    let sample: Vec<_> = new
        .packs
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 32 == 0)
        .map(|(_, p)| p)
        .collect();
    let (mut frames, mut same, mut changed, mut missing, mut added) = (0usize, 0, 0, 0, 0);
    let mut source_missing = 0usize;
    let (mut pixels, mut max_error) = (0u64, 0u16);
    let (mut over16, mut over64, mut over128, mut shape_changed, mut big_frames) =
        (0u64, 0u64, 0u64, 0usize, 0usize);
    let (mut max_rgb, mut rgb32, mut rgb64) = (0u16, 0u64, 0u64);
    let palette = super::super::animation_bake::bake_palette(&source).expect("palette");
    let mut decode_ms = Vec::new();
    let mut source_ms = Vec::new();
    let mut install_ms = Vec::new();
    let mut frames_in_pack = Vec::new();
    for entry in &sample {
        let Some(before) = old_by_source.get(entry.source.as_str()) else {
            source_missing += 1;
            continue;
        };
        let started = std::time::Instant::now();
        let input = super::super::animation_inputs::load_component_file(
            &source,
            &entry.source,
            entry.format(),
            entry.file_key(),
        )
        .expect("source loads")
        .expect("source exists");
        source_ms.push(started.elapsed().as_secs_f64() * 1e3);
        let pack =
            read_baked_pack(&std::path::Path::new(&new_dir).join(&entry.pack)).expect("new pack");
        let started = std::time::Instant::now();
        let rebuilt = pack.decode_to_frames(&input, &lut).expect("decode");
        decode_ms.push(started.elapsed().as_secs_f64() * 1e3);
        frames_in_pack.push(rebuilt.frames.len());
        let mut store = crate::frames::FrameStore::new();
        let mut selected = super::super::playback::PackFrames::default();
        let started = std::time::Instant::now();
        selected.install(&rebuilt, &mut store).expect("install");
        install_ms.push(started.elapsed().as_secs_f64() * 1e3);
        let old_bytes = read_bounded(&std::path::Path::new(&old_dir).join(&before.pack), 64 << 20)
            .expect("old pack bytes");
        let mut inflated = Vec::new();
        flate2::read::DeflateDecoder::new(old_bytes.as_slice())
            .take(64 << 20)
            .read_to_end(&mut inflated)
            .expect("old pack inflates");
        let reference = super::super::frame_pack::FramePack::decode(&inflated).expect("old pack");
        for (key, picture) in &rebuilt.frames {
            frames += 1;
            match reference.frames.get(key) {
                None => added += 1,
                Some(want) if want == picture => same += 1,
                Some(want) => {
                    changed += 1;
                    if (want.width, want.height) == (picture.width, picture.height) {
                        let mut frame_max = 0u16;
                        for (a, b) in want.pixels.iter().zip(&picture.pixels) {
                            if a != b {
                                pixels += 1;
                                let diff = a.abs_diff(*b);
                                frame_max = frame_max.max(u16::from(diff));
                                max_error = max_error.max(u16::from(diff));
                                if diff > 16 {
                                    over16 += 1;
                                }
                                if diff > 64 {
                                    over64 += 1;
                                }
                                if diff > 128 {
                                    over128 += 1;
                                }
                                let ca = palette.colors[usize::from(*a)];
                                let cb = palette.colors[usize::from(*b)];
                                let rgb = [
                                    ca.r.abs_diff(cb.r),
                                    ca.g.abs_diff(cb.g),
                                    ca.b.abs_diff(cb.b),
                                ]
                                .into_iter()
                                .max()
                                .unwrap_or(0);
                                max_rgb = max_rgb.max(u16::from(rgb));
                                if rgb > 32 {
                                    rgb32 += 1;
                                }
                                if rgb > 64 {
                                    rgb64 += 1;
                                }
                            }
                        }
                        if frame_max > 64 {
                            big_frames += 1;
                        }
                    } else {
                        shape_changed += 1;
                    }
                }
            }
        }
        for key in reference.frames.keys() {
            if !rebuilt.frames.contains_key(key) {
                missing += 1;
            }
        }
    }
    println!(
        "v2 verify: {} packs, {source_missing} source(s) missing from the reference, {frames} frames: {same} identical, {changed} changed ({shape_changed} different shape), {missing} only in the reference, {added} only in v2, {pixels} differing pixels (>16: {over16}, >64: {over64}, >128: {over128}), {big_frames} changed frames with an index diff >64, max index diff {max_error}, max RGB channel diff {max_rgb}, pixels with RGB diff >32: {rgb32}, >64: {rgb64}",
        sample.len()
    );
    if !decode_ms.is_empty() {
        let mut sorted = decode_ms.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mut install_sorted = install_ms.clone();
        install_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mean = decode_ms.iter().sum::<f64>() / decode_ms.len() as f64;
        let source_mean = source_ms.iter().sum::<f64>() / source_ms.len() as f64;
        let install_mean = install_ms.iter().sum::<f64>() / install_ms.len() as f64;
        let p95 = sorted[(sorted.len() * 95 / 100).min(sorted.len() - 1)];
        let install_p95 =
            install_sorted[(install_sorted.len() * 95 / 100).min(install_sorted.len() - 1)];
        let total_frames: usize = frames_in_pack.iter().sum();
        println!(
            "v2 load: {} packs, decode {mean:.3} ms/pack mean, {p95:.3} ms p95, {:.3} ms max; source read {source_mean:.3} ms/pack mean; main-thread install {install_mean:.3} ms/pack mean, {install_p95:.3} ms p95, {:.3} ms max; {:.1} frames/pack",
            decode_ms.len(),
            sorted.last().copied().unwrap_or(0.0),
            install_sorted.last().copied().unwrap_or(0.0),
            total_frames as f64 / decode_ms.len() as f64
        );
    }
    assert!(frames > 0);
}
