//! The item-use dispatcher on the inventory model (the `use_item_at` /
//! `use_item` seams) and the Town Portal entry.
//!
//! [`InvDesk::dispatch_use`] picks the entry n from the books row (`pSpell`,
//! extra = `BookSkill`) or the items row (`pSpell`), runs the first / second
//! use with item flag 0x4, and does the failure reset with S→C 0x7C on a
//! refused use. Entry 2 (Town Portal) is the portal cast, run whole by the
//! host that owns the objects and the path code
//! ([`LifecycleHooks::town_portal_cast`]: the action wiring's
//! `View::town_portal_cast`, with the pair made in the call). Without such
//! a host the desk runs the cast up to its town refusal and records a cast
//! that passes it as a request
//! ([`InvState::portal_requests`](super::InvState::portal_requests)) for
//! the host to take after the call. Its cost (the scroll's skill count
//! and consumption, a tome's charge) is the caller's (the item-move
//! handlers), paid only for a 1.
//!
//! Entry 3 (healing and mana potions) is the potion body
//! ([`InvDesk::use_entry3`]). The other entries keep their earlier
//! answers: stamina and rejuvenation (D2RUST's own) potions and
//! identify on `use_item`, everything else on the rest
//! (their bodies are not settled).
//!
//! A mod game's item use functions (`docs/plans/mods/seams.md` MS7c) run
//! before D2RUST's body: for an entry its item provider
//! [`has`](crate::items::provider::ItemProvider::has_use_fn), also one past
//! D2RUST's 30, [`ItemProvider::use_fn`](crate::items::provider::ItemProvider::use_fn)
//! answers, with the desk as its [`UseHost`]; `None` runs D2RUST's body.
//!
//! The game host runs the whole cast inside the call, and the call returns
//! the cast's own result, so a failed creation costs nothing. Only test
//! hosts use the request path, where the creation runs after the call and a
//! failed one still costs the use.

use super::{InvDesk, InvRest};
use crate::items::moves::{iflag, layouts, ty, Guid, MovePending, MoveUnits, Owner};
use crate::items::provider::{UseCall, UseHost};
use crate::units::hooks::Sim;
use crate::units::lifecycle::LifecycleHooks;
use crate::units::UnitId;

/// Town Portal scroll and tome.
pub const SCROLL: [u8; 4] = *b"tsc ";
pub const TOME: [u8; 4] = *b"tbk ";

/// Entries of the use table (31 entries; the ones that have a body here).
pub mod entry {
    /// Identify (`isc`, `ibk`): the only entry with a first word.
    pub const IDENTIFY: u32 = 1;
    /// Town Portal (`tsc`, `tbk`).
    pub const TOWN_PORTAL: u32 = 2;
    /// Healing and mana potions (`hp1`–`hp5`, `mp1`–`mp5`).
    pub const POTION: u32 = 3;
    /// The table's size.
    pub const COUNT: u32 = 31;
}

/// Pandemonium Finale: the cast's other refusal besides a town.
const LEVEL_FINALE: u32 = 136;
/// Sound 24 `notintown`, queued when the cast is refused.
const SOUND_NOT_IN_TOWN: u16 = 24;
/// Item flag 0x4: armed for use.
const ARMED: u32 = iflag::TARGETING;

impl<H: LifecycleHooks, R: InvRest + ?Sized> InvDesk<'_, '_, H, R> {
    /// The entry n of item `item` and the extra e (−1 unless a
    /// books row gives `BookSkill`); `None` for n < 1, or n ≥ 31 unless
    /// a mod game's item provider has a use function n.
    pub fn use_entry(&self, item: Guid) -> Option<(u32, i32)> {
        let rec = self.tables.item(self.record_of(item)?)?;
        let book_or_scroll = self.is_type(item, ty::BOOK) || self.is_type(item, ty::SCRO);
        let books = usize::try_from(self.spell(item))
            .ok()
            .and_then(|i| self.tables.books.get(i));
        let (n, e) = match books {
            Some(b) if book_or_scroll && b.pspell > 0 => (b.pspell, b.bookskill),
            _ => (rec.pspell, -1),
        };
        let rules = self.econ.hooks.rules();
        let modded = n > 0 && rules.items().is_some_and(|p| p.has_use_fn(n));
        let known = (1..entry::COUNT).contains(&n) || modded;
        known.then_some((n, e))
    }

