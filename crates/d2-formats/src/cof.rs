//! COF files: which layers make up a composite animation, its frame events,
//! and the layer draw order of every frame.
//!
//! A unit's animation (a mode, such as walking, for one token and weapon
//! class) is drawn from up to 16 components (head, torso, legs, arms, hands,
//! shield and eight special slots), each its own DCC or DC6 sprite. The `.cof`
//! file says which components are present, the animation box and rate, what
//! happens on each frame, and the back-to-front order the components are
//! stacked in for each direction and frame. [`Cof::parse`] takes the bytes of
//! a `.cof` file from the player's game data.
//!
//! Layout (little-endian):
//!
//! | offset | size        | field                                         |
//! |--------|-------------|-----------------------------------------------|
//! | 0x00   | 1           | number of layers `L`                          |
//! | 0x01   | 1           | frames per direction `F`                      |
//! | 0x02   | 1           | number of directions `D`                      |
//! | 0x03   | 1           | version                                       |
//! | 0x04   | 4           | unknown bytes                                 |
//! | 0x08   | 4 × 4       | animation box: x min, x max, y min, y max     |
//! | 0x18   | 4           | animation rate                                |
//! | 0x1C   | 9 × `L`     | layer records                                 |
//! |        | `F` + pad   | frame events, one byte per frame              |
//! |        | `D`×`F`×`L` | draw order: component IDs, back to front      |
//!
//! The event block is everything between the layer records and the draw
//! order, so it may be longer than `F` bytes; the extra bytes are kept as
//! [`Cof::event_padding`].

use crate::cursor::{invalid, Cursor, FormatError};

const FORMAT: &str = "cof";
/// Size of the fixed header.
const HEADER_LEN: usize = 28;
/// Size of one layer record.
const LAYER_LEN: usize = 9;
/// Number of animation components (layer slots).
pub const COMPONENTS: usize = 16;

/// Component names, indexed by component ID: head, torso, legs, right arm,
/// left arm, right hand, left hand, shield, and special slots 1 to 8.
pub const COMPONENT_NAMES: [&str; COMPONENTS] = [
    "HD", "TR", "LG", "RA", "LA", "RH", "LH", "SH", "S1", "S2", "S3", "S4", "S5", "S6", "S7", "S8",
];

/// One layer record of a COF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CofLayer {
    /// Component ID of the layer, an index into [`COMPONENT_NAMES`].
    pub component: u8,
    /// Shadow byte of the layer record.
    pub shadow: u8,
    /// Selectable byte of the layer record.
    pub selectable: u8,
    /// Override-translucency byte of the layer record.
    pub override_translucency: u8,
    /// New-translucency byte of the layer record.
    pub new_translucency: u8,
    /// Weapon class, e.g. `*b"hth\0"`.
    pub weapon_class: [u8; 4],
}

/// A parsed COF: header, layers, per-frame events and layer draw order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cof {
    /// Number of layers.
    pub layers_count: u8,
    /// Frames per direction.
    pub frames: u8,
    /// Number of directions.
    pub directions: u8,
    /// Version byte of the header.
    pub version: u8,
    /// Header bytes at +0x04, kept as read.
    pub unknown: [u8; 4],
    /// Left edge of the animation box.
    pub x_min: i32,
    /// Right edge of the animation box.
    pub x_max: i32,
    /// Top edge of the animation box.
    pub y_min: i32,
    /// Bottom edge of the animation box.
    pub y_max: i32,
    /// Animation rate word of the header.
    pub animation_rate: u32,
    /// The layer records, in file order.
    pub layers: Vec<CofLayer>,
    /// One event per frame: 0 none, 1 attack, 2 missile, 3 sound, 4 skill.
    pub events: Vec<u8>,
    /// Extra event bytes beyond one per frame.
    pub event_padding: Vec<u8>,
    /// `[(d * frames + f) * layers + slot]` = component ID, back to front.
    pub draw_order: Vec<u8>,
}

impl Cof {
    /// Parses a COF file. Fails if the file is truncated or its sizes do not add up.
    pub fn parse(data: &[u8]) -> Result<Cof, FormatError> {
        let mut c = Cursor::new(data, FORMAT);
        let layers_count = c.u8()?;
        let frames = c.u8()?;
        let directions = c.u8()?;
        let version = c.u8()?;
        let mut unknown = [0u8; 4];
        unknown.copy_from_slice(c.bytes(4)?);
        let x_min = c.i32()?;
        let x_max = c.i32()?;
        let y_min = c.i32()?;
        let y_max = c.i32()?;
        let animation_rate = c.u32()?;

        let (l, f, d) = (
            usize::from(layers_count),
            usize::from(frames),
            usize::from(directions),
        );
        let mut layers = Vec::with_capacity(l);
        for i in 0..l {
            let component = c.u8()?;
            if usize::from(component) >= COMPONENTS {
                return Err(invalid(FORMAT, format!("layer {i}: component {component}")));
            }
            let shadow = c.u8()?;
            let selectable = c.u8()?;
            let override_translucency = c.u8()?;
            let new_translucency = c.u8()?;
            let mut weapon_class = [0u8; 4];
            weapon_class.copy_from_slice(c.bytes(4)?);
            layers.push(CofLayer {
                component,
                shadow,
                selectable,
                override_translucency,
                new_translucency,
                weapon_class,
            });
        }

        // The event block is whatever lies between the layer records and the
        // draw order, and must hold at least one event per frame.
        let order_len = d * f * l;
        let fixed = HEADER_LEN + LAYER_LEN * l + order_len;
        let k = data
            .len()
            .checked_sub(fixed)
            .filter(|&k| k >= f)
            .ok_or_else(|| {
                invalid(
                    FORMAT,
                    format!("{} bytes is too short for {f} frame events", data.len()),
                )
            })?;
        let event_bytes = c.bytes(k)?;
        let events = event_bytes[..f].to_vec();
        let event_padding = event_bytes[f..].to_vec();
        let draw_order = c.bytes(order_len)?.to_vec();
        if let Some(&bad) = draw_order.iter().find(|&&b| usize::from(b) >= COMPONENTS) {
            return Err(invalid(FORMAT, format!("draw order component {bad}")));
        }
        Ok(Cof {
            layers_count,
            frames,
            directions,
            version,
            unknown,
            x_min,
            x_max,
            y_min,
            y_max,
            animation_rate,
            layers,
            events,
            event_padding,
            draw_order,
        })
    }

