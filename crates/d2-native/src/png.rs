//! The indexed PNG reader and writer of the picture kinds (sprite sheets,
//! tile sets, palette transforms, the expansion field).
//!
//! Pixels are palette **indices**, never colours: the game recolours the
//! same indices with different palettes (acts, light, item colours). The
//! format is fixed so output is deterministic: colour type 3, bit depth 8,
//! not interlaced; chunks `IHDR`, `PLTE`, `tRNS`, `IDAT`, `IEND` only;
//! filter type 0; one fixed compression level. `PLTE` and `tRNS` exist only
//! so ordinary image viewers show the picture (index 0 transparent); the
//! loaders ignore them.

use crate::kind::{NativeError, ViewPalette};

/// Largest side accepted either way (the PNG limit is 2^31 - 1; this keeps
/// a hostile header from asking for gigabytes).
const MAX_SIDE: u32 = 1 << 16;
const MAX_PIXELS: u64 = 1 << 28;

/// Encodes `width × height` indices (row-major, top row first).
pub fn write_indexed(
    file: &str,
    width: u32,
    height: u32,
    indices: &[u8],
    view: &ViewPalette,
) -> Result<Vec<u8>, NativeError> {
    let err = |m: String| NativeError::new(file, m);
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(err(format!("image size {width} x {height} not writable")));
    }
    if indices.len() as u64 != u64::from(width) * u64::from(height) {
        return Err(err(format!(
            "{} indices for a {width} x {height} image",
            indices.len()
        )));
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, width, height);
        enc.set_color(png::ColorType::Indexed);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Balanced);
        enc.set_filter(png::Filter::NoFilter);
        enc.set_palette(
            view.iter()
                .flat_map(|c| [c.r, c.g, c.b])
                .collect::<Vec<u8>>(),
        );
        let mut trns = vec![255u8; 256];
        trns[0] = 0;
        enc.set_trns(trns);
        let mut w = enc
            .write_header()
            .map_err(|e| err(format!("PNG header: {e}")))?;
        w.write_image_data(indices)
            .map_err(|e| err(format!("PNG data: {e}")))?;
        w.finish().map_err(|e| err(format!("PNG end: {e}")))?;
    }
    Ok(out)
}

/// The decoded indices of an indexed 8-bit PNG: `(width, height, indices)`.
/// Any other colour type, depth or interlacing is an error naming the file.
pub fn read_indexed(file: &str, bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), NativeError> {
    let err = |m: String| NativeError::new(file, m);
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::IDENTITY);
    let mut reader = dec.read_info().map_err(|e| err(format!("bad PNG: {e}")))?;
    let info = reader.info();
    if info.color_type != png::ColorType::Indexed
        || info.bit_depth != png::BitDepth::Eight
        || info.interlaced
    {
        return Err(err(format!(
            "PNG must be indexed (colour type 3), 8-bit, not interlaced; got {:?}, {:?}, interlaced {}",
            info.color_type, info.bit_depth, info.interlaced
        )));
    }
    let (w, h) = (info.width, info.height);
    if w > MAX_SIDE || h > MAX_SIDE || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(err(format!("image size {w} x {h} too large")));
    }
    let mut buf = vec![0u8; reader.output_buffer_size().unwrap_or(0)];
    let frame = reader
        .next_frame(&mut buf)
        .map_err(|e| err(format!("bad PNG data: {e}")))?;
    if frame.width != w || frame.height != h || frame.line_size != w as usize {
        return Err(err("PNG frame does not match its header".into()));
    }
    buf.truncate(w as usize * h as usize);
    Ok((w, h, buf))
}
