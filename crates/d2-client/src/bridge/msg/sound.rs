//! S→C 0x2C PlaySound: no model state; the event unit is looked up in
//! set S and one `ServerSound` output carries
//! its key, class and position as read now, with the
//! event. The audio layer applies the event table
//! at delivery.

use super::super::dispatch::{HandlerError, Message};
use super::super::output::Output;
use super::super::world::{ClientWorld, UnitKey};
use super::Bytes;

/// 0x2C PlaySound: unit type u8@1, GUID u32@2, event u16@6;
/// a unit not in S → nothing.
pub fn play_sound(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 8 {
        return Err(HandlerError::Invalid("0x2C is 8 bytes"));
    }
    let key = UnitKey::new(b.u8(1)?, b.u32(2)?);
    let event = b.u16(6)?;
    if let Some(u) = w.units.get(&key) {
        msg.out.push(Output::ServerSound {
            unit: key,
            class: u.class,
            at: u.position,
            event,
        });
    }
    Ok(())
}
