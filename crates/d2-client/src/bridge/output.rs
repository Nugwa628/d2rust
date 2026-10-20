//! Client outputs: the base-game UI and sound calls an S→C handler
//! makes, carried out of the bridge as [`Output`]s. A handler appends
//! them to its message's sink ([`Outputs`]); the receive path moves them
//! to the bridge's list in message order, and the bridge hands the list
//! over whole at the end of the frame. [`dispatch`] applies
//! a list in order, each output to its one consumer (step 5). Consumers
//! never write the model (step 6).

use std::cell::RefCell;

use super::objects::{ObjFx, ObjSound};
use super::world::{RosterRecord, UnitKey};

/// One output: a base-game UI, sound or client-effect entry point with the
/// values it read when the handler ran (3). Each variant is
/// owned by its producer ([`ROWS`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    /// S→C 0x2C: the event unit's key,
    /// class and position x, y at receive (`None`: the
    /// unit has no position), and the event.
    ServerSound {
        unit: UnitKey,
        class: u32,
        at: Option<(u16, u16)>,
        event: u16,
    },
    /// S→C 0x5D: chain, flags, status, extra.
    QuestUi {
        chain: u8,
        flags: u8,
        status: u8,
        extra: i16,
    },
    /// S→C 0x63: the waypoint object's GUID and
    /// the 16-byte record as received.
    WaypointMenu { guid: u32, record: [u8; 16] },
    /// S→C 0x77: the code, and the dead test
    /// captured at receive: no local player, or its mode is 0x11
    /// (dead).
    TradeAction { code: u8, dead_or_absent: bool },
    /// S→C 0x26: the record fields, whether
    /// the unit was in S, and a present player's name.
    ChatLine {
        kind: u8,
        lang: u8,
        unit: UnitKey,
        b8: u8,
        b9: u8,
        /// The name as copied (at most 16 bytes, up to its NUL).
        name: Vec<u8>,
        /// The text as copied (at most 256 bytes, up to its NUL).
        text: Vec<u8>,
        present: bool,
        /// The `name` of a present player unit (type 0).
        player_name: Option<[u8; 16]>,
    },
    /// S→C 0x27: the 40 bytes, whether the unit
    /// of type 1 or 2 was in S, and a type-2 object's class (0 when
    /// absent).
    NpcText {
        bytes: [u8; 40],
        present: bool,
        object_class: u32,
    },
    /// An `hst ` / `qf2 ` placed in page 3 of the local player by an S→C
    /// 0x9C (actions 0x04, 0x0B, 0x0C; 0x15 with header mode 0): the
    /// Horadric animation start with (code) is the UI's.
    HoradricItem { code: [u8; 4] },
    /// A cursor write of an S→C 0x9C / 0x9D on the local player:
    /// The base game's handler then calls
    /// the cursor set with (item) even
    /// when the cursor item is unchanged (placement, belt): state, type
    /// and frame reset.
    CursorRefresh { item: bool },
    /// S→C 0x4E.
    HireOffer { name: u16, seed: u32 },
    /// S→C 0x4F.
    HireListReset,
    /// S→C 0x50 codes 1–4, 23, 36.
    QuestSpecial { code: u16, words: [u16; 6] },
    /// S→C 0x58.
    OpenUi { guid: u32, code: u8, arg: u8 },
    /// S→C 0x78.
    TradePartner { name: [u8; 16], guid: u32 },
    /// S→C 0x8A.
    NpcInteract {
        unit: UnitKey,
        present: bool,
        class: u32,
        /// Monster data +0x3C; `None`: no monster U.
        /// Its only writer is the 0xAC create with the bit-stream value,
        /// −1 when not sent
        /// (`msg/units.rs`).
        mdata_3c: Option<i32>,
        /// S holds an object of class 318 in mode 2.
        blocker_open: bool,
    },
    /// S→C 0x91.
    NpcIntro { slots: [u16; 12] },
    /// S→C 0x29.
    GameQuestFlags { record: [u8; 96] },
    /// S→C 0x52.
    QuestLog { status: [u8; 41] },
    /// S→C 0x5E.
    QuestAvailability { bytes: [u8; 37] },
    /// S→C 0x9B.
    MercRevive { state: u16, value: u16 },
    /// S→C 0x99, 0x9A.
    SkillEvent {
        unit: UnitKey,
        skill: u16,
        level: u8,
        target: SkillTarget,
        w: u16,
    },
    /// S→C 0xA3.
    SkillDo {
        unit: UnitKey,
        target: Option<UnitKey>,
        skill: u16,
        level: i16,
        x: u32,
        y: u32,
        v: u8,
    },
    /// The shrine functions of S→C 0x0E code 3 and 0x4D code 0x15.
    ShrineFx {
        kind: ShrineFxKind,
        code: u8,
        object: UnitKey,
        player: Option<UnitKey>,
        /// Entry +0x08, +0x0C (−1 = none).
        overlays: [i32; 2],
        /// The object's mode when the function runs (after 0x0E's mode
        /// change): the on-mode function creates the overlays only in
        /// mode 0.
        mode: u32,
    },
    /// The shrine sound of S→C 0x4D code 0x15 (step 4).
    ShrineSound { sound: u32, player: UnitKey },
    /// S→C 0x11.
    UnitOverlay {
        unit: UnitKey,
        overlay: u16,
        mode: u8,
        /// 0 (none), 396 or 397.
        sound: u32,
    },
    /// S→C 0x57: the nine umod bytes and
    /// flags bit 3.
    UmodFx {
        unit: UnitKey,
        umods: [u8; 9],
        flag8: bool,
    },
    /// S→C 0x73: the local player's key and
    /// the message fields.
    ClientMissile {
        owner: Option<UnitKey>,
        class: u16,
        f07: u32,
        f0b: u32,
        f0f: u32,
        f13: u32,
        f17: u16,
        source: UnitKey,
        f1e: u8,
        f1f: u8,
    },
    /// S→C 0x7E: the act index.
    CommonCof { act: u32 },
    /// S→C 0xA4.
    MonsterPreload { class: u16 },
    /// S→C 0x5B, 0x5C, 0x65: the active roster
    /// records after the message.
    RosterChanged { roster: Vec<RosterRecord> },
    /// S→C 0xA5.
    SkillEndFx {
        unit: UnitKey,
        skill: u16,
        srvdofunc: i16,
    },
    /// S→C 0x28 type 6.
    QuestFlags { record: [u8; 96] },
    /// S→C 0x28, unit absent.
    NpcGone { guid: u32 },
    /// S→C 0x28, unit present.
    NpcDialog(Box<NpcDialog>),
    /// S→C 0x62.
    NpcDialogEnd { kind: u8 },
    /// S→C 0x2A.
    NpcTransaction { bytes: [u8; 15], gold: i32 },
    /// S→C 0x5A: the 40 bytes (name cut) and
    /// the local player's name.
    EventText {
        bytes: [u8; 40],
        local_name: Option<[u8; 16]>,
    },
    /// S→C 0x61.
    ActVideo { video: u8 },
    /// S→C 0x76.
    OverheadClear { unit: UnitKey },
    /// S→C 0x7B.
    HotkeyAssign {
        slot: u8,
        skill: i32,
        left: bool,
        item: u32,
    },
    /// S→C 0xB4: the mapped error number n
    /// of the join error.
    JoinRefused { error: u8 },
    /// The update pass's town exit (delivery): the local player's key and
    /// the GUIDs of every monster (type 1) in S, in key order.
    TownExit { player: UnitKey, monsters: Vec<u32> },
    /// One effect call of state on / hooks / off:
    /// the unit, the state, the phase, whether the bit was
    /// set before, the unit's dead test, the hook number (setfunc /
    /// remfunc; 0 = none) and the two hook values the model captured.
    StateFx {
        unit: UnitKey,
        state: u16,
        phase: StatePhase,
        was_set: bool,
        dead: bool,
        hook: u8,
        values: [i32; 2],
    },
    /// A model unit free (S→C 0x0A and every other free path), in free
    /// order: the audio
    /// layer detaches the unit's requests without force.
    /// `client_only`: a unit of set C.
    UnitFreed { unit: UnitKey, client_only: bool },
    /// The client object update's audio calls (mode sound calls,
    /// requests, the player event sound), in
    /// update order.
    ObjectSound(ObjSound),
    /// The client object update's effect calls (graphics refresh and
    /// loads, overlays, lights, the client
    /// skill start), in update order.
    ObjectFx(ObjFx),
    /// The client missiles' sound calls,
    /// in update order.
    MissileSound(super::client_missiles::MissileSound),
}

