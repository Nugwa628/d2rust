//! The stash and the button dispatch for C→S 0x4F ClickButton: the dispatch
//! ([`click_button`]) and the stash buttons ([`stash_button`]: close, gold
//! withdraw, gold deposit, and the clamped stat add). The packet handler
//! calls [`click_button`]; it reaches the world through [`StashWorld`].
//!
//! The cube's buttons (0x17, 0x18) are [`crate::world::cube`]'s; the
//! player-trade buttons are handled elsewhere, so the dispatch only routes
//! to them ([`ButtonRoute::Trade`]).

use crate::units::UnitId;

/// Button 0x12: stash close.
pub const BUTTON_STASH_CLOSE: u16 = 0x12;
/// Button 0x13: withdraw gold from the stash.
pub const BUTTON_WITHDRAW: u16 = 0x13;
/// Button 0x14: deposit gold into the stash.
pub const BUTTON_DEPOSIT: u16 = 0x14;
/// Button 0x17: cube close (the cube's, see [`crate::world::cube`]).
pub const BUTTON_CUBE_CLOSE: u16 = 0x17;
/// Button 0x18: cube transmute (the cube's).
pub const BUTTON_CUBE_TRANSMUTE: u16 = 0x18;

/// Interaction type 2: an object (the stash).
pub const INTERACT_OBJECT: u8 = 2;
/// Interaction type 0: a player (trade).
pub const INTERACT_PLAYER: u8 = 0;
/// The stash object class 0x10B (267).
pub const STASH_CLASS: u32 = 0x10B;
/// The stash gold cap: a constant, not read from a table.
pub const STASH_CAP: i32 = 2_500_000;
/// Carried gold per character level (level × 10000).
pub const GOLD_PER_LEVEL: i32 = 10_000;
/// Sound event 19, `impossible`.
pub const SOUND_IMPOSSIBLE: u16 = 19;

/// Stats read and written (`itemstatcost`).
pub mod stat {
    /// Character level.
    pub const LEVEL: u16 = 12;
    /// Carried gold.
    pub const GOLD: u16 = 14;
    /// Gold in the stash.
    pub const GOLDBANK: u16 = 15;
}

/// S→C 0x77 TradeAction codes of the dispatch.
pub mod trade_code {
    /// No active interaction (close trade).
    pub const CLOSE: u8 = 0x0C;
    /// Another button with a non-trade interaction (close trade(1)).
    pub const CLOSE_1: u8 = 0x0D;
}

/// The calls the stash buttons make, with their expected providers.
pub trait StashWorld {
    /// The active interaction (type, GUID); `None` when the
    /// active byte is 0 (unit record).
    fn interaction(&self, player: UnitId) -> Option<(u8, u32)>;
    /// Reset the interaction: GUID −1, type 6, inactive.
    fn reset_interaction(&mut self, player: UnitId);
    /// The scroll / tome recount of the inventory.
    fn inventory_pass(&mut self, player: UnitId);
    /// A message to the player's client.
    fn send(&mut self, player: UnitId, msg: &[u8]);
    /// The object unit with this GUID and
    /// its class.
    fn object_by_guid(&self, guid: u32) -> Option<(UnitId, u32)>;
    /// Whether the unit has a room and the room is
    /// in a town level.
    fn in_town(&self, unit: UnitId) -> bool;
    /// The full value of a stat.
    fn stat(&self, unit: UnitId, stat: u16) -> i32;
    /// Set the base value of a stat.
    fn set_base(&mut self, unit: UnitId, stat: u16, value: i32);
    /// Add `d` to the base value of a stat.
    fn add_base(&mut self, unit: UnitId, stat: u16, d: i32);
    /// The unit is a player (type 0).
    fn is_player(&self, unit: UnitId) -> bool;
    /// Drop a gold pile at the player (the overflow of [`receive`]).
    fn drop_gold(&mut self, player: UnitId, amount: i32);
    /// Play a sound event from `unit`, optionally aimed at `target`.
    fn sound(&mut self, unit: UnitId, event: u16, target: Option<UnitId>);
}

/// Where the dispatch sends a button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonRoute {
    /// Handled here: the handler result (0 and 1 enqueue the packet, 3
    /// drops it).
    Done(u32),
    /// Buttons 0x17 / 0x18 with an active interaction: the cube.
    Cube,
    /// Any other button with interaction type 0: the player-trade switch.
    Trade,
}

/// `v` = (p1 << 16) | p2 of a C→S 0x4F message (`button` u16 at 1, `p1`
/// u16 at 3, `p2` u16 at 5).
pub fn button_value(p1: u16, p2: u16) -> u32 {
    (u32::from(p1) << 16) | u32::from(p2)
}

/// The carried-gold cap: stat 12 (level) × 10000.
pub fn gold_cap<W: StashWorld + ?Sized>(w: &W, player: UnitId) -> i32 {
    w.stat(player, stat::LEVEL).wrapping_mul(GOLD_PER_LEVEL)
}

