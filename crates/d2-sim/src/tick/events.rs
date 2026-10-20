//! Timer event types and the default-handler dispatch data of the base game, as
//! data. What each handler does is owned by the unit, monster, missile,
//! object and item (open question 3); an
//! [`super::EventDispatch`] implementation plugs them in.

/// Event type numbers (D2MOO 1.10f names).
pub mod event {
    pub const MODE_CHANGE: u8 = 0;
    pub const END_ANIM: u8 = 1;
    pub const AI_THINK: u8 = 2;
    pub const STAT_REGEN: u8 = 3;
    pub const TRAP: u8 = 4;
    pub const ACTIVE_STATE: u8 = 5;
    pub const FREE_HOVER: u8 = 6;
    pub const MON_UMOD: u8 = 7;
    pub const PERIODIC_SKILLS: u8 = 8;
    pub const PERIODIC_STATS: u8 = 9;
    pub const AI_RESET: u8 = 10;
    pub const DELAYED_PORTAL: u8 = 11;
    pub const REMOVE_STATE: u8 = 12;
    pub const UPDATE_TRADE: u8 = 13;
    pub const REMOVE_SKILL_COOLDOWN: u8 = 14;
}

/// Player default handler table: which types have a handler
/// (null entries do nothing).
pub const PLAYER_HANDLERS: [bool; 15] = [
    true, true, false, true, false, true, true, false, true, true, false, true, true, true, false,
];

/// Monster default handler table: which types have a handler.
pub const MONSTER_HANDLERS: [bool; 15] = [
    true, true, true, true, false, true, true, true, true, true, true, false, true, false, false,
];

/// Monster events dropped when the monster has state 1 (`STATE_FREEZE`)
/// and the freeze test is false: types 0, 1, 2, 6, 7, 9, 10, 11, 13,
/// 14. Types 3, 4, 5, 8, 12 always dispatch.
pub const MONSTER_DROPPED_WHEN_FROZEN: [bool; 15] = [
    true, true, true, false, false, false, true, true, false, true, true, true, false, true, true,
];

/// Whether the player default handler has an entry for `event`.
pub fn player_has_handler(event: u8) -> bool {
    PLAYER_HANDLERS
        .get(event as usize)
        .copied()
        .unwrap_or(false)
}

/// Whether the monster default handler has an entry for `event`.
pub fn monster_has_handler(event: u8) -> bool {
    MONSTER_HANDLERS
        .get(event as usize)
        .copied()
        .unwrap_or(false)
}

/// Whether a monster event of type `event` is subject to the freeze drop.
pub fn monster_dropped_when_frozen(event: u8) -> bool {
    MONSTER_DROPPED_WHEN_FROZEN
        .get(event as usize)
        .copied()
        .unwrap_or(false)
}