/// The phase of a `StateFx`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatePhase {
    /// State on.
    On,
    /// State on hooks.
    Hooks,
    /// State off.
    Off,
}

/// The target of a skill event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillTarget {
    Unit(UnitKey),
    Point(u16, u16),
}

/// Which shrine function a `ShrineFx` runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShrineFxKind {
    /// Entry +0x00, run by a code-3 mode change.
    OnMode,
    /// Entry +0x04, run by a code-0x15 request.
    OnUse,
}

/// The NPC classes with (class) holds.
pub const CLASSES_4B1A10: [u32; 13] = [
    146, 251, 266, 331, 377, 378, 406, 408, 521, 527, 537, 538, 539,
];

/// With (class): 1 when `class` is one
/// of [`CLASSES_4B1A10`], else 0.
pub fn f4b1a10(class: u32) -> u32 {
    u32::from(CLASSES_4B1A10.contains(&class))
}

/// The payload of `NpcDialog`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NpcDialog {
    pub kind: u8,
    pub guid: u32,
    pub quest_flags: [u8; 96],
    pub unit: UnitKey,
    pub class: u32,
    /// The monstats `interact` flag of the class (bit 9).
    pub interact: bool,
    /// With (class): 1 for a class of [`CLASSES_4B1A10`],
    /// else 0; `None`: not captured.
    pub f4b1a10: Option<u32>,
    /// The local player has a cursor item.
    pub cursor_item: bool,
    /// The local player's level (base stat 12) at receive:
    /// the menu uses it even when a later message of the same
    /// chunk changes the stat.
    pub level: i32,
    /// The number of unidentified items of the local player at receive.
    pub unidentified: u32,
    /// The game is an expansion game (`ClientWorld::expansion` ≠ 0) at receive.
    pub expansion: bool,
    /// The S monsters whose monstats `npc` flag (bit 8) is set, in key
    /// order.
    pub npc_monsters: Vec<UnitKey>,
}

