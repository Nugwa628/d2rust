//! Tests for the item-use dispatcher on the desk (with
//! `wiring/inventory/town_portal.rs`): on a cast host the Town Portal
//! entry runs the cast of the hooks, and 0x20 / 0x27 charge the scroll or
//! tome only when the cast made the pair.

use super::*;
use crate::items::inventory::tables::InvBookRec;
use crate::items::moves::{iflag, MovePending, MoveUnits};

const QUANTITY: u16 = 70;
/// Book of Townportal / Scroll of Townportal skills (any ids for the test).
const BOOK_SKILL: i32 = 220;
const SCROLL_SKILL: i32 = 219;

/// `record` stored at the cell (x, y) of page 0; drained.
fn stored(w: &mut World, record: usize, x: u32, y: u32) -> Guid {
    let g = w.cursor_item(record);
    assert_eq!(w.handle(&insert(g, x, y, 0)), Ok(0));
    w.drain();
    g
}

/// The 0x7C / 0x3F messages the rest was sent, in order.
fn use_msgs(w: &mut World) -> Vec<Vec<u8>> {
    std::mem::take(&mut w.rest.sent)
        .into_iter()
        .map(|(_, m)| m)
        .filter(|m| m[0] == 0x7C || m[0] == 0x3F)
        .collect()
}

fn used(g: Guid) -> Vec<u8> {
    let mut m = vec![0x7C, 4];
    m.extend_from_slice(&g.to_le_bytes());
    m
}

fn reset(g: Guid) -> Vec<u8> {
    let mut m = vec![0x3F, 0xFF];
    m.extend_from_slice(&g.to_le_bytes());
    m.extend_from_slice(&[0xFF, 0xFF]);
    m
}

/// A world whose books row 0 is "of Town Portal" (`pSpell` 2).
fn with_books() -> World {
    let mut t = inv_tables();
    t.books = vec![InvBookRec {
        pspell: 2,
        scrollskill: SCROLL_SKILL,
        bookskill: BOOK_SKILL,
        spellicon: 0,
    }];
    World::with_tables(t)
}

/// `tsc` / `tbk` index 2 from `misc.txt` without a books row;
/// with one, from its `pSpell` and e = `BookSkill`; a key has none (index
/// 0, outside 1..31: no entry).
#[test]
fn use_index_from_the_books_row_then_the_items_record() {
    let mut w = World::new();
    let s = stored(&mut w, TSC, 0, 0);
    let k = stored(&mut w, KEY, 2, 0);
    assert_eq!(w.desk(|d| d.use_entry(s)), Some((2, -1)));
    assert_eq!(w.desk(|d| d.use_entry(k)), None);
    let mut w = with_books();
    let b = stored(&mut w, TBK, 0, 0);
    assert_eq!(w.desk(|d| d.use_entry(b)), Some((2, BOOK_SKILL)));
    // The tome's skill is `bookskill`, the scroll's
    // `scrollskill`.
    let s = stored(&mut w, TSC, 2, 0);
    assert_eq!(w.desk(|d| MovePending::item_skill(d, b)), BOOK_SKILL);
    assert_eq!(w.desk(|d| MovePending::item_skill(d, s)), SCROLL_SKILL);
}

/// 0x20 with a scroll, the cast made the pair: the cast was asked for the
/// player, its own 0x7C went out, then the targeting reset clears the
/// scroll's flag 0x4 (S→C 0x3F) and the scroll is consumed.
#[test]
fn a_made_cast_uses_the_scroll() {
    let mut w = World::new();
    w.hooks.cast = Some((1, true));
    let s = stored(&mut w, TSC, 0, 0);
    w.rest.sent.clear();
    assert_eq!(w.handle(&msg(0x20, &[s, 10, 10])), Ok(0));
    assert_eq!(w.hooks.casts, [w.player]);
    assert!(w.unit(s).is_none(), "the scroll is used up");
    assert_eq!(use_msgs(&mut w), [used(s), reset(s)]);
}

/// The town refusal (result 0, no 0x7C from the cast): the scroll stays,
/// its flag 0x4 is cleared with one S→C 0x3F, and the dispatcher's 0x7C
/// is the only one.
#[test]
fn a_refused_cast_keeps_the_scroll() {
    let mut w = World::new();
    w.hooks.cast = Some((0, false));
    let s = stored(&mut w, TSC, 0, 0);
    w.rest.sent.clear();
    assert_eq!(w.handle(&msg(0x20, &[s, 10, 10])), Ok(0));
    assert!(w.unit(s).is_some(), "the scroll stays");
    assert_eq!(w.data(s).flags & iflag::TARGETING, 0);
    assert_eq!(use_msgs(&mut w), [reset(s), used(s)]);
}

/// A cast that fails outside a town (result 0 after its own 0x7C): two
/// 0x7C. Without a cast host the desk records a request
/// (`tests/town_portal.rs`) and the use is a 1.
#[test]
fn a_failed_cast_sends_two_item_messages() {
    let mut w = World::new();
    w.hooks.cast = Some((0, true));
    let s = stored(&mut w, TSC, 0, 0);
    w.rest.sent.clear();
    assert_eq!(w.handle(&msg(0x20, &[s, 10, 10])), Ok(0));
    assert!(w.unit(s).is_some());
    assert_eq!(use_msgs(&mut w), [used(s), reset(s), used(s)]);
    w.hooks.cast = None;
    assert_eq!(w.handle(&msg(0x20, &[s, 10, 10])), Ok(0));
    let pu = w.player;
    assert_eq!(w.desk(|d| d.take_portal_requests()), [pu]);
}

