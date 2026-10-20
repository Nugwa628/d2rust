//! Local developer proof of one class's walk/run samples. Images are written
//! only to the explicitly chosen local directory and never uploaded.

use super::animation_inputs::{load_animation, AnimationSelection};
use super::frame_pack::FramePack;
use super::generation_job::{generate, AnimationInput};
use super::generator::ramps_from_source;
use crate::assets::path::{read_cof, read_pl2};
use crate::bridge::world::{ClientUnit, UnitKey, PLAYER};
use crate::frames::{FramePart, FrameSet, FrameSetKey, FrameStore, IndexFrame};
use crate::rules::unit_composite::cof_direction;
use crate::rules::unit_composite::CofPath;
use crate::scene::{DrawItem, MapTable, Rect};
use crate::world_view::unit_assets::{unit_cof, UnitLooks};
use d2_formats::mpq::ArchiveSet;
use d2_formats::palette::Palette;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

pub fn run_cli(args: &[String]) -> Result<(), String> {
    let mut out = None;
    let mut game = None;
    let mut class = 1u32;
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--game-dir" => {
                game = Some(PathBuf::from(args.next().ok_or("missing game directory")?))
            }
            "--out" => out = Some(PathBuf::from(args.next().ok_or("missing proof directory")?)),
            "--class" => {
                class = args
                    .next()
                    .ok_or("missing class")?
                    .parse()
                    .map_err(|_| "invalid class")?
            }
            _ => return Err(format!("unknown animation-proof option {flag}")),
        }
    }
    if class > 6 {
        return Err("class must be 0..6".into());
    }
    let game = game
        .or_else(|| std::env::var_os("D2_GAME_DIR").map(PathBuf::from))
        .ok_or("D2_GAME_DIR required")?;
    let out = out.ok_or("--out required; proof must stay local")?;
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let source = Arc::new(ArchiveSet::open_dir(&game).map_err(|e| e.to_string())?);
    let looks = UnitLooks::live(source.as_ref())?;
    let palette = read_pl2(source.as_ref(), "data/global/palette/act1/pal.pl2")
        .ok_or("act palette missing")??
        .base_palette;
    let ramps = ramps_from_source(source.as_ref())?;
    let directions: Vec<u8> = (0..64).step_by(4).collect();
    let mut accepted = 0usize;
    let mut rejected = BTreeMap::<String, usize>::new();
    let mut generation_seconds = 0.0;
    let mut shots = 0usize;
    let mut changed_shots = 0usize;
    let mut changed_pixels = 0usize;
    let mut animations = Vec::new();
    for (mode, label) in [(2, "walk"), (3, "run")] {
        let mut unit = ClientUnit::new(UnitKey {
            unit_type: PLAYER,
            guid: 1,
        });
        unit.class = class;
        unit.mode = mode;
        let name = unit_cof(&looks, &unit).ok_or("class animation name missing")?;
        let path = name.path().map_err(|e| e.to_string())?;
        let cof = read_cof(source.as_ref(), path.as_str()).ok_or("class COF missing")??;
        if cof.frames == 0 || cof.animation_rate == 0 {
            return Err(format!("{label} has no source animation clock"));
        }
        let inputs = load_animation(
            source.as_ref(),
            &looks,
            &AnimationSelection {
                unit,
                hands: None,
                equipment: None,
                directions: directions.clone(),
            },
        )?;
        if inputs.is_empty() {
            return Err(format!("no source components loaded for {label}"));
        }
        let result = generate(
            &inputs,
            &palette,
            &ramps,
            Some(&out.join("cache").join(label)),
            |progress| {
                if progress.completed == 0
                    || progress.completed == progress.total
                    || progress.completed.is_multiple_of(100)
                {
                    crate::say!(
                        "{label}: {} / {} (accepted {}; rejected {})",
                        progress.completed,
                        progress.total,
                        progress.accepted,
                        progress.rejected
                    );
                }
            },
        )?;
        accepted += result.pack.frames.len();
        generation_seconds += result.elapsed.as_secs_f64();
        for reject in &result.rejected {
            *rejected.entry(format!("{:?}", reject.reason)).or_default() += 1;
        }
        let rate = u64::from(cof.animation_rate.max(1));
        let cycle_ticks = (u64::from(cof.frames) * 256 * 60)
            .div_ceil(rate * 25)
            .max(1);
        if cycle_ticks > 600 {
            return Err(format!("{label} exceeds the 10-second local proof limit"));
        }
        for &direction in &directions {
            let directory = out.join(label).join(format!("dir-{direction:02}"));
            std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
            for tick in 0..cycle_ticks {
                let phase = tick * rate * 25 / 60;
                let frame = ((phase >> 8) % u64::from(cof.frames.max(1))) as usize;
                let sub_frame = phase as u8 / 64 * 64;
                let order = crate::composite::slot_order(
                    &cof,
                    usize::from(
                        cof_direction(cof.directions, direction).map_err(|e| e.to_string())?,
                    ),
                    frame,
                )
                .map_err(|e| e.to_string())?;
                let components: Vec<_> = order.iter().map(|slot| slot.component).collect();
                let context = ProofFrame {
                    inputs: &inputs,
                    pack: &result.pack,
                    palette: &palette,
                    direction,
                    frame,
                    sub_frame,
                    components: &components,
                };
                let rgba = render_pair(&context)?;
                let changed = rgba
                    .as_chunks::<{ 512 * 4 }>()
                    .0
                    .iter()
                    .map(|row| {
                        row[..256 * 4]
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .zip(row[256 * 4..].as_chunks::<4>().0.iter())
                            .filter(|(left, right)| left != right)
                            .count()
                    })
                    .sum::<usize>();
                changed_shots += usize::from(changed > 0);
                changed_pixels += changed;
                image::save_buffer(
                    directory.join(format!("tick-{tick:04}.png")),
                    &rgba,
                    512,
                    256,
                    image::ExtendedColorType::Rgba8,
                )
                .map_err(|e| e.to_string())?;
                shots += 1;
            }
        }
        let hash: String = result
            .pack
            .input_hash
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        animations.push(serde_json::json!({"mode":label,"components":inputs.len(),"cof_frames":cof.frames,
            "facings":directions.len(),"cycle_ticks":cycle_ticks,"input_hash":hash,"cache_hit":result.cache_hit,
            "accepted":result.pack.frames.len(),"rejected":result.rejected.len(),"generation_seconds":result.elapsed.as_secs_f64()}));
    }
    let report = serde_json::json!({"version":1,"class":class,"tick_rate":60,
        "panels":{"left":"Original","right":"Smooth"},"view":"unlit body components, fixed feet, act 1 palette",
        "accepted":accepted,"rejected_by_reason":rejected,"generation_seconds":generation_seconds,
        "shots":shots,"changed_shots":changed_shots,"changed_pixels":changed_pixels,"animations":animations});
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let viewer = include_str!("animation_proof_viewer.html").replace(
        "__PROOF_REPORT__",
        &serde_json::to_string(&report).map_err(|e| e.to_string())?,
    );
    std::fs::write(out.join("index.html"), viewer).map_err(|e| e.to_string())?;
    std::fs::write(out.join("README.txt"), "LOCAL ONLY. Generated from the player's install. Never upload or commit.\nEach PNG: Original left, Smooth right. Sequence rate: 60 Hz. Fixed feet and unlit body components isolate animation changes.\nOpen index.html locally for the 60 Hz side-by-side viewer, cycle/facing selectors and pause/frame controls. Numbered PNGs can also be opened directly.\ncache/*.review.txt lists rejected candidates; those samples retain source pictures.\n").map_err(|e| e.to_string())?;
    crate::say!("local proof: {}", out.join("report.json").display());
    Ok(())
}

