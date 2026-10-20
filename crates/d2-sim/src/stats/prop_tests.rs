//! State-machine property test of the stat lists (`StatLists`) on the
//! synthetic itemstatcost of `stats::tests`, under random sequences of
//! base writes, attach / detach / free, equip and dynamic toggles, park /
//! unpark, expiry, death and overlay removal. After every step:
//!
//! - arrays: base sorted by key, unique, never 0; full sorted,
//!   unique, 0 only for `keepzero`, never an invalid stat;
//! - chains: every child of an active / parked chain names its
//!   parent, prev / next agree, the parked chain holds exactly the SET
//!   lists, a list is in at most one chain; a live list whose parent is
//!   freed is a parked child of a freed list (edge case 6);
//! - which lists live, predicted from the rules;
//! - totals: for every extended list E and every stat without ops (A51 =
//!   A52 = 0), the full value equals the full-value sum over the
//!   current chain: base(E) + Σ active children (full if extended, else
//!   base; a damage-related stat skips DYNAMIC children), plus the
//!   amounts the rules send to E without a child that counts: edge case
//!   3 (a plain DYNAMIC child's damage-related base change), minus a
//!   parked extended list's own base changes, and a
//!   dynamic toggle of a list that is not in E's active chain
//!   (parked, or left with its unit set by a refused attach), and
//!   the reverse for E holding the toggled list in its active chain when
//!   E is not the toggling unit's list (the flag flips, E's full stays);
//! - the mod array of player lists: exactly the keys of base
//!   writes (and unit sets) of saved stats outside 6, 8, 10, 13, 14.
//!
//! The wide variant adds the op stats, the server callback
//! and an act time; its totals check covers the stats without ops.

use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;

use super::lists::{flag, ListId, StatHost, StatListError, ValueCallback};
use super::tests::{data, N};
use super::{key, key_stat, StatLists};
use crate::units::{UnitId, UnitType};

