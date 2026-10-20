//! Commit a canvas resize to the persistent index frame and its GPU images.
//!
//! Validate every target and prepare replacements before changing anything.
//! Same-size targets retain their pixels; changed targets start empty. The
//! caller drains the preceding GPU job before this transaction, then refreshes
//! layout/camera facts and publishes the new frame before accepting input.

use super::native_ui::NativeUiTargets;
use super::node::LayerTargets;
use crate::rules::adaptive_integer::FrameSelection;
use crate::scene::{FrameCycle, SceneError};
use crate::ui::{FrameError, Presentation};
use bevy::prelude::*;
use bevy::render::render_resource::Extent3d;

/// Images that belong to one committed game canvas. Handles stay stable so
/// materials never sample an old-size texture after the transaction.
pub struct FrameTargets<'a> {
    pub frame: Option<&'a Handle<Image>>,
    pub layers: Option<&'a LayerTargets>,
    pub native: Option<&'a mut NativeUiTargets>,
}

#[derive(Debug, thiserror::Error)]
pub enum ResizeError {
    #[error("invalid frame {0:?}")]
    Size(crate::rules::camera::FrameSize),
    #[error("frame resize target is missing: {0:?}")]
    MissingImage(AssetId<Image>),
    #[error(transparent)]
    Cycle(#[from] SceneError),
    #[error(transparent)]
    Presentation(#[from] FrameError),
}

/// Stage all changed targets, then commit them together. Returns true if
/// geometry changed. Call only after any in-flight compositor readback is
/// drained; its old dimensions must never be committed to the new cycle.
pub fn resize_resources(
    cycle: &mut FrameCycle,
    images: &mut Assets<Image>,
    targets: FrameTargets<'_>,
    selection: FrameSelection,
    window: UVec2,
) -> Result<bool, ResizeError> {
    let f = selection.frame;
    if f.width <= 0 || f.height <= crate::scene::UNCLEARED_ROWS as i32 {
        return Err(ResizeError::Size(f));
    }
    let size = UVec2::new(f.width as u32, f.height as u32);
    let p = Presentation::with_mode(window.x, window.y, size.x, size.y, selection.mode)?;
    let output = UVec2::new(p.image_w, p.image_h);
    let mut requested = Vec::new();
    if let Some(frame) = targets.frame {
        requested.push((frame.clone(), size));
    }
    if let Some(layers) = targets.layers {
        for h in [&layers.world, &layers.screen, &layers.prev] {
            requested.push((h.clone(), size));
        }
    }
    if let Some(native) = targets.native.as_ref() {
        for h in [&native.cover, &native.add] {
            requested.push((h.clone(), output));
        }
    }
    for (handle, _) in &requested {
        if images.get(handle).is_none() {
            return Err(ResizeError::MissingImage(handle.id()));
        }
    }
    let changed = (cycle.width(), cycle.height()) != (size.x, size.y);
    let next = if changed {
        let mut next = FrameCycle::new(size.x, size.y)?;
        next.set_post_clear(cycle.post_clear());
        Some(next)
    } else {
        None
    };
    let mut replacements = Vec::new();
    for (handle, want) in requested {
        let image = images.get(&handle).expect("all handles validated");
        if image.size() != want {
            let mut image = image.clone();
            image.resize(Extent3d {
                width: want.x,
                height: want.y,
                depth_or_array_layers: 1,
            });
            if let Some(data) = image.data.as_mut() {
                data.fill(0);
            }
            replacements.push((handle, image));
        }
    }
    let changed = changed || !replacements.is_empty();
    for (handle, image) in replacements {
        *images.get_mut(&handle).expect("validated target") = image;
    }
    if let Some(next) = next {
        *cycle = next;
    }
    if let Some(native) = targets.native {
        native.size = output;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::camera::FrameSize;
    use crate::ui::ScaleMode;
    use bevy::render::render_resource::TextureFormat;

    fn image(images: &mut Assets<Image>) -> Handle<Image> {
        let mut image = Image::new_target_texture(800, 600, TextureFormat::Rgba8Unorm, None);
        image.data = None;
        images.add(image)
    }

    #[test]
    fn live_matrix_resizes_index_gpu_layers_and_native_targets_together() {
        let mut images = Assets::default();
        let frame = image(&mut images);
        let layers = LayerTargets {
            world: image(&mut images),
            screen: image(&mut images),
            prev: image(&mut images),
        };
        let mut native = NativeUiTargets {
            cover: image(&mut images),
            add: image(&mut images),
            size: UVec2::new(800, 600),
        };
        let mut cycle = FrameCycle::new(800, 600).unwrap();
        cycle.set_post_clear(2);
        for (w, h) in [
            (1920, 1080),
            (3840, 2160),
            (2560, 1440),
            (1920, 1200),
            (2560, 1080),
            (3440, 1440),
            (5120, 2160),
            (1365, 767),
            (1919, 1079),
            (800, 1600),
            (7680, 2160),
        ] {
            for mode in [ScaleMode::Integer, ScaleMode::Fit, ScaleMode::Integer] {
                let choice = FrameSelection::for_window(w, h, FrameSize::WIDE, mode);
                resize_resources(
                    &mut cycle,
                    &mut images,
                    FrameTargets {
                        frame: Some(&frame),
                        layers: Some(&layers),
                        native: Some(&mut native),
                    },
                    choice,
                    UVec2::new(w, h),
                )
                .unwrap();
                let want = UVec2::new(choice.frame.width as u32, choice.frame.height as u32);
                assert_eq!((cycle.width(), cycle.height()), (want.x, want.y));
                assert_eq!(cycle.post_clear(), 2);
                assert!(cycle.pixels().iter().all(|&p| p == 0));
                for handle in [&frame, &layers.world, &layers.screen, &layers.prev] {
                    assert_eq!(images.get(handle).unwrap().size(), want);
                }
                let p = Presentation::with_mode(w, h, want.x, want.y, choice.mode).unwrap();
                assert_eq!(native.size, UVec2::new(p.image_w, p.image_h));
                for handle in [&native.cover, &native.add] {
                    assert_eq!(images.get(handle).unwrap().size(), native.size);
                }
            }
        }
    }

    #[test]
    fn same_size_preserves_pixels_and_missing_targets_reject_the_whole_resize() {
        let mut images = Assets::default();
        let frame = image(&mut images);
        let mut cycle = FrameCycle::with_pixels(800, 600, vec![7; 800 * 600]).unwrap();
        let old = cycle.clone();
        let choice = FrameSelection {
            frame: FrameSize::D2RUST,
            mode: ScaleMode::Fit,
        };
        assert!(!resize_resources(
            &mut cycle,
            &mut images,
            FrameTargets {
                frame: Some(&frame),
                layers: None,
                native: None
            },
            choice,
            UVec2::new(800, 600)
        )
        .unwrap());
        assert_eq!(cycle, old);
        let missing = Handle::<Image>::default();
        let layers = LayerTargets {
            world: frame.clone(),
            screen: missing,
            prev: frame.clone(),
        };
        let choice = FrameSelection::for_window(1920, 1080, FrameSize::WIDE, ScaleMode::Integer);
        assert!(matches!(
            resize_resources(
                &mut cycle,
                &mut images,
                FrameTargets {
                    frame: Some(&frame),
                    layers: Some(&layers),
                    native: None
                },
                choice,
                UVec2::new(1920, 1080)
            ),
            Err(ResizeError::MissingImage(_))
        ));
        assert_eq!(cycle, old);
        assert_eq!(images.get(&frame).unwrap().size(), UVec2::new(800, 600));
    }
}
