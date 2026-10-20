use super::*;
use d2_formats::palette::Rgb;

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
fn linear_colour_interpolation_is_quantized_to_a_palette_colour() {
    let palette = palette();
    let ramps = [0; 256];
    let generator = Generator::new(&palette, &ramps);
    let a = IndexFrame::new(4, 4, 0, 0, vec![1; 16]).unwrap();
    let b = IndexFrame::new(4, 4, 0, 0, vec![200; 16]).unwrap();
    let result = generator.interpolate(&a, &b, 128);
    assert_eq!(result.rejected, None);
    assert!(result
        .picture
        .pixels
        .iter()
        .all(|&p| (145..=147).contains(&p)));
    assert_eq!(generator.interpolate(&a, &b, 0).picture, a);
}

#[test]
fn motion_follows_the_component_instead_of_cross_fading_two_silhouettes() {
    let palette = palette();
    let ramps = [0; 256];
    let generator = Generator::new(&palette, &ramps);
    let a = IndexFrame::new(4, 4, 0, 0, vec![100; 16]).unwrap();
    let b = IndexFrame::new(4, 4, 4, 2, vec![100; 16]).unwrap();
    let result = generator.interpolate(&a, &b, 128);
    assert_eq!(result.rejected, None);
    assert_eq!((result.picture.x_off, result.picture.y_off), (2, 1));
    assert_eq!((result.picture.width, result.picture.height), (4, 4));
    assert_eq!(result.picture.pixels, a.pixels);
}

#[test]
fn mapped_colours_stay_on_their_ramp_even_when_another_colour_is_closer() {
    let mut palette = palette();
    palette.colors[10] = Rgb {
        r: 30,
        g: 30,
        b: 30,
    };
    palette.colors[11] = Rgb {
        r: 80,
        g: 80,
        b: 80,
    };
    let mut ramps = [0; 256];
    ramps[10] = 1;
    ramps[11] = 1;
    let generator = Generator::new(&palette, &ramps);
    let a = IndexFrame::new(4, 4, 0, 0, vec![10; 16]).unwrap();
    let b = IndexFrame::new(4, 4, 0, 0, vec![11; 16]).unwrap();
    let result = generator.interpolate(&a, &b, 128);
    assert_eq!(result.rejected, None);
    assert!(result.picture.pixels.iter().all(|&p| p == 10 || p == 11));
}

#[test]
fn incompatible_ramps_or_empty_inputs_keep_the_starting_source_frame() {
    let palette = palette();
    let mut ramps = [0; 256];
    ramps[100] = 1;
    ramps[200] = 2;
    let generator = Generator::new(&palette, &ramps);
    let a = IndexFrame::new(4, 4, 0, 0, vec![100; 16]).unwrap();
    let b = IndexFrame::new(4, 4, 0, 0, vec![200; 16]).unwrap();
    let result = generator.interpolate(&a, &b, 128);
    assert_eq!(result.rejected, Some(RejectReason::RampConflict));
    assert_eq!(result.picture, a);
    let empty = IndexFrame::new(4, 4, 0, 0, vec![0; 16]).unwrap();
    let result = generator.interpolate(&a, &empty, 128);
    assert_eq!(result.rejected, Some(RejectReason::EmptySource));
    assert_eq!(result.picture, a);
}

#[test]
fn overlapping_shades_from_different_families_can_blend() {
    let palette = palette();
    let mut ramps = [0; 256];
    ramps[100] = 1;
    ramps[101] = 2;
    let generator = Generator::new(&palette, &ramps);
    let a = IndexFrame::new(4, 4, 0, 0, vec![100; 16]).unwrap();
    let b = IndexFrame::new(4, 4, 0, 0, vec![101; 16]).unwrap();
    let result = generator.interpolate(&a, &b, 128);
    assert_eq!(result.rejected, None);
    assert!(result.picture.pixels.iter().all(|&p| p == 100 || p == 101));
}
#[test]
fn quality_gates_keep_source_for_motion_shape_and_quantization_failure() {
    let palette = palette();
    let ramps = [0; 256];
    let generator = Generator::new(&palette, &ramps);
    let a = IndexFrame::new(4, 4, 0, 0, vec![1; 16]).unwrap();
    let far = IndexFrame::new(4, 4, 30, 0, vec![1; 16]).unwrap();
    let result = generator.interpolate(&a, &far, 128);
    assert_eq!(result.rejected, Some(RejectReason::MotionMismatch));
    assert_eq!(result.picture, a);
    let larger = IndexFrame::new(16, 16, 0, 0, vec![1; 256]).unwrap();
    let result = generator.interpolate(&a, &larger, 128);
    assert_eq!(result.rejected, Some(RejectReason::ShapeChange));
    assert_eq!(result.picture, a);
    let mut sparse = palette.clone();
    sparse.colors.fill(Rgb::default());
    sparse.colors[2] = Rgb {
        r: 200,
        g: 200,
        b: 200,
    };
    let generator = Generator::new(&sparse, &ramps);
    let b = IndexFrame::new(4, 4, 0, 0, vec![2; 16]).unwrap();
    let result = generator.interpolate(&a, &b, 128);
    assert_eq!(result.rejected, Some(RejectReason::PaletteError));
    assert_eq!(result.picture, a);
}

#[test]
fn ramp_partition_groups_shades_by_colour_family() {
    let mut first: [u8; 256] = std::array::from_fn(|i| i as u8);
    first[10..20].fill(99);
    first[200] = 99;
    let mut second: [u8; 256] = std::array::from_fn(|i| i as u8);
    second[30..40].fill(98);
    let labels = ramps_from_files(&[&[first, second]]);
    assert_eq!(labels[1], 0);
    assert_ne!(labels[10], 0);
    assert_eq!(labels[10], labels[19]);
    // Two maps of one file are one colour family, even where they recolour
    // different bands.
    assert_eq!(labels[10], labels[30]);
    assert_eq!(labels[30], labels[39]);
    // The same signature is one ramp even when its indices are not adjacent.
    assert_eq!(labels[200], labels[10]);

    let mut other: [u8; 256] = std::array::from_fn(|i| i as u8);
    other[50..60].fill(97);
    let labels = ramps_from_files(&[&[first, second], &[other]]);
    assert_eq!(labels[10], labels[30]);
    assert_ne!(labels[10], labels[50]);
    assert_eq!(labels[50], labels[59]);
}