fn config(default: u32) -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default);
    ProptestConfig {
        cases,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

/// Units with a list slot: a player, a monster, two items.
const UNITS: [(UnitId, UnitType, u32); 4] = [
    (UnitId(1), UnitType::Player, 0),
    (UnitId(2), UnitType::Monster, 1),
    (UnitId(10), UnitType::Item, 0),
    (UnitId(11), UnitType::Item, 0),
];

/// Stats without ops in the synthetic table (19, 21 damage-related; 8,
/// 10 keepzero; 400 invalid).
const SIMPLE: [u16; 9] = [0, 2, 6, 8, 10, 19, 21, 40, 50];
const INVALID: u16 = 400;
/// Op stats and op targets of the synthetic table.
const OPS: [u16; 16] = [
    1, 3, 7, 9, 11, 12, 31, 73, 75, 76, 77, 214, 216, 217, 269, 270,
];
const STATES: [u32; 4] = [0, 30, 31, 40];
const EXCLUDED: [u16; 5] = [6, 8, 10, 13, 14];

#[derive(Clone, Copy, Debug)]
enum Op {
    AllocPlain { owner: u32, state: u32, flags: u32 },
    AllocExt(usize),
    Set(usize, usize, i32, u16),
    Add(usize, usize, i32, u16),
    UnitSet(usize, usize, i32),
    RemoveAll(usize),
    Merge(usize, usize),
    Attach(usize, usize, bool),
    Detach(usize),
    Free(usize),
    Equip(usize, usize, bool, bool),
    Toggle(usize, usize, bool),
    Park(usize, usize, bool),
    SetExpire(usize, i32),
    Expire(usize, i32),
    Death(usize),
    RemoveOverlay(usize),
    Clamp(usize),
    ByTime(usize, usize),
}

fn value() -> impl Strategy<Value = i32> {
    prop_oneof![
        8 => -300i32..300,
        1 => Just(0),
        1 => prop_oneof![Just(i32::MAX), Just(i32::MIN), any::<i32>()],
    ]
}

fn op() -> impl Strategy<Value = Op> {
    let s = any::<usize>;
    prop_oneof![
        3 => (prop::sample::select(vec![0u32, 1, 4]), prop::sample::select(STATES.to_vec()),
              prop::sample::select(vec![0u32, flag::TEMPONLY, flag::OVERLAY, flag::BASIC, 0x100]))
            .prop_map(|(owner, state, flags)| Op::AllocPlain { owner, state, flags }),
        1 => s().prop_map(Op::AllocExt),
        6 => (s(), s(), value(), 0u16..2).prop_map(|(l, k, v, y)| Op::Set(l, k, v, y)),
        4 => (s(), s(), value(), 0u16..2).prop_map(|(l, k, v, y)| Op::Add(l, k, v, y)),
        1 => (s(), s(), value()).prop_map(|(u, k, v)| Op::UnitSet(u, k, v)),
        1 => s().prop_map(Op::RemoveAll),
        1 => (s(), s()).prop_map(|(a, b)| Op::Merge(a, b)),
        5 => (s(), s(), any::<bool>()).prop_map(|(u, l, r)| Op::Attach(u, l, r)),
        2 => s().prop_map(Op::Detach),
        2 => s().prop_map(Op::Free),
        2 => (s(), s(), prop::bool::weighted(0.2), any::<bool>())
            .prop_map(|(u, l, w, r)| Op::Equip(u, l, w, r)),
        2 => (s(), s(), any::<bool>()).prop_map(|(u, l, d)| Op::Toggle(u, l, d)),
        2 => (s(), s(), any::<bool>()).prop_map(|(u, st, p)| Op::Park(u, st, p)),
        1 => (s(), 1i32..30).prop_map(|(l, f)| Op::SetExpire(l, f)),
        1 => (s(), 0i32..40).prop_map(|(u, f)| Op::Expire(u, f)),
        1 => s().prop_map(Op::Death),
        1 => s().prop_map(Op::RemoveOverlay),
        1 => s().prop_map(Op::Clamp),
        1 => (s(), s()).prop_map(|(u, l)| Op::ByTime(u, l)),
    ]
}

#[derive(Default)]
struct Host {
    act_time: Option<i32>,
}

impl StatHost for Host {
    fn act_time(&self, _unit: UnitId) -> Option<i32> {
        self.act_time
    }
}

struct Machine {
    wide: bool,
    lists: StatLists,
    host: Host,
    /// Lists the model knows to be live, in allocation order.
    live: Vec<ListId>,
    /// Amounts sent to a list by no counted child (see module docs).
    drift: BTreeMap<(ListId, i32), i32>,
    /// Expected mod arrays of player lists.
    mods: BTreeMap<ListId, BTreeSet<i32>>,
    /// keepzero keys seen in a full array: they stay.
    kept: BTreeSet<(ListId, i32)>,
}

fn pick<T: Copy>(v: &[T], s: usize) -> Option<T> {
    (!v.is_empty()).then(|| v[s % v.len()])
}

impl Machine {
    fn new(wide: bool) -> Self {
        Self {
            wide,
            lists: StatLists::new(data()),
            host: Host {
                act_time: wide.then_some(45),
            },
            live: Vec::new(),
            drift: BTreeMap::new(),
            mods: BTreeMap::new(),
            kept: BTreeSet::new(),
        }
    }

    fn stats(&self) -> Vec<u16> {
        let mut v = SIMPLE.to_vec();
        v.push(INVALID);
        if self.wide {
            v.extend(OPS);
        }
        v
    }

    fn dyn_(&self, l: ListId) -> bool {
        self.lists.flags(l) & flag::DYNAMIC != 0
    }

    fn parked(&self, l: ListId) -> bool {
        self.lists.flags(l) & flag::SET != 0
    }

    fn damagerelated(&self, k: i32) -> bool {
        self.lists
            .data()
            .stats
            .get(key_stat(k))
            .is_some_and(|i| i.damagerelated)
    }

    /// The list values A.
    fn values(&self, l: ListId) -> Vec<(i32, i32)> {
        if self.lists.is_extended(l) {
            self.lists.full_entries(l)
        } else {
            self.lists.base_entries(l)
        }
    }

    /// Lists freed by `free(l)`: l, then the plain lists of its
    /// active chain.
    fn freed_by_free(&self, l: ListId) -> Vec<ListId> {
        let mut out = vec![l];
        if self.lists.is_extended(l) {
            out.extend(
                self.lists
                    .active_chain(l)
                    .into_iter()
                    .filter(|&c| !self.lists.is_extended(c)),
            );
        }
        out
    }

    /// toggle of `il` on `unit`: the values go to the unit's list
    /// r only, while the DYNAMIC flip changes how il counts in the list
    /// whose active chain holds it (module docs). Drift on r when il does
    /// not count there; drift on il's parent p ≠ r, which keeps its full
    /// value although il now counts the other way (sum).
    fn toggle_drift(&mut self, unit: UnitId, il: ListId, dynamic: bool) {
        let Some(r) = self.lists.unit_list(unit) else {
            return;
        };
        if self.dyn_(il) == dynamic {
            return;
        }
        let in_r = self.lists.active_chain(r).contains(&il);
        let p = self
            .lists
            .parent(il)
            .filter(|&p| p != r && self.lists.is_live(p))
            .filter(|&p| self.lists.active_chain(p).contains(&il));
        for (k, v) in self.values(il) {
            if self.damagerelated(k) {
                let d = if dynamic { v.wrapping_neg() } else { v };
                if !in_r {
                    let e = self.drift.entry((r, k)).or_insert(0);
                    *e = e.wrapping_add(d);
                }
                if let Some(p) = p {
                    let e = self.drift.entry((p, k)).or_insert(0);
                    *e = e.wrapping_sub(d);
                }
            }
        }
    }

    /// Model of equip for the drift: which branch runs.
    fn equip_drift(&mut self, unit: UnitId, il: ListId, swap: bool, reset: bool) {
        if swap || self.lists.attached_unit(il) != Some(unit) {
            return;
        }
        let d = self.dyn_(il);
        if reset && d {
            self.toggle_drift(unit, il, false);
        } else if !reset && !d {
            self.toggle_drift(unit, il, true);
        }
    }

    fn apply(&mut self, op: Op) {
        let stats = self.stats();
        let live = self.live.clone();
        let unit = |s: usize| UNITS[s % UNITS.len()];
        let mut freed: Vec<ListId> = Vec::new();
        let bases_before: BTreeMap<ListId, Vec<(i32, i32)>> = live
            .iter()
            .map(|&l| (l, self.lists.base_entries(l)))
            .collect();
        let mut unit_set_key = None;
        match op {
            Op::AllocPlain {
                owner,
                state,
                flags,
            } => {
                let l = self.lists.alloc(flags, 0, owner, 77);
                self.lists.set_state(l, state);
                self.live.push(l);
            }
            Op::AllocExt(s) => {
                let (u, ty, class) = unit(s);
                if let Some(r) = self.lists.unit_list(u) {
                    freed = self.freed_by_free(r);
                }
                let cb = (self.wide && ty != UnitType::Item).then_some(ValueCallback::Server);
                let l = self
                    .lists
                    .alloc_extended(&mut self.host, u, ty, 100 + u.0, class, 0, cb);
                self.live.push(l);
                if ty == UnitType::Player {
                    self.mods.insert(l, BTreeSet::new());
                }
            }
            Op::Set(l, k, v, y) => {
                let (Some(l), Some(s)) = (pick(&live, l), pick(&stats, k)) else {
                    return;
                };
                self.lists.set(&mut self.host, l, s, v, y, None);
            }
            Op::Add(l, k, d, y) => {
                let (Some(l), Some(s)) = (pick(&live, l), pick(&stats, k)) else {
                    return;
                };
                self.lists.add(&mut self.host, l, s, d, y);
            }
            Op::UnitSet(u, k, v) => {
                let Some(s) = pick(&stats, k) else { return };
                let u = unit(u).0;
                if let Some(r) = self.lists.unit_list(u) {
                    unit_set_key = Some((r, key(s, 0)));
                }
                self.lists.unit_set(&mut self.host, u, s, v, 0);
            }
            Op::RemoveAll(l) => {
                let Some(l) = pick(&live, l) else { return };
                self.lists.remove_all(&mut self.host, l);
            }
            Op::Merge(a, b) => {
                let (Some(a), Some(b)) = (pick(&live, a), pick(&live, b)) else {
                    return;
                };
                self.lists.merge(&mut self.host, a, b);
            }
            Op::Attach(u, l, reset) => {
                let Some(l) = pick(&live, l) else { return };
                self.lists.attach(&mut self.host, unit(u).0, l, reset);
            }
            Op::Detach(l) => {
                let Some(l) = pick(&live, l) else { return };
                self.lists.detach(&mut self.host, l);
            }
            Op::Free(l) => {
                let Some(l) = pick(&live, l) else { return };
                freed = self.freed_by_free(l);
                self.lists.free(&mut self.host, l);
            }
            Op::Equip(u, l, swap, reset) => {
                let Some(l) = pick(&live, l) else { return };
                let u = unit(u).0;
                self.equip_drift(u, l, swap, reset);
                self.lists.equip(&mut self.host, u, Some(l), swap, reset);
            }
            Op::Toggle(u, l, dynamic) => {
                let Some(l) = pick(&live, l) else { return };
                let u = unit(u).0;
                if self.lists.attached_unit(l) == Some(u) {
                    self.toggle_drift(u, l, dynamic);
                }
                if dynamic {
                    self.lists.make_dynamic(&mut self.host, u, l, false);
                } else {
                    self.lists.make_static(&mut self.host, u, l, false);
                }
            }
            Op::Park(u, st, park) => {
                // Real states only: parking by state 0 would park another
                // unit's own list (state 0), and a propagation that starts
                // at a parked list does nothing; the model does
                // not follow sums through a parked unit list.
                let state = STATES[1 + st % (STATES.len() - 1)];
                self.lists.park(&mut self.host, unit(u).0, state, park);
            }
            Op::SetExpire(l, f) => {
                let Some(l) = pick(&live, l) else { return };
                self.lists.set_expire(l, f);
            }
            Op::Expire(u, frame) => {
                let u = unit(u).0;
                let Some(r) = self.lists.unit_list(u) else {
                    assert_eq!(self.lists.expire_lists(&mut self.host, u, frame), Ok(()));
                    return;
                };
                // Every NEWLENGTH list of the active chain with
                // expire ≤ frame (after the client form's −1).
                let due: Vec<ListId> = self
                    .lists
                    .active_chain(r)
                    .into_iter()
                    .filter(|&c| self.lists.flags(c) & flag::NEWLENGTH != 0)
                    .filter(|&c| self.lists.expire(c) - i32::from(frame == 0) <= frame)
                    .collect();
                // An expired extended list: endless in the base game (edge case
                // 4); D2RUST stops at it with an error, after freeing the
                // plain due lists before it in the chain.
                let blocker = due.iter().copied().find(|&c| self.lists.is_extended(c));
                freed = due
                    .into_iter()
                    .take_while(|&c| !self.lists.is_extended(c))
                    .collect();
                let got = self.lists.expire_lists(&mut self.host, u, frame);
                assert_eq!(
                    got,
                    blocker.map_or(Ok(()), |c| Err(StatListError::EndlessExpiry(c)))
                );
            }
            Op::Death(u) => {
                let u = unit(u).0;
                if let Some(r) = self.lists.unit_list(u) {
                    freed = self
                        .lists
                        .active_chain(r)
                        .into_iter()
                        .filter(|&c| {
                            !self.lists.is_extended(c)
                                && self.lists.owner_type(c) != 4
                                && self.lists.flags(c) & 0x181 == 0
                        })
                        .collect();
                }
                self.lists.death(&mut self.host, u);
            }
            Op::RemoveOverlay(u) => self.lists.remove_overlay(&mut self.host, unit(u).0),
            Op::Clamp(u) => self.lists.clamp_to_max(&mut self.host, unit(u).0),
            Op::ByTime(u, l) => {
                let Some(l) = pick(&live, l) else { return };
                self.lists.by_time_refresh(&mut self.host, unit(u).0, l);
            }
        }
        // Liveness as predicted.
        for &l in &live {
            assert_eq!(
                self.lists.is_live(l),
                !freed.contains(&l),
                "{l:?} live after {op:?}"
            );
        }
        self.live.retain(|l| !freed.contains(l));
        self.drift.retain(|(l, _), _| !freed.contains(l));
        self.mods.retain(|l, _| !freed.contains(l));
        self.kept.retain(|(l, _)| !freed.contains(l));
        // Base changes: edge case 3 drift and the mod array.
        for (&l, before) in &bases_before {
            if freed.contains(&l) {
                continue;
            }
            let after: BTreeMap<i32, i32> = self.lists.base_entries(l).into_iter().collect();
            let before: BTreeMap<i32, i32> = before.iter().copied().collect();
            let keys: BTreeSet<i32> = before.keys().chain(after.keys()).copied().collect();
            for k in keys {
                let (b, a) = (
                    before.get(&k).copied().unwrap_or(0),
                    after.get(&k).copied().unwrap_or(0),
                );
                if a == b {
                    continue;
                }
                if let Some(m) = self.mods.get_mut(&l) {
                    m.insert(k);
                }
                // A parked list propagates nothing, not even
                // into its own full array.
                if self.lists.is_extended(l) && self.parked(l) {
                    let e = self.drift.entry((l, k)).or_insert(0);
                    *e = e.wrapping_sub(a.wrapping_sub(b));
                }
                let parent = self.lists.parent(l).filter(|&p| self.lists.is_live(p));
                if let Some(p) = parent {
                    if !self.lists.is_extended(l)
                        && self.dyn_(l)
                        && !self.parked(l)
                        && self.damagerelated(k)
                    {
                        let e = self.drift.entry((p, k)).or_insert(0);
                        *e = e.wrapping_add(a.wrapping_sub(b));
                    }
                }
            }
        }
        if let Some((r, k)) = unit_set_key {
            if let Some(m) = self.mods.get_mut(&r) {
                m.insert(k);
            }
        }
        for m in self.mods.values_mut() {
            m.retain(|&k| {
                let s = key_stat(k);
                s < 16 && !EXCLUDED.contains(&s)
            });
        }
    }

    fn check(&mut self) {
        for &l in &self.live {
            let full = self.lists.full_entries(l);
            for &(l2, k) in self.kept.iter().filter(|(x, _)| *x == l) {
                assert!(
                    full.iter().any(|e| e.0 == k),
                    "keepzero entry {k:#x} left {l2:?} (§6.3)"
                );
            }
            for (k, _) in full {
                if self
                    .lists
                    .data()
                    .stats
                    .get(key_stat(k))
                    .is_some_and(|i| i.keepzero)
                {
                    self.kept.insert((l, k));
                }
            }
        }
        let data = self.lists.data();
        let mut seen = BTreeSet::new();
        for &l in &self.live {
            let base = self.lists.base_entries(l);
            assert!(base.windows(2).all(|w| w[0].0 < w[1].0), "base sorted §3.1");
            assert!(base.iter().all(|e| e.1 != 0), "no 0 in a base array §3.4");
            let full = self.lists.full_entries(l);
            assert!(full.windows(2).all(|w| w[0].0 < w[1].0), "full sorted");
            for &(k, v) in &full {
                let info = data.stats.get(key_stat(k)).expect("valid stat in full");
                assert!(v != 0 || info.keepzero, "0 in full only for keepzero §3.4");
            }
            let Some(p) = self.lists.parent(l) else {
                continue;
            };
            if !self.lists.is_live(p) {
                assert!(self.parked(l), "only parked children outlive their parent");
                continue;
            }
            let chain = if self.parked(l) {
                self.lists.parked_chain(p)
            } else {
                self.lists.active_chain(p)
            };
            assert!(chain.contains(&l), "{l:?} in its parent's chain");
        }
        for &e in self.live.iter().filter(|&&l| self.lists.is_extended(l)) {
            for (chain, parked) in [
                (self.lists.active_chain(e), false),
                (self.lists.parked_chain(e), true),
            ] {
                let mut newer = None;
                for &c in &chain {
                    assert!(seen.insert(c), "{c:?} in two chains");
                    assert_eq!(self.lists.parent(c), Some(e));
                    assert_eq!(self.parked(c), parked);
                    assert_eq!(self.lists.next(c), newer);
                    newer = Some(c);
                }
            }
            // Totals.
            let full: BTreeMap<i32, i32> = self.lists.full_entries(e).into_iter().collect();
            let base: BTreeMap<i32, i32> = self.lists.base_entries(e).into_iter().collect();
            for s in SIMPLE {
                for y in 0..2 {
                    let k = key(s, y);
                    let mut want = base.get(&k).copied().unwrap_or(0);
                    for c in self.lists.active_chain(e) {
                        if self.damagerelated(k) && self.dyn_(c) {
                            continue;
                        }
                        let v = self
                            .values(c)
                            .into_iter()
                            .find(|x| x.0 == k)
                            .map_or(0, |x| x.1);
                        want = want.wrapping_add(v);
                    }
                    // eval (A52 = 0) is the sum itself.
                    assert_eq!(
                        self.lists.eval(&self.host, e, k),
                        want,
                        "eval of {s} on {e:?}"
                    );
                    want = want.wrapping_add(self.drift.get(&(e, k)).copied().unwrap_or(0));
                    assert_eq!(
                        full.get(&k).copied().unwrap_or(0),
                        want,
                        "full of stat {s} layer {y} on {e:?}"
                    );
                }
            }
            assert!(full.keys().all(|&k| key_stat(k) < N as u16));
        }
        for (&l, want) in &self.mods {
            let got: BTreeSet<i32> = self.lists.mods(l).into_iter().collect();
            assert_eq!(&got, want, "mod array of {l:?} §11.1");
        }
    }
}

fn run(wide: bool, ops: Vec<Op>) {
    let mut m = Machine::new(wide);
    for i in 0..UNITS.len() {
        m.apply(Op::AllocExt(i));
    }
    for op in ops {
        m.apply(op);
        m.check();
    }
}

proptest! {
    #![proptest_config(config(256))]

    #[test]
    fn stat_lists_match_the_model(ops in prop::collection::vec(op(), 0..120)) {
        run(false, ops);
    }

    #[test]
    fn stat_lists_with_ops_and_callbacks(ops in prop::collection::vec(op(), 0..120)) {
        run(true, ops);
    }
}

/// Regression (found by `stat_lists_match_the_model`): a parked child of
/// a freed list (edge case 6) could not be detached, re-attached or
/// freed: detach fixed the heads of the freed parent and panicked.
#[test]
fn regress_parked_child_outlives_its_parent() {
    let mut lists = StatLists::new(data());
    let mut host = Host::default();
    let p = UnitId(1);
    let r = lists.alloc_extended(&mut host, p, UnitType::Player, 1, 0, 0, None);
    let a = lists.alloc(0, 0, 0, 5);
    let b = lists.alloc(0, 0, 0, 6);
    lists.set_state(a, 30);
    lists.set_state(b, 31);
    lists.set(&mut host, a, 0, 5, 0, None);
    lists.attach(&mut host, p, a, true);
    lists.attach(&mut host, p, b, true);
    assert!(lists.park(&mut host, p, 30, true));
    assert!(lists.park(&mut host, p, 31, true));
    lists.free(&mut host, r);
    assert!(!lists.is_live(r));
    assert_eq!(lists.parent(a), Some(r));
    // A new player list; the orphan joins it (parked), then is unparked.
    let r2 = lists.alloc_extended(&mut host, p, UnitType::Player, 1, 0, 0, None);
    lists.attach(&mut host, p, a, true);
    assert_eq!(lists.parked_chain(r2), [a]);
    assert!(lists.park(&mut host, p, 30, false));
    assert_eq!(lists.active_chain(r2), [a]);
    assert_eq!(lists.full_entries(r2), [(key(0, 0), 5)]);
    assert_eq!(lists.prev(b), None);
    assert_eq!(lists.next(b), None);
    lists.free(&mut host, b);
    assert!(!lists.is_live(b));
}

/// Regression (CI, nextest random seed): proptest's minimal input for
/// `stat_lists_match_the_model`.
#[test]
fn regress_full_after_free_and_toggle() {
    run(
        false,
        vec![
            Op::Free(7306593420852098055),
            Op::Set(9636650453837638358, 2407644361622197065, -1, 0),
            Op::Attach(17415746941517033564, 7661639184940325630, true),
            Op::AllocPlain {
                owner: 0,
                state: 0,
                flags: 0,
            },
            Op::AllocPlain {
                owner: 0,
                state: 0,
                flags: 0,
            },
            Op::Free(7669981465254258163),
            Op::Toggle(18125049263891989335, 6524073951232642306, false),
            Op::AllocExt(15362796750465878983),
            Op::AllocPlain {
                owner: 0,
                state: 0,
                flags: 0,
            },
            Op::AllocPlain {
                owner: 0,
                state: 0,
                flags: 0,
            },
            Op::Equip(7168547649646239135, 8265621924537273655, false, false),
        ],
    );
}

// ---- helpers; regeneration ----

fn edge() -> impl Strategy<Value = i32> {
    prop_oneof![
        4 => any::<i32>(),
        1 => prop::sample::select(vec![i32::MIN, i32::MIN + 1, -1, 0, 1, 255, 256, i32::MAX]),
    ]
}

proptest! {
    #![proptest_config(config(2048))]

    // No input panics a helper (32-bit arithmetic of the base game).
    #[test]
    fn helpers_take_any_input(a in edge(), b in edge(), c in edge()) {
        let _ = super::muldiv(a, b, c);
        let _ = super::by_time(a, b);
        let _ = super::act_base_time(a, b);
        let _ = super::life_fraction(a, b);
        let _ = super::fraction_changed(a, b);
        for p in [24, 53, 64] {
            let _ = super::x87_rescale(a, b, c, p);
        }
    }

    // Inside the 32-bit product rule without wrap, MulDiv is the
    // exact truncated quotient; divide first.
    #[test]
    fn muldiv_rules(a in any::<i32>(), b in any::<i32>(), c in any::<i32>()) {
        let got = super::muldiv(a, b, c);
        let exact = |a: i32, b: i32| {
            (i64::from(a) * i64::from(b) / i64::from(c)) as i32
        };
        if c == 0 {
            prop_assert_eq!(got, 0);
        } else if a > 0x10_0000 {
            if c <= a >> 4 {
                prop_assert_eq!(got, (a / c).wrapping_mul(b));
            } else {
                prop_assert_eq!(got, exact(a, b));
            }
        } else if b > 0x1_0000 {
            if c <= b >> 4 {
                prop_assert_eq!(got, (b / c).wrapping_mul(a));
            } else {
                prop_assert_eq!(got, exact(a, b));
            }
        } else if let Some(p) = a.checked_mul(b) {
            prop_assert_eq!(got, p.wrapping_div(c));
        }
    }

    // The result lies between lo and hi; t = 90·p gives hi.
    #[test]
    fn by_time_between_lo_and_hi(v in any::<i32>(), t in -(1i32 << 29)..(1 << 29)) {
        let lo = ((v >> 2) & 0x3FF) - 256;
        let hi = ((v >> 12) & 0x3FF) - 256;
        let got = super::by_time(v, t);
        prop_assert!(got >= lo.min(hi) && got <= lo.max(hi), "{} not in [{}, {}]", got, lo, hi);
        prop_assert_eq!(super::by_time(v, 90 * (v & 3)), hi);
    }
}

mod regen {
    use super::*;
    use crate::game::Game;
    use crate::stats::stat;
    use crate::units::dispatch::player_regen;
    use crate::units::hooks::{Sim, UnitData, UnitHooks};
    use crate::units::record::{UnitRecord, Units};

    struct NoHooks;
    impl StatHost for NoHooks {}
    impl UnitHooks for NoHooks {}

    #[derive(Clone, Debug)]
    struct Input {
        mode: u32,
        hp: i32,
        maxhp: i32,
        mana: i32,
        maxmana: i32,
        stamina: i32,
        maxstamina: i32,
        regen: i32,
        manarecovery: i32,
        manabonus: i32,
        staminabonus: i32,
        nomanaregen: bool,
    }

    fn input() -> impl Strategy<Value = Input> {
        let v = || prop_oneof![-2000i32..2000, 0i32..200_000];
        (
            0u32..20,
            (v(), v(), v(), v(), v(), v()),
            prop_oneof![Just(0), -3000i32..3000],
            -100i32..100,
            -200i32..300,
            prop_oneof![-100i32..300, 900i32..1200],
            any::<bool>(),
        )
            .prop_map(
                |(
                    mode,
                    (hp, maxhp, mana, maxmana, stamina, maxstamina),
                    regen,
                    manarecovery,
                    manabonus,
                    staminabonus,
                    nomanaregen,
                )| Input {
                    mode,
                    hp,
                    maxhp,
                    mana,
                    maxmana,
                    stamina,
                    maxstamina,
                    regen,
                    manarecovery,
                    manabonus,
                    staminabonus,
                    nomanaregen,
                },
            )
    }

    /// The minimum rule of a present raw value.
    /// Raw 0 is absent and reads 0 without the rule.
    fn min_rule(raw: i32, min: i32, shift: u32) -> i32 {
        if raw != 0 && raw < min {
            min << shift
        } else {
            raw
        }
    }

    proptest! {
        #![proptest_config(config(1024))]

        // Player: reschedule at f + 1; nothing more
        // for a dead player; else life, stamina, mana by the rules.
        #[test]
        fn player_regeneration(i in input()) {
            let mut game = Game::new();
            game.frame = 77;
            let unit = game.spawn_unit(UnitType::Player, None, true).unwrap();
            let mut units = Units::new();
            let mut rec = UnitRecord::new(UnitType::Player, 0, 1);
            rec.mode = i.mode;
            units.insert(unit, rec);
            let mut stats = StatLists::new(data());
            let mut h = NoHooks;
            stats.alloc_extended(&mut h, unit, UnitType::Player, 1, 0, 0, None);
            for (s, v) in [
                (stat::HITPOINTS, i.hp), (stat::MAXHP, i.maxhp), (stat::MANA, i.mana),
                (stat::MAXMANA, i.maxmana), (stat::STAMINA, i.stamina),
                (stat::MAXSTAMINA, i.maxstamina), (stat::HPREGEN, i.regen),
                (stat::MANARECOVERY, i.manarecovery), (stat::MANARECOVERYBONUS, i.manabonus),
                (stat::STAMINARECOVERYBONUS, i.staminabonus),
            ] {
                stats.unit_set(&mut h, unit, s, v, 0);
            }
            if i.nomanaregen {
                stats.toggle_state(unit, super::super::states::state::NOMANAREGEN, true);
            }
            let data = UnitData::default();
            let mut sim = Sim { game: &mut game, units: &mut units, stats: &mut stats, data: &data };
            player_regen(&mut sim, &mut h, unit, 3, 4).unwrap();
            let t = game.timers.unit_timers(unit);
            prop_assert_eq!(t.len(), 1);
            prop_assert_eq!(game.timers.event(t[0]), Some((3, 3, 4)));
            prop_assert_eq!(game.timers.expire(t[0]), Some(78));
            // Raw reads: 6, 8, 10 have no fMin.
            let raw = |s| stats.base_entries(stats.unit_list(unit).unwrap())
                .into_iter().find(|e| e.0 == key(s, 0)).map_or(0, |e| e.1);
            if matches!(i.mode, 0 | 17) {
                prop_assert_eq!((raw(6), raw(8), raw(10)), (i.hp, i.mana, i.stamina));
                return Ok(());
            }
            // Life.
            let max_life = min_rule(i.maxhp, 1, 8);
            let mut hp = i.hp;
            if i.regen != 0 {
                hp = i.hp.wrapping_add(i.regen);
                if hp > max_life {
                    hp = max_life;
                }
                if hp < 256 {
                    hp = 256;
                }
            }
            prop_assert_eq!(raw(6), hp, "life");
            // Stamina.
            let (s, b) = (i.stamina, i.staminabonus);
            let shift = match i.mode {
                1 | 5 => Some(8),
                2 if s as u32 & 0xFFFF_FF00 != 0 => Some(9),
                2 => None,
                6 => Some(9),
                _ if b >= 1000 => Some(8),
                _ => None,
            };
            let m = min_rule(i.maxstamina, 0, 8);
            let mut want_s = s;
            if let Some(shift) = shift {
                if s < m {
                    let mut inc = m >> shift;
                    if b != 0 {
                        inc += inc * b / 100;
                    }
                    want_s = (s + inc).min(m);
                }
            }
            prop_assert_eq!(raw(10), want_s, "stamina");
            // Mana, class 0: ManaRegen 0 → q = 7500.
            let (v, m) = (i.mana, min_rule(i.maxmana, 0, 8));
            let mut inc = 0;
            if !i.nomanaregen {
                inc = super::super::muldiv((m / 7500).max(1), i.manabonus + 100, 100);
            }
            inc += i.manarecovery;
            inc = inc.min(m - v).max(-v);
            prop_assert_eq!(raw(8), v + inc, "mana");
        }
    }
}
