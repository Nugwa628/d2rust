//! Reference-free picture checks. Metadata names tick/phase pairs explicitly:
//! repeated pairs must agree; ordered phase triples must not flash and return.
use anyhow::{bail, Context, Result};
use image::RgbaImage;
use serde_json::{json, Value};

pub struct Frame {
    pub name: String,
    pub tick: u64,
    /// Milliseconds within the 40 ms tick (T6).
    pub phase: u64,
    pub image: RgbaImage,
}

pub fn frame_rules(image: &RgbaImage, missing: [u8; 3]) -> Vec<Value> {
    let mut failures = Vec::new();
    if let Some(first) = image.pixels().next() {
        if image.pixels().all(|p| p.0[..3] == first.0[..3]) {
            failures
                .push(json!({"rule": if first.0[..3] == [0,0,0] {"black"} else {"one-colour"}}));
        }
    }
    let count = image.pixels().filter(|p| p.0[..3] == missing).count();
    if count > 0 {
        failures.push(json!({"rule": "missing-texture", "pixels": count, "colour": missing}));
    }
    failures
}

fn distance(a: &RgbaImage, b: &RgbaImage, tol: u8) -> Result<f64> {
    if a.dimensions() != b.dimensions() || a.is_empty() {
        bail!("sequence frames must have the same nonempty dimensions");
    }
    Ok(a.pixels()
        .zip(b.pixels())
        .map(|(a, b)| {
            a.0[..3]
                .iter()
                .zip(&b.0[..3])
                .map(|(a, b)| a.abs_diff(*b).saturating_sub(tol) as f64)
                .sum::<f64>()
        })
        .sum::<f64>()
        / (a.width() as f64 * a.height() as f64 * 3.0))
}

pub fn sequence_rules(frames: &[Frame], tol: u8, jump: f64) -> Result<Vec<Value>> {
    if let Some(first) = frames.first() {
        if first.image.is_empty()
            || frames
                .iter()
                .any(|f| f.image.dimensions() != first.image.dimensions())
        {
            bail!("sequence frames must have the same nonempty dimensions");
        }
    }
    let mut failures = Vec::new();
    let mut ordered: Vec<&Frame> = frames.iter().collect();
    ordered.sort_by_key(|f| (f.tick, f.phase));
    let mut unique: Vec<&Frame> = Vec::new();
    for f in ordered {
        if f.phase >= 40 {
            bail!("phase must be milliseconds in 0..40");
        }
        if let Some(previous) = unique.last() {
            if (previous.tick, previous.phase) == (f.tick, f.phase) {
                if distance(&previous.image, &f.image, tol)? > 0.0 {
                    failures
                        .push(json!({"rule":"stationary-change", "frames":[previous.name,f.name]}));
                }
                continue;
            }
        }
        unique.push(f);
    }
    for triple in unique.windows(3) {
        let [a, b, c] = [triple[0], triple[1], triple[2]];
        // Only adjacent phases of one tick (including the following boundary)
        // are evidence; widely spaced captures cannot establish a flash.
        if b.phase == 0
            || a.tick != b.tick
            || c.tick > b.tick.saturating_add(1)
            || (c.tick != b.tick && c.phase != 0)
        {
            continue;
        }
        let left = distance(&a.image, &b.image, tol)?;
        let right = distance(&b.image, &c.image, tol)?;
        let bypass = distance(&a.image, &c.image, tol)?;
        if left.min(right) > jump && left.min(right) > 4.0 * bypass.max(0.25) {
            failures.push(json!({"rule":"phase-jump", "frame":b.name, "left":left,"right":right,"neighbours":bypass}));
        }
    }
    // Also catch a discontinuous step that stays displaced: the middle edge
    // is an outlier against both surrounding edges, even without a return.
    for window in unique.windows(4) {
        let [a, b, c, d] = [window[0], window[1], window[2], window[3]];
        if b.tick != c.tick
            || c.phase == 0
            || a.tick != b.tick
            || d.tick > c.tick.saturating_add(1)
            || (d.tick != c.tick && d.phase != 0)
        {
            continue;
        }
        let middle = distance(&b.image, &c.image, tol)?;
        let neighbours = distance(&a.image, &b.image, tol)?.max(distance(&c.image, &d.image, tol)?);
        if middle > jump && middle > 4.0 * neighbours.max(0.25) {
            failures.push(json!({"rule":"phase-jump", "frames":[b.name,c.name],
                "jump":middle, "neighbour_edges":neighbours}));
        }
    }
    Ok(failures)
}