/// Who applies an output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Consumer {
    Ui,
    Audio,
    /// The client effect layer (client missiles, overlays, client skill
    /// code; Phase 6).
    Effects,
}

/// Who emits a variant (table `Producer` column): an S→C message
/// handler, or the client update pass (written `update`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Producer {
    /// The first S→C id of the cell.
    Message(u8),
    /// The update pass.
    Update,
}

/// One row of the output table: variant name, producer, consumer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub variant: &'static str,
    pub producer: Producer,
    pub consumer: Consumer,
}

const fn row(variant: &'static str, producer: u8, consumer: Consumer) -> Row {
    Row {
        variant,
        producer: Producer::Message(producer),
        consumer,
    }
}

const fn update_row(variant: &'static str, consumer: Consumer) -> Row {
    Row {
        variant,
        producer: Producer::Update,
        consumer,
    }
}

use Consumer::{Audio, Effects, Ui};

/// The variants in code, in the output table's order (checked against the
/// table).
pub const ROWS: [Row; 46] = [
    row("ServerSound", 0x2C, Audio),
    row("QuestUi", 0x5D, Ui),
    row("WaypointMenu", 0x63, Ui),
    row("TradeAction", 0x77, Ui),
    row("ChatLine", 0x26, Ui),
    row("NpcText", 0x27, Ui),
    row("HireOffer", 0x4E, Ui),
    row("HireListReset", 0x4F, Ui),
    row("QuestSpecial", 0x50, Ui),
    row("OpenUi", 0x58, Ui),
    row("TradePartner", 0x78, Ui),
    row("NpcInteract", 0x8A, Ui),
    row("NpcIntro", 0x91, Ui),
    row("GameQuestFlags", 0x29, Ui),
    row("QuestLog", 0x52, Ui),
    row("QuestAvailability", 0x5E, Ui),
    row("MercRevive", 0x9B, Ui),
    row("SkillEvent", 0x99, Effects),
    row("SkillDo", 0xA3, Effects),
    row("ShrineFx", 0x0E, Effects),
    row("ShrineSound", 0x4D, Audio),
    row("UnitOverlay", 0x11, Effects),
    row("UmodFx", 0x57, Effects),
    row("ClientMissile", 0x73, Effects),
    row("CommonCof", 0x7E, Effects),
    row("MonsterPreload", 0xA4, Effects),
    row("RosterChanged", 0x5B, Ui),
    row("SkillEndFx", 0xA5, Effects),
    row("QuestFlags", 0x28, Ui),
    row("NpcGone", 0x28, Ui),
    row("NpcDialog", 0x28, Ui),
    row("NpcDialogEnd", 0x62, Ui),
    row("NpcTransaction", 0x2A, Ui),
    row("EventText", 0x5A, Ui),
    row("ActVideo", 0x61, Ui),
    row("OverheadClear", 0x76, Ui),
    row("HotkeyAssign", 0x7B, Ui),
    row("JoinRefused", 0xB4, Ui),
    update_row("TownExit", Ui),
    row("StateFx", 0xA8, Effects),
    row("UnitFreed", 0x0A, Audio),
    update_row("ObjectSound", Audio),
    update_row("ObjectFx", Effects),
    row("HoradricItem", 0x9C, Ui),
    update_row("MissileSound", Audio),
    row("CursorRefresh", 0x9C, Ui),
];