struct ProofFrame<'a> {
    inputs: &'a [AnimationInput],
    pack: &'a FramePack,
    palette: &'a Palette,
    components: &'a [u8],
    direction: u8,
    frame: usize,
    sub_frame: u8,
}

fn render_pair(context: &ProofFrame<'_>) -> Result<Vec<u8>, String> {
    let mut frames = FrameStore::new();
    let mut items = Vec::new();
    for panel in 0..2 {
        for &component in context.components {
            let Some(input) = context
                .inputs
                .iter()
                .find(|input| input.key.component == component)
            else {
                continue;
            };
            let Some(source) = input
                .directions
                .get(&context.direction)
                .and_then(|frames| frames.get(context.frame))
            else {
                continue;
            };
            let mut key = input.key.clone();
            key.direction = context.direction;
            key.frame = context.frame as u16;
            key.sub_frame = context.sub_frame;
            let picture = if panel == 1 && context.sub_frame > 0 {
                context.pack.frames.get(&key).unwrap_or(source)
            } else {
                source
            };
            add_picture(&mut frames, &mut items, picture, panel)?;
        }
    }
    crate::scene::compose_rgba(
        &items,
        &frames,
        &MapTable::default(),
        context.palette,
        Rect::new(0, 0, 512, 256),
    )
    .map_err(|e| e.to_string())
}