/// T9 `ui` reply: `ids` with id and rect [x,y,w,h] in frame pixels.
/// A bare element array is accepted too. Containment also counts as overlap.
pub fn ui_rules(value: &Value, width: u32, height: u32) -> Result<Vec<Value>> {
    let elements = value
        .get("ids")
        .or_else(|| value.get("elements"))
        .unwrap_or(value)
        .as_array()
        .context("UI ids: expected elements array")?;
    let mut rects = Vec::new();
    let mut failures = Vec::new();
    for e in elements {
        let id = e["id"].as_str().context("UI element needs id")?;
        let r = e.get("rect").unwrap_or(e);
        let number = |key: &str| -> Result<f64> {
            let index = match key {
                "x" => 0,
                "y" => 1,
                "w" => 2,
                _ => 3,
            };
            let n = (if r.is_array() { &r[index] } else { &r[key] })
                .as_f64()
                .with_context(|| format!("{id}: missing {key}"))?;
            if !n.is_finite() {
                bail!("{id}: nonfinite {key}");
            }
            Ok(n)
        };
        let (x, y, w, h) = (number("x")?, number("y")?, number("w")?, number("h")?);
        if w <= 0.0
            || h <= 0.0
            || x < 0.0
            || y < 0.0
            || x + w > width as f64
            || y + h > height as f64
        {
            failures.push(json!({"rule":"ui-offscreen", "id":id}));
        }
        for &(other, ox, oy, ow, oh) in &rects {
            if w > 0.0
                && h > 0.0
                && ow > 0.0
                && oh > 0.0
                && x < ox + ow
                && ox < x + w
                && y < oy + oh
                && oy < y + h
            {
                failures.push(json!({"rule":"ui-overlap", "ids":[other,id]}));
            }
        }
        rects.push((id, x, y, w, h));
    }
    Ok(failures)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;
    fn image(n: u8) -> RgbaImage {
        let mut i = RgbaImage::from_pixel(8, 8, Rgba([n, n, n, 255]));
        i.put_pixel(0, 0, Rgba([12, 34, 56, 255]));
        i
    }
    fn frame(tick: u64, phase: u64, n: u8) -> Frame {
        Frame {
            name: format!("{tick}-{phase}-{n}"),
            tick,
            phase,
            image: image(n),
        }
    }
    #[test]
    fn black_and_uniform() {
        for (n, rule) in [(0, "black"), (42, "one-colour")] {
            assert_eq!(
                frame_rules(
                    &RgbaImage::from_pixel(2, 2, Rgba([n, n, n, 255])),
                    [255, 0, 255]
                )[0]["rule"],
                rule
            );
        }
        assert!(frame_rules(&image(4), [255, 0, 255]).is_empty());
    }
    #[test]
    fn missing_colour() {
        let mut i = image(4);
        i.put_pixel(1, 1, Rgba([255, 0, 255, 255]));
        assert_eq!(frame_rules(&i, [255, 0, 255])[0]["rule"], "missing-texture");
        assert!(frame_rules(&i, [0, 255, 0]).is_empty());
    }
    #[test]
    fn stationary_change_and_tolerance() {
        assert_eq!(
            sequence_rules(&[frame(1, 10, 10), frame(1, 10, 11)], 0, 8.0).unwrap()[0]["rule"],
            "stationary-change"
        );
        assert!(
            sequence_rules(&[frame(1, 10, 10), frame(1, 10, 11)], 1, 8.0)
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn phase_flash_and_smooth_motion() {
        assert_eq!(
            sequence_rules(
                &[frame(1, 0, 10), frame(1, 10, 250), frame(1, 20, 10)],
                0,
                8.0
            )
            .unwrap()[0]["rule"],
            "phase-jump"
        );
        assert!(sequence_rules(
            &[frame(1, 0, 10), frame(1, 10, 20), frame(1, 20, 30)],
            0,
            8.0
        )
        .unwrap()
        .is_empty());
        assert!(sequence_rules(
            &[frame(1, 0, 10), frame(2, 10, 250), frame(3, 20, 10)],
            0,
            8.0
        )
        .unwrap()
        .is_empty());
    }
    #[test]
    fn phase_step_outlier() {
        assert_eq!(
            sequence_rules(
                &[
                    frame(1, 0, 10),
                    frame(1, 10, 10),
                    frame(1, 20, 200),
                    frame(1, 30, 200)
                ],
                0,
                8.0
            )
            .unwrap()[0]["rule"],
            "phase-jump"
        );
        assert!(sequence_rules(
            &[
                frame(1, 0, 10),
                frame(1, 10, 20),
                frame(1, 20, 30),
                frame(1, 30, 40)
            ],
            0,
            8.0
        )
        .unwrap()
        .is_empty());
    }
    #[test]
    fn real_t9_reply_and_invalid_phase() {
        assert!(ui_rules(
            &json!({"k":"ui","ids":[{"id":"menu.start","rect":[0,0,4,4],"at":[2,2]}]}),
            8,
            8
        )
        .unwrap()
        .is_empty());
        assert!(sequence_rules(&[frame(1, 40, 10)], 0, 8.0).is_err());
        let mut wrong = frame(1, 0, 10);
        wrong.image = RgbaImage::new(1, 1);
        assert!(sequence_rules(&[frame(1, 0, 10), wrong], 0, 8.0).is_err());
    }
    #[test]
    fn ui_clipped_overlap_and_touching() {
        let ui = json!([{"id":"a","x":-1,"y":0,"w":4,"h":4},{"id":"b","x":2,"y":2,"w":4,"h":4}]);
        let rules = ui_rules(&ui, 8, 8).unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0]["rule"], "ui-offscreen");
        assert_eq!(rules[1]["rule"], "ui-overlap");
        assert!(ui_rules(
            &json!([{"id":"a","x":0,"y":0,"w":4,"h":4},{"id":"b","x":4,"y":0,"w":4,"h":4}]),
            8,
            8
        )
        .unwrap()
        .is_empty());
        assert!(ui_rules(&json!([{"id":"bad"}]), 8, 8).is_err());
    }
}
