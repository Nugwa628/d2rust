//! The item timer event 3 handler and its replenish step
//! Durability (`replenish_durability`, stat 252) and
//! quantity (`replenish_quantity`, stat 253) regrow over time, and the
//! chain ends when the value is full.
//!
//! The item facts and the effects (stat total and base, the stat update
//! to the owner's client, the repair, the timer) are reached through
//! [`ReplenishItem`].

/// Stat 252 `item_replenish_durability`.
pub const STAT_REPLENISH_DURABILITY: u16 = 252;
/// Stat 253 `item_replenish_quantity`.
pub const STAT_REPLENISH_QUANTITY: u16 = 253;
/// Stat 72 `durability`.
pub const STAT_DURABILITY: u16 = 72;
/// Stat 70 `quantity`.
pub const STAT_QUANTITY: u16 = 70;
/// Minimum reschedule delay in frames.
pub const MIN_DELAY: i32 = 125;

/// The item acts on.
pub trait ReplenishItem {
    /// The current frame.
    fn frame(&self) -> i32;
    /// The game's tick rate: durations stated in base ticks are
    /// converted through it.
    /// No default, so no world runs at the base rate by accident.
    fn tick_rate(&self) -> crate::tick::TickRate;
    /// The item's total of `stat` (layer 0).
    fn total(&self, stat: u16) -> i32;
    /// Sets the base value of `stat`.
    fn set_base(&mut self, stat: u16, value: i32);
    /// Item flag 0x100: broken.
    fn broken(&self) -> bool;
    /// The item has durability.
    fn has_durability(&self) -> bool;
    /// Maximum durability.
    fn max_durability(&self) -> i32;
    /// The item is stackable.
    fn stackable(&self) -> bool;
    /// Maximum stack.
    fn max_stack(&self) -> i32;
    /// The stat update with (client, item, 1, stat, v, 0) when the
    /// owner GUID is not −1 and names a player with a
    /// client; the implementation tests that.
    fn notify_owner(&mut self, stat: u16, value: i32);
    /// Repair.
    fn repair(&mut self);
    /// Schedules event 3 at `frame`, args (0, 0).
    fn reschedule(&mut self, frame: i32);
}

/// Delay of the reschedule: `max(2500 / r + 1, 125)` (integer division).
pub fn delay(r: i32) -> i32 {
    (2500 / r + 1).max(MIN_DELAY)
}

/// Replenish: returns 0 for
/// no reschedule (rate 0), 1 otherwise (full: no reschedule either).
pub fn replenish<I: ReplenishItem>(item: &mut I, rate: u16, value: u16, max: i32) -> i32 {
    let r = item.total(rate);
    if r == 0 {
        return 0;
    }
    let v = item.total(value).wrapping_add(1);
    if v > max {
        return 1;
    }
    item.set_base(value, v);
    item.notify_owner(value, v);
    if v >= 1 && item.broken() {
        item.repair();
    }
    let at = item.frame().wrapping_add(item.tick_rate().ticks(delay(r)));
    item.reschedule(at);
    1
}

