//! Offline build entry point for a mod's own animation pipeline. Source packs,
//! palettes and ramp labels are supplied by the mod and stay outside this repo.

use super::frame_pack::{FrameKey, FramePack, MAX_BYTES};
use super::generation_job::{generate, read_bounded, AnimationInput};
use crate::frames::IndexFrame;
use d2_formats::palette::Palette;
use std::collections::BTreeMap;
use std::path::Path;

pub fn run_cli(args: &[String]) -> Result<(), String> {
    let mut source = None;
    let mut palette = None;
    let mut ramps = None;
    let mut out = None;
    let mut looping = false;
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--source-pack" => source = Some(args.next().ok_or("missing --source-pack path")?),
            "--palette" => palette = Some(args.next().ok_or("missing --palette path")?),
            "--ramps" => ramps = Some(args.next().ok_or("missing --ramps path")?),
            "--out" => out = Some(args.next().ok_or("missing --out directory")?),
            "--loop" => looping = true,
            _ => return Err(format!("unknown animation-build option {flag}")),
        }
    }
    let source = Path::new(source.ok_or("--source-pack is required")?);
    let palette = Path::new(palette.ok_or("--palette is required")?);
    let ramps = Path::new(ramps.ok_or("--ramps is required")?);
    let out = Path::new(out.ok_or("--out is required")?);
    let pack = FramePack::decode(&read_bounded(source, MAX_BYTES)?).map_err(|e| e.to_string())?;
    let inputs = source_inputs(pack, looping)?;
    let palette = Palette::parse(&read_bounded(palette, 768)?).map_err(|e| e.to_string())?;
    let ramps: [u8; 256] = read_bounded(ramps, 256)?
        .try_into()
        .map_err(|_| "ramps must contain 256 labels")?;
    let result = generate(&inputs, &palette, &ramps, Some(out), |p| {
        if p.completed == 0 || p.completed == p.total || p.completed.is_multiple_of(100) {
            crate::say!(
                "animation frames: {}/{}; accepted {}; rejected {}",
                p.completed,
                p.total,
                p.accepted,
                p.rejected
            );
        }
    })?;
    let hash: String = result
        .pack
        .input_hash
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    crate::say!(
        "pack: {} ({} frames; {} rejected; {:.3}s; cache {})",
        out.join(format!("{hash}.d2frames")).display(),
        result.pack.frames.len(),
        result.rejected.len(),
        result.elapsed.as_secs_f64(),
        result.cache_hit
    );
    Ok(())
}

type SourceDirections = BTreeMap<u8, BTreeMap<u16, IndexFrame>>;

pub fn source_inputs(pack: FramePack, looping: bool) -> Result<Vec<AnimationInput>, String> {
    let mut groups: BTreeMap<FrameKey, SourceDirections> = BTreeMap::new();
    for (key, picture) in pack.frames {
        if key.sub_frame != 0 {
            return Err("source packs must contain sub-frame zero only".into());
        }
        let mut base = key.clone();
        base.direction = 0;
        base.frame = 0;
        groups
            .entry(base)
            .or_default()
            .entry(key.direction)
            .or_default()
            .insert(key.frame, picture);
    }
    groups
        .into_iter()
        .map(|(key, dirs)| {
            let directions = dirs
                .into_iter()
                .map(|(dir, frames)| {
                    if frames
                        .keys()
                        .enumerate()
                        .any(|(index, &frame)| index != usize::from(frame))
                    {
                        return Err("source frames must be contiguous from zero".into());
                    }
                    Ok((dir, frames.into_values().collect()))
                })
                .collect::<Result<_, String>>()?;
            Ok(AnimationInput {
                key,
                directions,
                looping,
            })
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    use d2_formats::palette::Rgb;

    #[test]
    fn mod_source_packs_feed_the_shared_generator_and_refuse_sampled_or_gapped_sources() {
        let key = FrameKey {
            kind: 0,
            token: *b"QA  ",
            mode: *b"QW  ",
            component: 1,
            component_token: *b"QT  ",
            armor_class: *b"lit ",
            weapon_class: *b"hth ",
            direction: 0,
            frame: 0,
            sub_frame: 0,
        };
        let picture = IndexFrame::new(2, 2, 0, 0, vec![10; 4]).unwrap();
        let mut second = key.clone();
        second.frame = 1;
        let source = FramePack {
            input_hash: [0; 32],
            frames: [
                (key.clone(), picture.clone()),
                (second.clone(), picture.clone()),
            ]
            .into(),
        };
        let inputs =
            source_inputs(FramePack::decode(&source.encode().unwrap()).unwrap(), false).unwrap();
        let palette = Palette {
            colors: std::array::from_fn(|i| Rgb {
                r: i as u8,
                g: i as u8,
                b: i as u8,
            }),
        };
        let result = generate(&inputs, &palette, &[0; 256], None, |_| {}).unwrap();
        assert_eq!(result.pack.frames.len(), 6);
        assert!(result.rejected.is_empty());
        assert!(result
            .pack
            .frames
            .values()
            .all(|frame| frame.pixels == picture.pixels));
        let mut gap = source.clone();
        gap.frames.remove(&key);
        assert!(source_inputs(gap, false).is_err());
        let mut sampled = source;
        let image = sampled.frames.remove(&second).unwrap();
        second.sub_frame = 64;
        sampled.frames.insert(second, image);
        assert!(source_inputs(sampled, true).is_err());
    }
}