/// The failure reset clears flag 0x4 on every flagged item of the
/// inventory, in list order, with `3F FF <GUID> FF FF` each.
#[test]
fn the_failure_reset_clears_every_flagged_item() {
    let mut w = World::new();
    w.hooks.cast = Some((0, false));
    let k = stored(&mut w, KEY, 4, 0);
    let s = stored(&mut w, TSC, 0, 0);
    w.desk(|d| {
        let f = d.item_flags(k);
        d.set_item_flags(k, f | iflag::TARGETING);
    });
    w.rest.sent.clear();
    let me = w.me();
    assert!(!w.desk(|d| d.dispatch_use(me, s, Owner::item(s), 0, 0)));
    let order: Vec<Guid> = w
        .inventory()
        .items()
        .iter()
        .map(|&u| w.units.get(u).unwrap().guid)
        .filter(|g| *g == k || *g == s)
        .collect();
    let mut want: Vec<Vec<u8>> = order.into_iter().map(reset).collect();
    want.push(used(s));
    assert_eq!(use_msgs(&mut w), want);
    assert_eq!(w.data(k).flags & iflag::TARGETING, 0);
}

/// A tome loses its charge (stat 70, S→C 0x3E) only when the cast made
/// the pair; a failed cast keeps the charge.
#[test]
fn a_tome_is_charged_only_on_a_made_cast() {
    let mut w = with_books();
    let t = stored(&mut w, TBK, 0, 0);
    let tu = w.unit(t).unwrap();
    w.set_stat(tu, QUANTITY, 2);
    let it = Owner::item(t);
    w.hooks.cast = Some((0, true));
    assert_eq!(w.handle(&msg(0x20, &[t, 10, 10])), Ok(0));
    assert_eq!(w.desk(|d| d.stat(it, QUANTITY)), 2, "a failed cast is free");
    w.hooks.cast = Some((1, true));
    w.rest.sent.clear();
    assert_eq!(w.handle(&msg(0x20, &[t, 10, 10])), Ok(0));
    assert_eq!(w.desk(|d| d.stat(it, QUANTITY)), 1);
    // The charge goes out as S→C 0x3E.
    let stat_msgs: Vec<_> = w.rest.sent.iter().filter(|(_, m)| m[0] == 0x3E).collect();
    assert_eq!(stat_msgs.len(), 1, "one item stat message for the charge");
    assert!(w.unit(t).is_some(), "a tome stays");
}

/// Items without an entry are not the dispatcher's: a key's use goes
/// to the rest, no cast is asked.
#[test]
fn other_entries_keep_the_rest_answer() {
    let mut w = World::new();
    w.hooks.cast = Some((1, true));
    let k = stored(&mut w, KEY, 0, 0);
    let me = w.me();
    assert_eq!(w.desk(|d| d.use_entry(k)), None);
    let _ = w.desk(|d| d.use_item_at(me, k, 0, 0));
    assert!(w.hooks.casts.is_empty());
}

/// 0x27 with a Town Portal scroll on an item: the use runs the cast;
/// made → the scroll is consumed, not made → result 1 and
/// the scroll stays.
#[test]
fn use_item_action_runs_the_cast() {
    let mut w = World::new();
    let target = stored(&mut w, CAP, 0, 0);
    let s = stored(&mut w, TSC, 4, 0);
    w.hooks.cast = Some((0, true));
    let _ = w.handle(&msg(0x27, &[target, s]));
    assert!(w.unit(s).is_some());
    w.hooks.cast = Some((1, true));
    assert_eq!(w.handle(&msg(0x27, &[target, s])), Ok(0));
    assert!(w.unit(s).is_none());
    assert_eq!(w.hooks.casts.len(), 2);
}

/// The sound call at every move site: the pickup sound is event 1, the
/// others their own event, each with the player itself the target, so
/// only its own client hears it.
#[test]
fn move_sounds_target_the_player_itself() {
    use crate::units::sound::SoundSlot;
    let mut w = World::new();
    let (me, p) = (w.me(), w.player);
    w.desk(|d| d.pickup_sound(me, 0));
    let slot = |w: &World| w.game.sounds.get(p);
    assert_eq!(
        slot(&w),
        Some(SoundSlot {
            event: 1,
            target: Some(p)
        })
    );
    w.desk(|d| d.sound(me, 0x13));
    assert_eq!(
        slot(&w),
        Some(SoundSlot {
            event: 0x13,
            target: Some(p)
        })
    );
}

/// `quest_item_used(player, chain)`: with the move effects on, the desk sends
/// S→C 0x5D `5D chain 02 00 0000` to the player's own client; without
/// them the rest keeps the call.
#[test]
fn quest_item_use_sends_the_chain_notice() {
    let mut w = World::new();
    let me = w.me();
    w.state.move_effects = true;
    for chain in [8u8, 18, 33] {
        w.desk(|d| MovePending::quest_item_used(d, me, chain));
    }
    assert_eq!(
        w.rest.sent,
        [8u8, 18, 33].map(|c| (me, vec![0x5D, c, 0x02, 0, 0, 0]))
    );
    let mut w = World::new();
    let me = w.me();
    w.desk(|d| MovePending::quest_item_used(d, me, 8));
    assert!(w.rest.sent.is_empty());
}