    /// The dispatcher for player U using item I on target T at (x, y):
    /// true = used.
    pub fn dispatch_use(
        &mut self,
        player: Owner,
        item: Guid,
        target: Owner,
        x: i32,
        y: i32,
    ) -> bool {
        // Only unit types 0 and 1 may use items; the item must exist.
        if player.ty > 1 || self.item_unit(item).is_none() {
            return false;
        }
        // Find the use entry; items without one are not taken.
        let Some((n, e)) = self.use_entry(item) else {
            return false;
        };
        let armed = self.item_flags(item) & ARMED != 0;
        // First use: only entry 1 has a first word. It arms the item (flag
        // 0x4), sends S→C 0x3F (the spell icon, the GUID, e) and returns 1,
        // so nothing is used yet.
        if n == entry::IDENTIFY && !armed {
            let f = self.item_flags(item);
            self.set_item_flags(item, f | ARMED);
            if let Some(icon) = self.use_spell_icon(item) {
                self.send(player, layouts::use_stackable(icon, item, e as u16));
            }
            return false;
        }
        // Second use: arm the item. Entries without a second word are 12–30 only.
        let f = self.item_flags(item);
        self.set_item_flags(item, f | ARMED);
        let call = UseCall {
            entry: n,
            extra: e,
            player,
            item,
            target,
            x,
            y,
        };
        let r = match self.mod_use(&call) {
            Some(r) => r,
            None => self.builtin_use(&call),
        };
        if !r {
            self.use_failure_reset(player);
            self.send(player, layouts::item_used(Owner::ITEM, item));
        }
        r
    }

    /// The item's use entry is one a mod game's item provider answers
    /// ([`ItemProvider::has_use_fn`](crate::items::provider::ItemProvider::has_use_fn)):
    /// a used item of such an entry is consumed as a used scroll is.
    pub fn mod_use_entry(&self, item: Guid) -> bool {
        let Some((n, _)) = self.use_entry(item) else {
            return false;
        };
        let rules = self.econ.hooks.rules();
        rules.items().is_some_and(|p| p.has_use_fn(n))
    }

    /// A mod game's use function for the call's entry
    /// ([`ItemProvider::use_fn`](crate::items::provider::ItemProvider::use_fn));
    /// `None` without one, or when it leaves the entry to D2RUST.
    fn mod_use(&mut self, call: &UseCall) -> Option<bool> {
        let rules = self.econ.hooks.rules().clone();
        let p = rules.items().filter(|p| p.has_use_fn(call.entry))?;
        p.use_fn(self, call)
    }

    /// D2RUST's body of the call's entry: true = used. An entry past
    /// D2RUST's (a mod game's use function that left it) is refused.
    fn builtin_use(&mut self, call: &UseCall) -> bool {
        let UseCall {
            entry: n,
            player,
            item,
            target,
            x,
            y,
            ..
        } = *call;
        match n {
            entry::TOWN_PORTAL => {
                let (r, item_message) = self.cast_town_portal(player);
                // The cast's own S→C 0x7C for the item.
                if item_message {
                    self.send(player, layouts::item_used(Owner::ITEM, item));
                }
                r
            }
            // U drinks it, whatever the target.
            entry::POTION => self
                .unit_of(player)
                .is_some_and(|u| self.use_entry3(u, item)),
            n if n >= entry::COUNT => false,
            _ => self.use_unwritten(player, item, target, x, y),
        }
    }

    /// The entries whose bodies are not settled: the earlier answers (stamina
    /// and D2RUST's own rejuvenation potions, identify on a target, the rest).
    fn use_unwritten(&mut self, player: Owner, item: Guid, target: Owner, x: i32, y: i32) -> bool {
        if target == Owner::item(item) {
            // A potion used from the grid (0x20) is drunk by U, the
            // player, as entry 3 is (entry 3 reads only U and I).
            if self.use_potion(player, item) {
                return true;
            }
            return self.rest.use_item_at(player, item, x, y);
        }
        if target == player && self.use_potion(player, item) {
            return true;
        }
        if self.use_identify(player, target, item) {
            return true;
        }
        self.rest.use_item(player, target, item)
    }

    /// The Town Portal cast: true = 1, and whether the cast's own S→C 0x7C
    /// for the item is due. On a cast host
    /// ([`LifecycleHooks::town_portal_cast`]) the whole cast; else up to its
    /// town refusal, the rest requested (module doc).
    fn cast_town_portal(&mut self, player: Owner) -> (bool, bool) {
        let Some(p) = self.unit_of(player).filter(|_| player.is_player()) else {
            return (false, false);
        };
        if let Some((r, item_message)) = self.town_portal_cast_hook(p) {
            return (r != 0, item_message);
        }
        let level = self
            .econ
            .game
            .lists
            .unit(p)
            .and_then(|e| e.room())
            .and_then(|r| self.econ.hooks.room_level(self.econ.game, r));
        if self.in_town(player) || level == Some(LEVEL_FINALE) {
            let _ = crate::units::sound::queue_sound(self.econ.game, p, SOUND_NOT_IN_TOWN, Some(p));
            return (false, false);
        }
        self.state.portal_requests.push(p);
        (true, false)
    }