/// Dispatch a ClickButton for the player P. A missing P is the caller's to
/// handle (result 1).
pub fn click_button<W: StashWorld + ?Sized>(
    w: &mut W,
    player: UnitId,
    button: u16,
    v: u32,
) -> ButtonRoute {
    // No active interaction: checked before the button test.
    let Some((ty, guid)) = w.interaction(player) else {
        w.send(player, &[0x77, trade_code::CLOSE]);
        return ButtonRoute::Done(0);
    };
    match button {
        // Stash buttons (0x12..=0x14).
        BUTTON_STASH_CLOSE..=BUTTON_DEPOSIT => {
            if ty != INTERACT_OBJECT {
                return ButtonRoute::Done(1);
            }
            stash_button(w, player, guid, button, v as i32);
            ButtonRoute::Done(0)
        }
        // Cube buttons.
        BUTTON_CUBE_CLOSE | BUTTON_CUBE_TRANSMUTE => ButtonRoute::Cube,
        // Anything else needs a player interaction (trade).
        _ if ty != INTERACT_PLAYER => {
            w.send(player, &[0x77, trade_code::CLOSE_1]);
            ButtonRoute::Done(3)
        }
        _ => ButtonRoute::Trade,
    }
}

/// The stash buttons: the common checks, then the button. `guid`: the
/// interaction's (type 2).
pub fn stash_button<W: StashWorld + ?Sized>(
    w: &mut W,
    player: UnitId,
    guid: u32,
    button: u16,
    v: i32,
) {
    let Some((stash, class)) = w.object_by_guid(guid) else {
        return;
    };
    if class != STASH_CLASS || !w.in_town(player) || !w.in_town(stash) {
        return;
    }
    match button {
        BUTTON_STASH_CLOSE => close(w, player),
        BUTTON_WITHDRAW => withdraw(w, player, v),
        BUTTON_DEPOSIT => deposit(w, player, v),
        _ => {}
    }
}

/// Close: reset the interaction (active, type 2), then the recount;
/// nothing is sent.
fn close<W: StashWorld + ?Sized>(w: &mut W, player: UnitId) {
    if w.interaction(player)
        .is_some_and(|(ty, _)| ty == INTERACT_OBJECT)
    {
        w.reset_interaction(player);
    }
    w.inventory_pass(player);
}

/// Withdraw `v` gold from the stash (signed compares).
fn withdraw<W: StashWorld + ?Sized>(w: &mut W, player: UnitId, v: i32) {
    if v <= 0 || v > w.stat(player, stat::GOLDBANK) {
        return;
    }
    if w.stat(player, stat::GOLD).wrapping_add(v) > gold_cap(w, player) {
        w.sound(player, SOUND_IMPOSSIBLE, Some(player));
        return;
    }
    receive(w, player, v);
    clamped_add(w, player, stat::GOLDBANK, v.wrapping_neg());
}

/// Deposit `v` gold into the stash; a deposit over the stash cap fills the
/// stash to the cap and keeps the rest.
fn deposit<W: StashWorld + ?Sized>(w: &mut W, player: UnitId, v: i32) {
    if v <= 0 || v > w.stat(player, stat::GOLD) {
        return;
    }
    let cap = STASH_CAP;
    let s = w.stat(player, stat::GOLDBANK);
    if (s as u32).wrapping_add(v as u32) <= cap as u32 {
        clamped_add(w, player, stat::GOLD, v.wrapping_neg());
        clamped_add(w, player, stat::GOLDBANK, v);
    } else if (s as u32) < cap as u32 {
        let part = cap.wrapping_sub(s);
        clamped_add(w, player, stat::GOLDBANK, part);
        clamped_add(w, player, stat::GOLD, part.wrapping_neg());
    }
}

/// Receive `a` gold: at the cap
/// the whole amount drops; over it the gold is set to the cap and the
/// rest drops; else gold += a.
pub fn receive<W: StashWorld + ?Sized>(w: &mut W, player: UnitId, a: i32) {
    let cap = gold_cap(w, player);
    let g = w.stat(player, stat::GOLD);
    if g == cap {
        w.drop_gold(player, a);
    } else if (g as u32).wrapping_add(a as u32) > cap as u32 {
        w.set_base(player, stat::GOLD, cap);
        w.drop_gold(player, g.wrapping_add(a).wrapping_sub(cap));
    } else {
        w.set_base(player, stat::GOLD, g.wrapping_add(a));
    }
}

/// The clamped add of `d` to a stat: a result below 0 sets the stat to 0
/// (not the cap), and for a player a gold or stash total over its cap also
/// sets 0.
pub fn clamped_add<W: StashWorld + ?Sized>(w: &mut W, unit: UnitId, id: u16, d: i32) {
    let n = w.stat(unit, id).wrapping_add(d);
    if n < 0 {
        w.set_base(unit, id, 0);
        return;
    }
    if w.is_player(unit) {
        let over = match id {
            stat::GOLD => n > gold_cap(w, unit),
            stat::GOLDBANK => n > STASH_CAP,
            _ => false,
        };
        if over {
            w.set_base(unit, id, 0);
            return;
        }
    }
    w.add_base(unit, id, d);
}

#[cfg(test)]
mod tests;
