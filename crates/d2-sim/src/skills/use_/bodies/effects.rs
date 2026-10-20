//! The calls of the batch 2 and 3 bodies into systems other modules own
//! that return nothing the bodies read: one [`BodyEffect`] per call
//! callee, sent through [`super::BodyWorld::effect`], and the path
//! operations ([`PathOp`], [`super::BodyWorld::path_op`]). Each variant
//! names its operation; the provider performs the call (`Pending` on the
//! wired host where no provider exists yet).

/// A call with no result the bodies read (`U` unit, `I` item, `R` room).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyEffect<U, I, R> {
    /// Remove the exhausted equipped stack after its final missile is created.
    ExhaustStack { owner: U, item: I },
    // ---- D2RUST faults (no call in the game's rules)
    /// The lightning fan / ring loop with a
    /// progressive step ≤ 0, which never ends in the game's rules (Edge case 1):
    /// D2RUST stopped it and reports the fault.
    EndlessProgressive { unit: U, skill: i32, step: i32 },
    // ---- monsters
    /// Owner data (owner GUID, owner type, a, b) (no
    /// owner: GUID −1, type 6).
    OwnerData {
        m: U,
        owner: Option<U>,
        a: i32,
        b: i32,
    },
    /// Umod.
    Umod { m: U, umod: i32, arg: i32 },
    /// AI params (p0, p1, p2) of m's AI control (−666
    /// = unchanged).
    AiParams { m: U, p0: i32, p1: i32, p2: i32 },
    /// The AI refresh of m (its think event updated).
    AiRefresh(U),
    /// An AI command {type, x, y, tx, ty} made current (through the
    /// command copy).
    AiCommand {
        m: U,
        kind: i32,
        x: i32,
        y: i32,
        tx: i32,
        ty: i32,
    },
    /// Leash owner.
    LeashOwner { m: U, owner: Option<U> },
    /// Target override.
    TargetOverride { m: U, kind: i32, guid: i32 },
    /// Target override cleared.
    ClearTargetOverride(U),
    /// Player mode request, unit form
    /// with the entry of `skill`, `mode`, the target's type / GUID,
    /// re-entry 1.
    UnitModeRequest {
        u: U,
        skill: i32,
        mode: i32,
        target: U,
    },
    /// Component byte k of m's monster data.
    Component { m: U, k: i32, v: i32 },
    /// Alignment.
    Alignment { u: U, a: i32, v: i32 },
    /// The pack dissolved.
    DissolvePack(U),
    /// `u` leaves its leader's minion list.
    LeaveLeader(U),
    /// `m` prepended to the leader's minion list.
    AddMinion { leader: U, m: U },
    /// Summon equipment.
    Equipment {
        owner: U,
        m: U,
        skill: i32,
        lvl: i32,
        ilvl: i32,
    },
    /// Pet list add (owner, pet, t, max).
    PetAdd { owner: U, pet: U, t: i32, max: i32 },
    /// Target-node insert after the head of game list `slot`
    /// (the caller tested the target slot and type).
    NodeInsert { m: U, slot: i32 },
    /// Target-list prepend.
    NodePrepend { u: U, slot: i32 },
    /// Target-list remove.
    NodeRemove(U),
    /// The owner's linked unit GUID := m's (−1 none).
    SetLinked { owner: U, m: Option<U> },
    /// The replaced linked unit is released.
    KillReplaced(U),
    /// Kill.
    Kill { u: U, a: i32, b: i32 },
    /// Unit removal.
    RemoveUnit(U),
    /// Room delete record.
    RoomDelete(U),
    /// An item leaves its room.
    ItemLeaveRoom(U),
    /// Item unit mode := v.
    ItemMode { item: U, mode: i32 },
    /// Equip.
    Equip { m: U, item: U, loc: i32 },
    /// Free the unit's combat records for `target`.
    FreeCombat { u: U, target: U },
    /// Item stat lists toggled.
    ItemLists { u: U, item: I, on: bool },
    /// Item break.
    BreakItem { u: U, item: I },
    /// Skill select.
    SelectSkill {
        u: U,
        side: i32,
        skill: i32,
        owner: i32,
    },
    /// The entry
    /// added when missing, its base level, the passive's stat refresh,
    /// a player's resync.
    SetSkill { m: U, skill: i32, lvl: i32 },
    /// The left hand skill cleared (−1).
    Op643C50(U),
    /// Set the source type and GUID, or zero both for none.
    SourceFields { m: U, owner: Option<U> },
    // ---- messages, sounds, world
    /// S→C 0xA3 record queued.
    MsgA3 {
        u: U,
        target: Option<U>,
        skill: i32,
        lvl: i32,
        x: i32,
        y: i32,
        v: i32,
    },
    /// S→C 0xA5 {unit, skill} queued on the unit.
    MsgA5 { u: U, skill: i32 },
    /// S→C 0x7F about m to the unit's client.
    AllyInfo { u: U, m: U },
    /// Object operate.
    OperateObject { u: U, object: U },
    /// Auto pickup.
    AutoPickup { u: U, item: U },
    /// Sound event.
    Sound { u: U, id: i32 },
    /// Item drop at `at` (code, quality).
    DropItem { at: U, code: [u8; 4], quality: i32 },
    /// Treasure drop.
    TreasureDrop { corpse: U, killer: U, q: i32 },
    /// Missile hit handler.
    MissileHit { missile: U, unit: U },
    /// Set the missile's first auxiliary data word.
    MissileData28 { missile: U, v: i32 },
    /// Set the missile's second auxiliary data word.
    MissileData2C { missile: U, v: i32 },
    // ---- rooms and collision
    /// Pattern stamp.
    PatternStamp {
        room: R,
        x: i32,
        y: i32,
        u: U,
        mask: u32,
    },
    /// Pattern clear.
    PatternClear {
        room: R,
        x: i32,
        y: i32,
        u: U,
        mask: u32,
    }, // ---- batch 4 (`bodies-3.md`, `bodies-4.md`)
    /// Pattern stamp (room, x, y, pattern, mask) with a fixed
    /// pattern number.
    PatternStampN {
        room: R,
        x: i32,
        y: i32,
        pattern: i32,
        mask: u32,
    },
    /// Pattern clear (room, x, y, pattern, mask) with a fixed
    /// pattern number.
    PatternClearN {
        room: R,
        x: i32,
        y: i32,
        pattern: i32,
        mask: u32,
    },
    /// Dead-body footprint.
    DeadFootprint(U),
    /// Evil-killed counter − 1.
    EvilCounterDec(U),
    /// Quest chain link (u, room, 0).
    QuestChainLink { u: U, room: R },
    /// Kill.
    KillBy { u: U, killer: Option<U>, b: i32 },
    /// Pet remove (owner, GUID, kill).
    PetRemove { owner: U, guid: i32, kill: bool },
    /// Item use by node kind: 1 → use the item at its position, 2 → the
    /// second item-use form.
    UseItem { u: U, item: I, kind: i32 },
    /// Monster mode request built: m's used
    /// skill is cleared and the record holds no target and point.
    ModeRequestBuild { m: U, mode: i32 },
    /// The built request (mode `mode`) sent with
    /// `flag`: the mode set; with `flag`, m's used skill is cleared again
    /// when m is in `mode` after it.
    ModeRequestSend { m: U, mode: i32, flag: bool },
    /// Copy the AI command kind, target type, GUID and frame to every
    /// minion of `unit`'s minion owner.
    MinionCommand {
        unit: U,
        kind: i32,
        ty: i32,
        guid: u32,
        frame: i32,
    },
    /// Class change (t, class, mode).
    ClassChange { t: U, class: i32, mode: i32 },
    /// Think in `frames` frames
    /// (the "wait N" tactic).
    WaitThink { m: U, frames: i32 },
    /// Every-tick event (u, kind, a1, a2).
    EveryTick { u: U, kind: i32, a1: i32, a2: i32 },
    /// The unit's path takes `from`'s precise position and room
    /// (`from`).
    PathFollow { u: U, from: U },
}

/// A path operation on the unit's path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathOp<U> {
    /// Move-test mask.
    MoveMask(u32),
    /// Footprint mask.
    FootprintMask(u32),
    /// Target point.
    TargetPoint(i32, i32),
    /// Unit path target (some, or none).
    TargetUnit(Option<U>),
    /// Path type.
    Type(u8),
    /// Velocity.
    Velocity(i32),
    /// Compute; [`super::BodyWorld::path_op`]
    /// returns its result.
    Compute,
    /// Snap to the sub-tile centre.
    SnapCenter,
    /// Set both path step counts.
    Steps(i32),
    Op649070(i32),
    Op648E40(i32),
    /// Clear the path's saved direction coordinates.
    Clear14,
    /// Direction toward (x, y) set on the path.
    Face(i32, i32),
    /// Path reset.
    Reset,
    /// Turn toward (x, y).
    TurnToward(i32, i32),
}
