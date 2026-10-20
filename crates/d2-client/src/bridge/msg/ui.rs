//! S→C 0x5D quest status, 0x63 waypoint menu and 0x77 UI action: each
//! handler does the model part of its base-game handler and emits the rest as
//! one output for the UI layer, which runs the
//! dispatch at delivery (`crate::ui::msg_ui`). [`quest_row`] is the
//! 0x5D dispatch both sides use.

use super::super::dispatch::{HandlerError, Message};
use super::super::output::Output;
use super::super::world::{ClientWorld, MONSTER};
use super::lighting::eclipse;
use super::Bytes;

/// One row of the 0x5D dispatch, first match wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuestRow {
    /// Model: every monster of set S of this class gets
    /// `quest_untargetable` (step 4): 146 `cain1`, 527 `drehyaiced`.
    Untargetable(u32),
    /// Model: the eclipse.
    Eclipse,
    /// Model: `exit_requested` := 1.
    ExitRequested,
    /// Output: a screen message of this string id.
    ScreenMessage(u16),
    /// Output: the monster effect 211.
    MonsterEffect211,
    /// Output: UI sound requests, in order.
    Sounds(&'static [i32]),
    /// Output: UI sound request of the extra value v.
    SoundExtra,
    /// Output: f bit 1, c 23 (the act 5 end: expansion flag or the
    /// video-5 path).
    ActEnd,
    /// Output: the video-7 flag := 1.
    Video7,
    /// Output: := v, then the quest-log tail.
    SetThenTail,
    /// Output: the Den of Evil counter path (step 7).
    DenCounter,
    /// Output: the quest-log tail T (step 6).
    Tail,
    /// Nothing.
    Nothing,
}

impl QuestRow {
    /// Rows marked "output" become a `QuestUi` output (step 5).
    pub fn is_output(self) -> bool {
        !matches!(
            self,
            QuestRow::Untargetable(_)
                | QuestRow::Eclipse
                | QuestRow::ExitRequested
                | QuestRow::Nothing
        )
    }
}

/// `cain1` (`monstats` row 146).
pub const CAIN1: u32 = 146;
/// `drehyaiced` (`monstats` row 527).
pub const DREHYA_ICED: u32 = 527;

/// The 0x5D dispatch for chain c and flags f.
pub fn quest_row(c: u8, f: u8) -> QuestRow {
    use QuestRow::*;
    if f & 1 != 0 {
        match c {
            3 => ScreenMessage(3708),
            4 => Untargetable(CAIN1),
            10 => Eclipse,
            13 => MonsterEffect211,
            15 => ScreenMessage(3710),
            23 => ExitRequested,
            33 => Sounds(&[237]),
            _ => Nothing,
        }
    } else if f & 2 != 0 {
        match c {
            4 => Sounds(&[241]),
            8 | 15 | 18 | 22 | 35 => Sounds(&[7]),
            23 => ActEnd,
            32 => Sounds(&[217]),
            33 => Sounds(&[243]),
            36 => Video7,
            _ => Nothing,
        }
    } else if f == 0x10 {
        match c {
            10 => Sounds(&[2456, 2474]),
            33 => SoundExtra,
            _ => Nothing,
        }
    } else if f & 0x20 != 0 {
        match c {
            32 => SetThenTail,
            33 => Untargetable(DREHYA_ICED),
            _ => DenCounter,
        }
    } else {
        Tail
    }
}

/// 0x5D QuestItemState: chain u8@1, flags u8@2, status u8@3, extra
/// i16@4. The model rows run at receive (step 3); every output row
/// becomes one `QuestUi` output (step 5).
pub fn quest_status(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 6 {
        return Err(HandlerError::Invalid("0x5D is 6 bytes"));
    }
    let (chain, flags, status) = (b.u8(1)?, b.u8(2)?, b.u8(3)?);
    let extra = b.u16(4)? as i16;
    let row = quest_row(chain, flags);
    match row {
        QuestRow::Untargetable(class) => {
            // Step 4: only units in S at receive; the order of the visit
            // does not change a flag set.
            for u in w.units.values_mut() {
                if u.key.unit_type == MONSTER && u.class == class {
                    u.quest_untargetable = true;
                    u.flag_2 = Some(false);
                }
            }
        }
        QuestRow::Eclipse => eclipse(w)?,
        QuestRow::ExitRequested => w.exit_requested = true,
        _ if row.is_output() => msg.out.push(Output::QuestUi {
            chain,
            flags,
            status,
            extra,
        }),
        _ => {}
    }
    Ok(())
}

/// 0x63 WaypointMenu: object GUID u32@1, the 16-byte record
/// @5. No model state; one `WaypointMenu` output.
pub fn waypoint_menu(_: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 21 {
        return Err(HandlerError::Invalid("0x63 is 21 bytes"));
    }
    let mut record = [0u8; 16];
    record.copy_from_slice(b.slice(5, 16)?);
    msg.out.push(Output::WaypointMenu {
        guid: b.u32(1)?,
        record,
    });
    Ok(())
}

/// 0x77 TradeAction (4): code u8@1. No model state; one
/// `TradeAction` output with the dead test captured: 1 with no local
/// player or when its mode is 0x11 (dead).
pub fn trade_action(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 2 {
        return Err(HandlerError::Invalid("0x77 is 2 bytes"));
    }
    let dead_or_absent = w
        .local_player
        .and_then(|k| w.units.get(&k))
        .is_none_or(|u| u.mode == 0x11);
    msg.out.push(Output::TradeAction {
        code: b.u8(1)?,
        dead_or_absent,
    });
    Ok(())
}