fn add_picture(
    frames: &mut FrameStore,
    items: &mut Vec<DrawItem>,
    picture: &IndexFrame,
    panel: i32,
) -> Result<(), String> {
    let view = Rect::new(panel * 256, 0, 256, 256);
    let placed = crate::rules::placement::place(picture, panel * 256 + 128, 200, view);
    let Some(clip) = placed.clip else {
        return Ok(());
    };
    let key = FrameSetKey::new(format!("proof/{}.dc6", frames.len()), FramePart::Dir(0))
        .map_err(|e| e.to_string())?;
    let id = frames
        .insert(
            key,
            FrameSet {
                frames: vec![picture.clone()],
            },
        )
        .map_err(|e| e.to_string())?;
    let mut item = DrawItem::new(id, placed.x, placed.y);
    item.clip = clip;
    items.push(item);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::super::frame_pack::FrameKey;
    use super::*;
    use d2_formats::palette::Rgb;

    #[test]
    fn proof_pairs_match_source_boundaries_and_draw_only_accepted_samples() {
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
        let source = IndexFrame::new(4, 4, 0, -4, vec![1; 16]).unwrap();
        let inputs = vec![AnimationInput {
            key: key.clone(),
            directions: [(0, vec![source])].into(),
            looping: true,
        }];
        let mut pack = FramePack {
            input_hash: [9; 32],
            frames: Default::default(),
        };
        let mut sample = key;
        sample.sub_frame = 128;
        pack.frames
            .insert(sample, IndexFrame::new(4, 4, 0, -4, vec![2; 16]).unwrap());
        let mut palette = Palette {
            colors: [Rgb::default(); 256],
        };
        palette.colors[1] = Rgb { r: 200, g: 0, b: 0 };
        palette.colors[2] = Rgb { r: 0, g: 200, b: 0 };
        for fraction in [0, 64, 128, 192] {
            let context = ProofFrame {
                inputs: &inputs,
                pack: &pack,
                palette: &palette,
                components: &[1],
                direction: 0,
                frame: 0,
                sub_frame: fraction,
            };
            let rgba = render_pair(&context).unwrap();
            let differences = rgba
                .as_chunks::<{ 512 * 4 }>()
                .0
                .iter()
                .flat_map(|row| {
                    row[..256 * 4]
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .zip(row[256 * 4..].as_chunks::<4>().0.iter())
                })
                .filter(|(a, b)| a != b)
                .count();
            assert_eq!(differences, if fraction == 128 { 16 } else { 0 });
        }
    }
}