    /// Component drawn in `slot` (back to front) for direction `d`, frame `f`.
    pub fn component_at(&self, d: usize, f: usize, slot: usize) -> Option<u8> {
        let (l, fr) = (usize::from(self.layers_count), usize::from(self.frames));
        if slot >= l || f >= fr {
            return None;
        }
        let at = d.checked_mul(fr)?.checked_add(f)?.checked_mul(l)?;
        self.draw_order.get(at.checked_add(slot)?).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(l: u8, f: u8, d: u8, components: &[u8], events: &[u8], order: &[u8]) -> Vec<u8> {
        let mut v = vec![l, f, d, 20, 0, 0, 0, 0];
        for x in [-10i32, 10, -20, 0] {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v.extend_from_slice(&256u32.to_le_bytes());
        for &comp in components {
            v.extend_from_slice(&[comp, 1, 1, 0, 0]);
            v.extend_from_slice(b"hth\0");
        }
        v.extend_from_slice(events);
        v.extend_from_slice(order);
        v
    }

    #[test]
    fn parses() {
        let cof = Cof::parse(&file(1, 2, 1, &[1], &[1, 0], &[1, 1])).unwrap();
        assert_eq!(cof.events, [1, 0]);
        assert!(cof.event_padding.is_empty());
        assert_eq!(cof.layers[0].weapon_class, *b"hth\0");
        assert_eq!(cof.component_at(0, 1, 0), Some(1));
        assert_eq!(cof.component_at(0, 2, 0), None);
        assert_eq!((cof.x_min, cof.y_min), (-10, -20));
    }

    #[test]
    fn padded_events() {
        let data = file(1, 1, 1, &[0], &[3, 0, 0, 0], &[0]);
        assert_eq!(data.len(), 42);
        let cof = Cof::parse(&data).unwrap();
        assert_eq!(cof.events, [3]);
        assert_eq!(cof.event_padding, [0, 0, 0]);
    }

    #[test]
    fn errors() {
        assert!(
            Cof::parse(&file(1, 1, 1, &[16], &[0], &[0])).is_err(),
            "component"
        );
        assert!(
            Cof::parse(&file(1, 1, 1, &[0], &[0], &[17])).is_err(),
            "order"
        );
        assert!(
            Cof::parse(&file(1, 2, 1, &[0], &[0], &[0, 0])).is_err(),
            "too short"
        );
    }

    #[test]
    fn integers_are_little_endian() {
        let mut data = file(1, 1, 1, &[0], &[0], &[0]);
        // Header i32 / u32 fields whose big-endian reading differs.
        data[8..12].copy_from_slice(&[0xFE, 0xFF, 0xFF, 0xFF]);
        data[12..16].copy_from_slice(&[0x04, 0x03, 0x02, 0x01]);
        data[16..20].copy_from_slice(&[0x00, 0x00, 0x00, 0x80]);
        data[20..24].copy_from_slice(&[0x01, 0x00, 0x00, 0x00]);
        data[24..28].copy_from_slice(&[0x00, 0x01, 0x00, 0x00]);
        let cof = Cof::parse(&data).unwrap();
        assert_eq!(cof.x_min, -2);
        assert_eq!(cof.x_max, 0x0102_0304);
        assert_eq!(cof.y_min, i32::MIN);
        assert_eq!(cof.y_max, 1);
        assert_eq!(cof.animation_rate, 256);
    }

    #[test]
    fn regress_component_at_overflow() {
        // `(d * frames + f) * layers + slot` overflowed for a huge
        // direction (panic in debug builds).
        let cof = Cof::parse(&file(1, 2, 1, &[1], &[1, 0], &[1, 1])).unwrap();
        assert_eq!(cof.component_at(usize::MAX, 0, 0), None);
        assert_eq!(cof.component_at(usize::MAX / 2, 1, 0), None);
    }

    mod robust {
        use super::*;
        use crate::robust::{bounded, mutated};
        use crate::robust_tests::config;
        use proptest::prelude::*;

        fn valid() -> Vec<u8> {
            file(2, 2, 2, &[0, 1], &[1, 0, 0], &[0, 1, 1, 0, 0, 1, 1, 0])
        }

        #[test]
        fn builder_is_valid() {
            assert!(Cof::parse(&valid()).is_ok());
        }

        proptest! {
            #![proptest_config(config(64))]

            #[test]
            fn mutated_file(
                data in mutated(valid()),
                d in any::<usize>(),
                f in any::<usize>(),
                s in any::<usize>(),
            ) {
                bounded(move || {
                    if let Ok(cof) = Cof::parse(&data) {
                        let _ = cof.component_at(d, f, s);
                        let _ = cof.component_at(d % 4, f % 4, s % 4);
                    }
                });
            }
        }
    }
}