impl Output {
    /// The variant's row in [`ROWS`].
    pub fn row(&self) -> &'static Row {
        let i = match self {
            Output::ServerSound { .. } => 0,
            Output::QuestUi { .. } => 1,
            Output::WaypointMenu { .. } => 2,
            Output::TradeAction { .. } => 3,
            Output::ChatLine { .. } => 4,
            Output::NpcText { .. } => 5,
            Output::HireOffer { .. } => 6,
            Output::HireListReset => 7,
            Output::QuestSpecial { .. } => 8,
            Output::OpenUi { .. } => 9,
            Output::TradePartner { .. } => 10,
            Output::NpcInteract { .. } => 11,
            Output::NpcIntro { .. } => 12,
            Output::GameQuestFlags { .. } => 13,
            Output::QuestLog { .. } => 14,
            Output::QuestAvailability { .. } => 15,
            Output::MercRevive { .. } => 16,
            Output::SkillEvent { .. } => 17,
            Output::SkillDo { .. } => 18,
            Output::ShrineFx { .. } => 19,
            Output::ShrineSound { .. } => 20,
            Output::UnitOverlay { .. } => 21,
            Output::UmodFx { .. } => 22,
            Output::ClientMissile { .. } => 23,
            Output::CommonCof { .. } => 24,
            Output::MonsterPreload { .. } => 25,
            Output::RosterChanged { .. } => 26,
            Output::SkillEndFx { .. } => 27,
            Output::QuestFlags { .. } => 28,
            Output::NpcGone { .. } => 29,
            Output::NpcDialog(_) => 30,
            Output::NpcDialogEnd { .. } => 31,
            Output::NpcTransaction { .. } => 32,
            Output::EventText { .. } => 33,
            Output::ActVideo { .. } => 34,
            Output::OverheadClear { .. } => 35,
            Output::HotkeyAssign { .. } => 36,
            Output::JoinRefused { .. } => 37,
            Output::TownExit { .. } => 38,
            Output::StateFx { .. } => 39,
            Output::UnitFreed { .. } => 40,
            Output::ObjectSound(_) => 41,
            Output::ObjectFx(_) => 42,
            Output::HoradricItem { .. } => 43,
            Output::MissileSound(_) => 44,
            Output::CursorRefresh { .. } => 45,
        };
        &ROWS[i]
    }

    pub fn consumer(&self) -> Consumer {
        self.row().consumer
    }
}

/// Moves the model's unit frees made since the last call to `outputs`
/// as `UnitFreed` outputs, in free order. The receive
/// path and the update pass call it after each handler (every free path
/// frees after the handler's own outputs), the bridge once more before
/// it hands the list over.
pub fn move_freed(world: &mut super::world::ClientWorld, outputs: &mut Vec<Output>) {
    outputs.extend(
        world
            .freed
            .drain(..)
            .map(|(unit, client_only)| Output::UnitFreed { unit, client_only }),
    );
}

/// The sink of one handler call: outputs in the handler's call order.
/// Shared through the message, so handlers keep their
/// signature.
#[derive(Debug, Default)]
pub struct Outputs(RefCell<Vec<Output>>);

impl Outputs {
    pub fn push(&self, o: Output) {
        self.0.borrow_mut().push(o);
    }

    /// The outputs pushed so far, in order; the sink is left empty.
    pub fn take(&self) -> Vec<Output> {
        std::mem::take(&mut *self.0.borrow_mut())
    }
}

