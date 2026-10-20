//! Synthetic cache tests: no install, network, window or global user folder.
use super::*;
use crate::frames::IndexFrame;
use d2_formats::palette::Rgb;

fn inputs() -> Vec<AnimationInput> {
    vec![AnimationInput {
        key: FrameKey {
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
        },
        directions: [(
            0,
            vec![
                IndexFrame::new(4, 4, 0, 0, vec![100; 16]).unwrap(),
                IndexFrame::new(4, 4, 2, 0, vec![100; 16]).unwrap(),
                IndexFrame::new(4, 4, 2, 0, vec![0; 16]).unwrap(),
            ],
        )]
        .into(),
        looping: true,
    }]
}
fn palette() -> Palette {
    Palette {
        colors: std::array::from_fn(|i| Rgb {
            r: i as u8,
            g: i as u8,
            b: i as u8,
        }),
    }
}

#[test]
fn cache_round_trip_retains_rejects_and_regenerates_corruption_and_changed_inputs() {
    let root = std::env::temp_dir().join(format!(
        "d2-animation-cache-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let mut progress = Vec::new();
    let inputs = inputs();
    let palette = palette();
    let ramps = [0; 256];
    let first = generate(&inputs, &palette, &ramps, Some(&root), |p| progress.push(p)).unwrap();
    assert!(!first.cache_hit);
    assert!(!first.pack.frames.is_empty());
    assert!(!first.rejected.is_empty());
    assert_eq!(first.pack.frames.len() + first.rejected.len(), 9);
    assert_eq!(progress.first().unwrap().completed, 0);
    assert_eq!(progress.last().unwrap().completed, 9);
    assert!(progress
        .windows(2)
        .all(|p| p[0].completed <= p[1].completed));
    let cached = generate(&inputs, &palette, &ramps, Some(&root), |_| {}).unwrap();
    assert!(cached.cache_hit);
    assert_eq!(cached.pack, first.pack);
    assert_eq!(cached.rejected, first.rejected);
    let hash: String = first
        .pack
        .input_hash
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let file = root.join(format!("{hash}.d2frames"));
    std::fs::write(&file, b"broken").unwrap();
    let recovered = generate(&inputs, &palette, &ramps, Some(&root), |_| {}).unwrap();
    assert!(!recovered.cache_hit);
    assert_eq!(recovered.pack, first.pack);
    assert_eq!(recovered.rejected, first.rejected);
    // A validly encoded pixel mutation also invalidates the cache checksum.
    let mut changed_picture = first.pack.clone();
    changed_picture.frames.values_mut().next().unwrap().pixels[0] = 99;
    std::fs::write(&file, changed_picture.encode().unwrap()).unwrap();
    let recovered = generate(&inputs, &palette, &ramps, Some(&root), |_| {}).unwrap();
    assert!(!recovered.cache_hit);
    assert_eq!(recovered.pack, first.pack);
    std::fs::write(file.with_extension("review.json"), b"broken").unwrap();
    assert!(
        !generate(&inputs, &palette, &ramps, Some(&root), |_| {})
            .unwrap()
            .cache_hit
    );
    let review_path = file.with_extension("review.json");
    let mut review: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&review_path).unwrap()).unwrap();
    review["rejected"][0]["candidate"] = serde_json::json!(999999);
    std::fs::write(&review_path, serde_json::to_vec(&review).unwrap()).unwrap();
    assert!(
        !generate(&inputs, &palette, &ramps, Some(&root), |_| {})
            .unwrap()
            .cache_hit
    );
    let mut changed = palette.clone();
    changed.colors[100].r = 101;
    assert_ne!(
        generate(&inputs, &changed, &ramps, None, |_| {})
            .unwrap()
            .pack
            .input_hash,
        first.pack.input_hash
    );
    let mut changed_ramps = ramps;
    changed_ramps[100] = 1;
    assert_ne!(
        generate(&inputs, &palette, &changed_ramps, None, |_| {})
            .unwrap()
            .pack
            .input_hash,
        first.pack.input_hash
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_or_duplicate_inputs_are_refused_before_work_and_nonloops_hold_the_end() {
    let mut inputs = inputs();
    inputs[0].directions.get_mut(&0).unwrap().pop();
    inputs[0].looping = false;
    let result = generate(&inputs, &palette(), &[0; 256], None, |_| {}).unwrap();
    assert_eq!(result.pack.frames.len(), 6);
    for (key, picture) in &result.pack.frames {
        if key.frame == 1 {
            assert_eq!(picture.pixels, vec![100; 16]);
        }
    }
    inputs.push(inputs[0].clone());
    let mut calls = 0;
    assert!(generate(&inputs, &palette(), &[0; 256], None, |_| calls += 1).is_err());
    assert_eq!(calls, 0);
}