/// The event 3 handler for an item.
pub fn handler<I: ReplenishItem>(item: &mut I) {
    let mut ran = 0;
    if !item.broken() && item.has_durability() {
        let max = item.max_durability();
        ran = replenish(item, STAT_REPLENISH_DURABILITY, STAT_DURABILITY, max);
    }
    if ran == 0 && item.stackable() {
        let max = item.max_stack();
        replenish(item, STAT_REPLENISH_QUANTITY, STAT_QUANTITY, max);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[derive(Default)]
    struct Fake {
        frame: i32,
        stats: BTreeMap<u16, i32>,
        broken: bool,
        durable: bool,
        max_dur: i32,
        stack: bool,
        max_stack: i32,
        notes: Vec<(u16, i32)>,
        repairs: u32,
        scheduled: Vec<i32>,
    }

    impl ReplenishItem for Fake {
        fn tick_rate(&self) -> crate::tick::TickRate {
            crate::tick::TickRate::BASE
        }
        fn frame(&self) -> i32 {
            self.frame
        }
        fn total(&self, s: u16) -> i32 {
            self.stats.get(&s).copied().unwrap_or(0)
        }
        fn set_base(&mut self, s: u16, v: i32) {
            self.stats.insert(s, v);
        }
        fn broken(&self) -> bool {
            self.broken
        }
        fn has_durability(&self) -> bool {
            self.durable
        }
        fn max_durability(&self) -> i32 {
            self.max_dur
        }
        fn stackable(&self) -> bool {
            self.stack
        }
        fn max_stack(&self) -> i32 {
            self.max_stack
        }
        fn notify_owner(&mut self, s: u16, v: i32) {
            self.notes.push((s, v));
        }
        fn repair(&mut self) {
            self.repairs += 1;
            self.broken = false;
        }
        fn reschedule(&mut self, f: i32) {
            self.scheduled.push(f);
        }
    }

    fn item() -> Fake {
        Fake {
            frame: 1000,
            durable: true,
            max_dur: 20,
            stack: true,
            max_stack: 50,
            ..Fake::default()
        }
    }

    #[test]
    fn durability_chain_grows_and_ends_when_full() {
        let mut i = item();
        i.stats.insert(STAT_REPLENISH_DURABILITY, 10);
        i.stats.insert(STAT_DURABILITY, 18);
        handler(&mut i);
        // 18 + 1, notify, reschedule at f + max(2500/10 + 1, 125).
        assert_eq!(i.stats[&STAT_DURABILITY], 19);
        assert_eq!(i.notes, [(STAT_DURABILITY, 19)]);
        assert_eq!(i.scheduled, [1000 + 251]);
        handler(&mut i);
        assert_eq!(i.stats[&STAT_DURABILITY], 20);
        // Full: v = 21 > max, return 1 → no reschedule, the stack rule is
        // skipped because the first part returned 1.
        i.scheduled.clear();
        handler(&mut i);
        assert_eq!(i.stats[&STAT_DURABILITY], 20);
        assert!(i.scheduled.is_empty());
        assert!(!i.stats.contains_key(&STAT_QUANTITY));
    }

    #[test]
    fn rate_zero_falls_through_to_quantity() {
        let mut i = item();
        i.stats.insert(STAT_REPLENISH_QUANTITY, 1000);
        i.stats.insert(STAT_QUANTITY, 3);
        handler(&mut i);
        // Durability rate 0 → returns 0, the quantity part runs; minimum
        // delay 125 (2500/1000 + 1 = 3).
        assert_eq!(i.stats[&STAT_QUANTITY], 4);
        assert_eq!(i.notes, [(STAT_QUANTITY, 4)]);
        assert_eq!(i.scheduled, [1000 + 125]);
        // Quantity at max: nothing more.
        i.stats.insert(STAT_QUANTITY, 50);
        i.scheduled.clear();
        handler(&mut i);
        assert!(i.scheduled.is_empty());
        // A broken item skips durability and regrows quantity.
        let mut b = item();
        b.broken = true;
        b.stats.insert(STAT_REPLENISH_DURABILITY, 10);
        b.stats.insert(STAT_REPLENISH_QUANTITY, 10);
        handler(&mut b);
        assert!(!b.stats.contains_key(&STAT_DURABILITY));
        assert_eq!(b.stats[&STAT_QUANTITY], 1);
        // The delay is integer division with no minimum clamp on the
        // start (here: the handler's clamp).
        assert_eq!(delay(5), 501);
        assert_eq!(delay(100), 125);
    }

    #[test]
    fn replenish_repairs_a_broken_item() {
        let mut i = item();
        i.broken = true;
        i.stats.insert(STAT_REPLENISH_DURABILITY, 10);
        assert_eq!(
            replenish(&mut i, STAT_REPLENISH_DURABILITY, STAT_DURABILITY, 20),
            1
        );
        assert_eq!(i.repairs, 1);
        assert_eq!(i.stats[&STAT_DURABILITY], 1);
        assert_eq!(
            replenish(&mut i, STAT_REPLENISH_DURABILITY, STAT_DURABILITY, 0),
            1
        );
        assert_eq!(i.stats[&STAT_DURABILITY], 1);
    }
}