/// Applies one frame's outputs in list order, each to its consumer
/// (steps 4–5). Never reorders, merges or drops.
pub fn dispatch<E>(
    outputs: &[Output],
    ui: &mut dyn FnMut(&Output) -> Result<(), E>,
    audio: &mut dyn FnMut(&Output) -> Result<(), E>,
    effects: &mut dyn FnMut(&Output) -> Result<(), E>,
) -> Result<(), E> {
    for o in outputs {
        match o.consumer() {
            Consumer::Ui => ui(o)?,
            Consumer::Audio => audio(o)?,
            Consumer::Effects => effects(o)?,
        }
    }
    Ok(())
}

/// A malformed table.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TableError {
    #[error("bridge.md has no §10 rows table")]
    NoTable,
    #[error(
        "§10 row {0:?}: expected | `Variant` | payload | 0xNN … or update | ui, audio or effects | owner |"
    )]
    Row(String),
}

/// One row of the output table as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableRow {
    pub variant: String,
    pub producer: Producer,
    pub consumer: Consumer,
}

/// The output table (the rows after `<!-- rows -->`); the producer is
/// the first `0xNN` of its cell, or the cell
/// `update`.
pub fn parse_table(spec: &str) -> Result<Vec<TableRow>, TableError> {
    let section = spec.split("### 10.").nth(1).ok_or(TableError::NoTable)?;
    let table = section
        .split("<!-- rows -->")
        .nth(1)
        .ok_or(TableError::NoTable)?;
    let mut rows = Vec::new();
    for line in table.lines().skip_while(|l| l.trim().is_empty()) {
        if !line.starts_with('|') {
            break;
        }
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if cells.len() < 7 || cells[1] == "Variant" || cells[1].starts_with("---") {
            continue;
        }
        let bad = || TableError::Row(line.to_owned());
        let variant = cells[1]
            .strip_prefix('`')
            .and_then(|v| v.strip_suffix('`'))
            .ok_or_else(bad)?;
        let producer = if cells[3] == "update" {
            Producer::Update
        } else {
            let hex = cells[3].get(2..4).filter(|_| cells[3].starts_with("0x"));
            hex.and_then(|h| u8::from_str_radix(h, 16).ok())
                .map(Producer::Message)
                .ok_or_else(bad)?
        };
        let consumer = match cells[4] {
            "UI" | "ui" => Consumer::Ui,
            "audio" => Consumer::Audio,
            "effects" => Consumer::Effects,
            _ => return Err(bad()),
        };
        rows.push(TableRow {
            variant: variant.to_owned(),
            producer,
            consumer,
        });
    }
    if rows.is_empty() {
        return Err(TableError::NoTable);
    }
    Ok(rows)
}

/// The step 8 check: every variant in code has exactly one row with
/// the same producer and consumer, and every row has a variant. Returns
/// the variant names that disagree.
pub fn check(table: &[TableRow], code: &[Row]) -> Vec<String> {
    let mut out = Vec::new();
    for c in code {
        let found: Vec<&TableRow> = table.iter().filter(|r| r.variant == c.variant).collect();
        if found.len() != 1 || (found[0].producer, found[0].consumer) != (c.producer, c.consumer) {
            out.push(c.variant.to_owned());
        }
    }
    for r in table {
        if !code.iter().any(|c| c.variant == r.variant) {
            out.push(r.variant.to_owned());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_keeps_list_order_and_routes_by_consumer() {
        let list = [
            Output::ServerSound {
                unit: UnitKey::new(1, 0x26),
                class: 3,
                at: None,
                event: 18,
            },
            Output::TradeAction {
                code: 0x10,
                dead_or_absent: false,
            },
            Output::ServerSound {
                unit: UnitKey::new(0, 1),
                class: 0,
                at: None,
                event: 2,
            },
            Output::MonsterPreload { class: 7 },
        ];
        let seen = RefCell::new(Vec::new());
        dispatch::<()>(
            &list,
            &mut |o| {
                seen.borrow_mut().push(("ui", o.clone()));
                Ok(())
            },
            &mut |o| {
                seen.borrow_mut().push(("audio", o.clone()));
                Ok(())
            },
            &mut |o| {
                seen.borrow_mut().push(("effects", o.clone()));
                Ok(())
            },
        )
        .unwrap();
        let seen = seen.into_inner();
        assert_eq!(
            seen.iter().map(|s| s.0).collect::<Vec<_>>(),
            ["audio", "ui", "audio", "effects"]
        );
        assert_eq!(seen.into_iter().map(|s| s.1).collect::<Vec<_>>(), list);
    }
}
