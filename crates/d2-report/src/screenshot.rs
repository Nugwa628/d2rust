//! The screenshot inside a bug report (`screenshot.png`).
//!
//! The game hands over the captured frame; this encodes it as an RGB PNG
//! (the alpha channel of a captured frame carries nothing worth sending) and,
//! when a large window makes the PNG too big for the bundle, halves the
//! picture until it fits.

use std::io::Cursor;

use image::{imageops::FilterType, DynamicImage, ImageFormat};

use crate::{FileKind, ReportError, MAX_SCREENSHOT_BYTES};

/// How many times a frame may be halved before the screenshot is given up.
const MAX_HALVINGS: u32 = 4;

/// `frame` as PNG bytes within [`MAX_SCREENSHOT_BYTES`].
pub fn screenshot_png(frame: &DynamicImage) -> Result<Vec<u8>, ReportError> {
    let mut img = DynamicImage::ImageRgb8(frame.to_rgb8());
    for _ in 0..=MAX_HALVINGS {
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .map_err(|e| ReportError::BadContent {
                file: FileKind::Screenshot.file_name(),
                why: e.to_string(),
            })?;
        if out.len() as u64 <= MAX_SCREENSHOT_BYTES {
            return Ok(out);
        }
        let (w, h) = (img.width() / 2, img.height() / 2);
        if w == 0 || h == 0 {
            break;
        }
        img = img.resize_exact(w, h, FilterType::Triangle);
    }
    Err(ReportError::TooLarge {
        what: FileKind::Screenshot.file_name().into(),
        size: u64::from(frame.width()) * u64::from(frame.height()) * 3,
        max: MAX_SCREENSHOT_BYTES,
    })
}