    /// The cast on the hooks with the call's units and game seed
    /// ([`LifecycleHooks::town_portal_cast`]); `None`: no cast host.
    fn town_portal_cast_hook(&mut self, player: UnitId) -> Option<(u32, bool)> {
        let e = &mut *self.econ;
        let mut sim = Sim {
            game: &mut *e.game,
            units: &mut *e.units,
            stats: &mut *e.stats,
            data: e.data,
        };
        e.hooks
            .town_portal_cast(&mut sim, &mut e.fields.seed, player)
    }

    /// The spell icon byte of a book or scroll (its books
    /// row), else the items record's `spellicon`.
    fn use_spell_icon(&self, item: Guid) -> Option<u8> {
        if self.is_type(item, ty::BOOK) || self.is_type(item, ty::SCRO) {
            let row = usize::try_from(self.spell(item)).ok()?;
            return self.tables.books.get(row).map(|b| b.spellicon);
        }
        Some(self.tables.item(self.record_of(item)?)?.spellicon)
    }

    /// Failure reset: every item of the
    /// player's inventory list with flag 0x4 loses it, each with S→C 0x3F
    /// (`3F FF`, GUID, `FF FF`).
    pub(super) fn use_failure_reset(&mut self, player: Owner) {
        let Some(items) = self
            .unit_of(player)
            .and_then(|p| self.state.inventories.get(&p))
            .map(|i| i.items().to_vec())
        else {
            return;
        };
        for u in items {
            let g = self.guid_of(u);
            let f = self.item_flags(g);
            if f & ARMED != 0 {
                self.set_item_flags(g, f & !ARMED);
                self.send(player, layouts::use_stackable(0xFF, g, 0xFFFF));
            }
        }
    }

    /// The item is a Town Portal scroll.
    pub fn is_portal_scroll(&self, item: Guid) -> bool {
        self.item_code(item) == Some(SCROLL)
    }

    fn record_of(&self, item: Guid) -> Option<usize> {
        let i = self.item_unit(item)?;
        self.state.items.get(&i).map(|d| d.record)
    }

    fn item_code(&self, item: Guid) -> Option<[u8; 4]> {
        self.tables.item(self.record_of(item)?).map(|r| r.code)
    }

    /// Item skill: a book's books `bookskill`, a scroll's `scrollskill`,
    /// else −1. No books row reads −1.
    pub fn book_item_skill(&self, item: Guid) -> i32 {
        let row = usize::try_from(self.spell(item))
            .ok()
            .and_then(|i| self.tables.books.get(i));
        match (self.primary_type(item), row) {
            (t, Some(b)) if t == ty::BOOK => b.bookskill,
            (t, Some(b)) if t == ty::SCRO => b.scrollskill,
            _ => -1,
        }
    }

    /// The Town Portal uses since the last call, by player unit.
    pub fn take_portal_requests(&mut self) -> Vec<UnitId> {
        std::mem::take(&mut self.state.portal_requests)
    }

    /// The walks to ground items since the last call (player, item,
    /// cursor flag).
    pub fn take_item_walks(&mut self) -> Vec<(UnitId, UnitId, bool)> {
        std::mem::take(&mut self.state.item_walks)
    }

    /// Whether `unit` was placed on the ground since the last
    /// [`Self::take_dropped`] (unit flag 0x1000), without taking.
    pub fn was_dropped(&self, unit: UnitId) -> bool {
        self.state.dropped.contains(&unit)
    }

    /// The items placed on the ground since the last call
    /// ([`InvState::dropped`](super::InvState::dropped)).
    pub fn take_dropped(&mut self) -> std::collections::BTreeSet<UnitId> {
        std::mem::take(&mut self.state.dropped)
    }
}

/// The desk as a mod game's item use function sees it (MS7c).
impl<H: LifecycleHooks, R: InvRest + ?Sized> UseHost for InvDesk<'_, '_, H, R> {
    fn unit_of(&self, owner: Owner) -> Option<UnitId> {
        InvDesk::unit_of(self, owner)
    }
    fn item_record(&self, item: Guid) -> Option<usize> {
        self.record_of(item)
    }
    fn item_code(&self, item: Guid) -> Option<[u8; 4]> {
        InvDesk::item_code(self, item)
    }
    fn stat(&self, unit: UnitId, stat: u16, layer: u16) -> i32 {
        self.econ.stats.unit_total(unit, stat, layer)
    }
    fn set_stat(&mut self, unit: UnitId, stat: u16, layer: u16, value: i32) {
        let s = &mut *self.econ.stats;
        s.unit_set(&mut *self.econ.hooks, unit, stat, value, layer);
    }
    fn roll(&mut self, unit: UnitId, n: i32) -> u32 {
        self.econ.units.get_mut(unit).map_or(0, |r| r.seed.roll(n))
    }
}
