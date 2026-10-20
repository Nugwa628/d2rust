//! Unit tests of the pet lists against an in-memory [`PetWorld`].

use std::collections::BTreeMap;

use super::*;

/// Synthetic `pettype` rows (only the numbers 1 and 7 are fixed by the set-max rules).
const COUNT: i32 = 10;
const SKELETON: i32 = 2;
const WOLF: i32 = 3;
const FENRIS: i32 = 4;
const GRIZZLY: i32 = 5;
const GOLEM: i32 = 6;

const P1: u32 = 1;
const P2: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Ev {
    Flags(u32, u32),
    Kill(u32),
    Mode(u32, Option<u32>),
    Resync(u32),
    Send(u32, PetMsg),
}

#[derive(Debug, Clone, Default)]
struct FakeUnit {
    guid: i32,
    player: bool,
    class: u16,
    flags: u32,
    c8_bit8: bool,
    killable: bool,
    owner: Option<u32>,
    gone: bool,
    // Players only.
    data: bool,
    lists: Option<PetLists>,
    state7: bool,
}

struct Fx {
    units: BTreeMap<u32, FakeUnit>,
    groups: Vec<i16>,
    /// What the resync writes: (player, t, new max).
    resync_sets: Option<(u32, i32, i32)>,
    log: Vec<Ev>,
    skills: Option<Vec<ResyncSkill>>,
    freed_hirelings: Vec<u32>,
}

impl Fx {
    fn new() -> Self {
        let mut groups = vec![0i16; COUNT as usize];
        // The base game: spiritwolf, fenris and grizzly share group 1.
        groups[WOLF as usize] = 1;
        groups[FENRIS as usize] = 1;
        groups[GRIZZLY as usize] = 1;
        let mut units = BTreeMap::new();
        for (id, guid) in [(P1, 1), (P2, 2)] {
            units.insert(
                id,
                FakeUnit {
                    guid,
                    player: true,
                    data: true,
                    lists: Some(PetLists::new(COUNT as usize)),
                    ..FakeUnit::default()
                },
            );
        }
        Self {
            units,
            groups,
            resync_sets: None,
            log: Vec::new(),
            skills: Some(Vec::new()),
            freed_hirelings: Vec::new(),
        }
    }

    /// A killable monster owned by P1; unit id = 100 + GUID.
    fn spawn(&mut self, guid: i32, class: u16) -> u32 {
        let id = 100 + guid as u32;
        self.units.insert(
            id,
            FakeUnit {
                guid,
                class,
                flags: FLAG_PET,
                killable: true,
                owner: Some(P1),
                ..FakeUnit::default()
            },
        );
        id
    }

    /// Links `guid` at the tail of P1's list `t` without any effect.
    fn link(&mut self, t: i32, guid: i32) {
        let e = &mut self.lists(P1).entries[t as usize];
        e.nodes.push(PetNode {
            flags: 0,
            guid,
            extra: [0; 3],
        });
        e.count += 1;
    }

    fn lists(&mut self, p: u32) -> &mut PetLists {
        self.units.get_mut(&p).unwrap().lists.as_mut().unwrap()
    }

    fn entry(&mut self, t: i32) -> PetEntry {
        self.lists(P1).entries[t as usize].clone()
    }

    fn guids(&mut self, t: i32) -> Vec<i32> {
        self.entry(t).nodes.iter().map(|n| n.guid).collect()
    }

    fn sends(&self) -> Vec<(u32, PetMsg)> {
        self.log
            .iter()
            .filter_map(|e| match e {
                Ev::Send(p, m) => Some((*p, *m)),
                _ => None,
            })
            .collect()
    }

    fn effects(&self) -> Vec<Ev> {
        self.log
            .iter()
            .filter(|e| !matches!(e, Ev::Send(..)))
            .cloned()
            .collect()
    }
}

impl PetWorld for Fx {
    type Unit = u32;

    fn is_player(&self, u: u32) -> bool {
        self.units[&u].player
    }
    fn has_player_data(&self, p: u32) -> bool {
        self.units[&p].data
    }
    fn pet_lists(&mut self, p: u32) -> Option<&mut PetLists> {
        let u = self.units.get_mut(&p).unwrap();
        assert!(u.data, "pet_lists called without player data");
        u.lists.as_mut()
    }
    fn pettype_count(&self) -> i32 {
        COUNT
    }
    fn pettype_group(&self, t: i32) -> i16 {
        self.groups[t as usize]
    }
    fn monster_by_guid(&self, guid: i32) -> Option<u32> {
        self.units
            .iter()
            .find(|(_, u)| !u.player && !u.gone && u.guid == guid)
            .map(|(id, _)| *id)
    }
    fn unit_guid(&self, u: u32) -> i32 {
        self.units[&u].guid
    }
    fn unit_class(&self, u: u32) -> u16 {
        self.units[&u].class
    }
    fn flag_c8_bit8(&self, u: u32) -> bool {
        self.units[&u].c8_bit8
    }
    fn unit_flags(&self, u: u32) -> u32 {
        self.units[&u].flags
    }
    fn set_unit_flags(&mut self, u: u32, flags: u32) {
        self.units.get_mut(&u).unwrap().flags = flags;
        self.log.push(Ev::Flags(u, flags));
    }
    fn killable(&self, u: u32) -> bool {
        self.units[&u].killable
    }
    fn kill(&mut self, u: u32) {
        self.log.push(Ev::Kill(u));
    }
    fn owner(&self, u: u32) -> Option<u32> {
        self.units[&u].owner
    }
    fn request_death_mode(&mut self, u: u32, target: Option<u32>) {
        self.log.push(Ev::Mode(u, target));
    }
    fn resync(&mut self, p: u32) {
        self.log.push(Ev::Resync(p));
        if let Some((rp, t, max)) = self.resync_sets {
            self.lists(rp).entries[t as usize].max = max;
        }
    }
    fn players(&self) -> Vec<u32> {
        self.units
            .iter()
            .filter(|(_, u)| u.player)
            .map(|(id, _)| *id)
            .collect()
    }
    fn has_state(&self, u: u32, state: u16) -> bool {
        state == STATE_NO_BROADCAST && self.units[&u].state7
    }
    fn send(&mut self, p: u32, msg: PetMsg) {
        self.log.push(Ev::Send(p, msg));
    }
}

impl PetLifecycleWorld for Fx {
    fn set_pet_lists(&mut self, p: u32, lists: Option<PetLists>) {
        self.units.get_mut(&p).unwrap().lists = lists;
    }
    fn pettype_basemax(&self, t: i32) -> i32 {
        i32::from(t == PETTYPE_SINGLE || t == PETTYPE_HIREABLE)
    }
    fn skills(&self, _p: u32) -> Option<Vec<ResyncSkill>> {
        self.skills.clone()
    }
    fn free_hireling_unit(&mut self, unit: u32) {
        self.freed_hirelings.push(unit);
        self.units.get_mut(&unit).unwrap().gone = true;
    }
}

fn rm(guid: i32) -> PetMsg {
    PetMsg::PetAction {
        action: 0,
        pet_type: 0,
        class: 0,
        owner: 0,
        pet: guid as u32,
    }
}

fn added(t: i32, class: u16, owner: u32, guid: i32) -> PetMsg {
    PetMsg::PetAction {
        action: 1,
        pet_type: t as u8,
        class,
        owner,
        pet: guid as u32,
    }
}

/// `msg` to both players, `n` times in a row.
fn to_all(msg: PetMsg, n: usize) -> Vec<(u32, PetMsg)> {
    (0..n).flat_map(|_| [(P1, msg), (P2, msg)]).collect()
}

/// The effects of dismissing a killable monster `id` with flags `FLAG_PET`.
fn dismissed(id: u32) -> Vec<Ev> {
    vec![Ev::Flags(id, FLAG_PET | FLAG_NO_EXPERIENCE), Ev::Kill(id)]
}

// ---- lists ----------------------------------------------------

#[test]
fn lists_new_and_with_max() {
    let l = PetLists::new(3);
    assert_eq!(l.entries.len(), 3);
    assert!(l
        .entries
        .iter()
        .all(|e| e.count == 0 && e.max == 0 && e.nodes.is_empty()));
    let l = PetLists::with_max(&[0, 1, 5]);
    assert_eq!(
        l.entries.iter().map(|e| e.max).collect::<Vec<_>>(),
        [0, 1, 5]
    );
    assert_eq!(l.entries[2].head_guid(), -1);
}

#[test]
fn first_pet_any_flag_and_head_guid() {
    let mut fx = Fx::new();
    let a = fx.spawn(10, 1);
    let b = fx.spawn(11, 1);
    fx.link(SKELETON, 10);
    fx.link(SKELETON, 11);
    fx.lists(P1).entries[SKELETON as usize].nodes[0].flags = NODE_SKIP;
    assert_eq!(head_guid(&fx.entry(SKELETON)), 10);
    assert_eq!(first_pet(&mut fx, P1, SKELETON, true), Some(a));
    assert_eq!(first_pet(&mut fx, P1, SKELETON, false), Some(b));
    // Empty list, out of range, P null, no player data: none.
    assert_eq!(first_pet(&mut fx, P1, GOLEM, true), None);
    assert_eq!(first_pet(&mut fx, P1, COUNT, true), None);
    assert_eq!(first_pet(&mut fx, P1, -1, true), None);
    fx.units.get_mut(&P1).unwrap().data = false;
    assert_eq!(first_pet(&mut fx, P1, SKELETON, true), None);
}

// ---- add ------------------------------------------------------

#[test]
fn add_links_at_tail_and_broadcasts_add() {
    let mut fx = Fx::new();
    let s1 = fx.spawn(10, 363);
    let s2 = fx.spawn(11, 363);
    add(&mut fx, Some(P1), Some(s1), SKELETON, 3).unwrap();
    add(&mut fx, Some(P1), Some(s2), SKELETON, 3).unwrap();
    let e = fx.entry(SKELETON);
    assert_eq!((e.count, e.max), (2, 3));
    assert_eq!(fx.guids(SKELETON), [10, 11]);
    assert_eq!(e.nodes[1].extra, [0; 3]);
    assert_eq!(e.nodes[1].flags, 0);
    let mut want = to_all(added(SKELETON, 363, 1, 10), 1);
    want.extend(to_all(added(SKELETON, 363, 1, 11), 1));
    assert_eq!(fx.sends(), want);
    assert!(fx.effects().is_empty());
}

#[test]
fn add_without_player_or_type_does_nothing() {
    let mut fx = Fx::new();
    let m = fx.spawn(10, 1);
    add(&mut fx, None, Some(m), SKELETON, 3).unwrap();
    // A monster as the "player".
    add(&mut fx, Some(m), Some(m), SKELETON, 3).unwrap();
    // t outside 1…count − 1.
    add(&mut fx, Some(P1), Some(m), 0, 3).unwrap();
    add(&mut fx, Some(P1), Some(m), COUNT, 3).unwrap();
    add(&mut fx, Some(P1), Some(m), -2, 3).unwrap();
    assert!(fx.log.is_empty());
    assert!(fx.lists(P1).entries.iter().all(|e| e.max == 0));
}

#[test]
fn add_with_lists_null_does_nothing_and_without_data_is_fatal() {
    let mut fx = Fx::new();
    let m = fx.spawn(10, 1);
    fx.units.get_mut(&P1).unwrap().lists = None;
    add(&mut fx, Some(P1), Some(m), GRIZZLY, 1).unwrap();
    assert!(fx.log.is_empty());
    // Player data none: the set-max asserts.
    fx.units.get_mut(&P1).unwrap().data = false;
    assert_eq!(
        add(&mut fx, Some(P1), Some(m), SKELETON, 3),
        Err(PetError::NoPlayerData)
    );
}

#[test]
fn add_without_pet_uses_none_values() {
    let mut fx = Fx::new();
    add(&mut fx, Some(P1), None, GOLEM, 1).unwrap();
    assert_eq!(fx.guids(GOLEM), [-1]);
    assert_eq!(fx.sends(), to_all(added(GOLEM, 0xFFFF, 1, -1), 1));
}

// ---- group eviction -------------------------------------------

#[test]
fn grizzly_evicts_three_spirit_wolves() {
    // Test vector 1: Summon Grizzly with 3 spirit wolves alive.
    let mut fx = Fx::new();
    let w: Vec<u32> = (10..13).map(|g| fx.spawn(g, 227)).collect();
    for g in 10..13 {
        fx.link(WOLF, g);
    }
    fx.lists(P1).entries[WOLF as usize].max = 3;
    let griz = fx.spawn(20, 228);
    add(&mut fx, Some(P1), Some(griz), GRIZZLY, 1).unwrap();
    assert_eq!(fx.entry(WOLF).count, 0);
    assert_eq!(fx.guids(GRIZZLY), [20]);
    // Oldest wolf first, each dismissed (kill 1).
    let want: Vec<Ev> = w.iter().flat_map(|&id| dismissed(id)).collect();
    assert_eq!(fx.effects(), want);
    // Per wolf: unlink, dismiss, Remove; then the grizzly's add.
    let mut sends = Vec::new();
    for g in 10..13 {
        sends.extend(to_all(rm(g), 3));
    }
    sends.extend(to_all(added(GRIZZLY, 228, 1, 20), 1));
    assert_eq!(fx.sends(), sends);
}

#[test]
fn group_eviction_perturbation_other_group_or_none() {
    // Perturbation of test vector 1: the same summon with the wolves in
    // another group, or the grizzly in group 0, keeps the wolves.
    for (wolf_group, griz_group) in [(2, 1), (1, 0), (1, -1)] {
        let mut fx = Fx::new();
        fx.groups[WOLF as usize] = wolf_group;
        fx.groups[GRIZZLY as usize] = griz_group;
        fx.spawn(10, 227);
        fx.link(WOLF, 10);
        fx.lists(P1).entries[WOLF as usize].max = 1;
        let griz = fx.spawn(20, 228);
        add(&mut fx, Some(P1), Some(griz), GRIZZLY, 1).unwrap();
        assert_eq!(fx.guids(WOLF), [10]);
        assert!(fx.effects().is_empty());
    }
}

#[test]
fn group_eviction_visits_types_in_row_order() {
    let mut fx = Fx::new();
    let f = fx.spawn(30, 1);
    let w = fx.spawn(10, 2);
    fx.link(FENRIS, 30);
    fx.link(WOLF, 10);
    group_evict(&mut fx, P1, GRIZZLY).unwrap();
    // Row 3 (wolf) before row 4 (fenris), regardless of link order.
    let mut want = dismissed(w);
    want.extend(dismissed(f));
    assert_eq!(fx.effects(), want);
    // The type itself is never evicted.
    fx.log.clear();
    fx.spawn(20, 3);
    fx.link(GRIZZLY, 20);
    group_evict(&mut fx, P1, GRIZZLY).unwrap();
    assert!(fx.log.is_empty());
    assert_eq!(fx.guids(GRIZZLY), [20]);
}

#[test]
fn group_eviction_stops_at_missing_head_unit() {
    // Edge case 3: the head's monster is gone; later wolves stay.
    let mut fx = Fx::new();
    let gone = fx.spawn(10, 1);
    fx.units.get_mut(&gone).unwrap().gone = true;
    fx.spawn(11, 1);
    fx.link(WOLF, 10);
    fx.link(WOLF, 11);
    group_evict(&mut fx, P1, GRIZZLY).unwrap();
    assert_eq!(fx.guids(WOLF), [10, 11]);
    assert!(fx.log.is_empty());
    // Same when the second wolf's unit is gone: the first is removed, then
    // the walk stops at the new head.
    let mut fx = Fx::new();
    let a = fx.spawn(10, 1);
    let b = fx.spawn(11, 1);
    fx.units.get_mut(&b).unwrap().gone = true;
    fx.spawn(12, 1);
    for g in 10..13 {
        fx.link(WOLF, g);
    }
    group_evict(&mut fx, P1, GRIZZLY).unwrap();
    assert_eq!(fx.guids(WOLF), [11, 12]);
    assert_eq!(fx.effects(), dismissed(a));
}

// ---- set max --------------------------------------------------

#[test]
fn fourth_skeleton_with_max_three_removes_oldest() {
    // Test vector 2: raise a 4th skeleton with max 3.
    let mut fx = Fx::new();
    let ids: Vec<u32> = (10..14).map(|g| fx.spawn(g, 363)).collect();
    for &id in &ids {
        add(&mut fx, Some(P1), Some(id), SKELETON, 3).unwrap();
    }
    let e = fx.entry(SKELETON);
    assert_eq!((e.count, e.max), (3, 3));
    assert_eq!(fx.guids(SKELETON), [11, 12, 13]);
    // The append unlinks the head with kill 1: the oldest is dismissed.
    assert_eq!(fx.effects(), dismissed(ids[0]));
    // Unlink and dismiss each broadcast; no Remove wrapper here.
    let tail: Vec<_> = fx.sends().split_off(6);
    let mut want = to_all(rm(10), 2);
    want.extend(to_all(added(SKELETON, 363, 1, 13), 1));
    assert_eq!(tail, want);
}

#[test]
fn set_max_trims_from_head_with_two_or_three_broadcasts() {
    // Edge case 4: lowering the max removes the oldest one by one. With
    // the monsters gone each remove broadcasts twice (unlink + Remove).
    let mut fx = Fx::new();
    for g in 10..14 {
        let id = fx.spawn(g, 1);
        fx.units.get_mut(&id).unwrap().gone = true;
        fx.link(SKELETON, g);
    }
    set_max(&mut fx, P1, SKELETON, 1).unwrap();
    let e = fx.entry(SKELETON);
    assert_eq!((e.count, e.max), (1, 1));
    assert_eq!(fx.guids(SKELETON), [13]);
    let mut want = Vec::new();
    for g in 10..13 {
        want.extend(to_all(rm(g), 2));
    }
    assert_eq!(fx.sends(), want);
    assert!(fx.effects().is_empty());
    // With the monster alive, kill 1 also dismisses: a third broadcast.
    let mut fx = Fx::new();
    let a = fx.spawn(10, 1);
    fx.spawn(11, 1);
    fx.link(SKELETON, 10);
    fx.link(SKELETON, 11);
    set_max(&mut fx, P1, SKELETON, 1).unwrap();
    assert_eq!(fx.sends(), to_all(rm(10), 3));
    assert_eq!(fx.effects(), dismissed(a));
}

#[test]
fn set_max_special_types_and_guards() {
    let mut fx = Fx::new();
    // `single` accepts only 1.
    set_max(&mut fx, P1, PETTYPE_SINGLE, 2).unwrap();
    assert_eq!(fx.entry(PETTYPE_SINGLE).max, 0);
    set_max(&mut fx, P1, PETTYPE_SINGLE, 1).unwrap();
    assert_eq!(fx.entry(PETTYPE_SINGLE).max, 1);
    // Hirelings are never trimmed.
    fx.spawn(50, 338);
    fx.spawn(51, 338);
    fx.link(PETTYPE_HIREABLE, 50);
    fx.link(PETTYPE_HIREABLE, 51);
    set_max(&mut fx, P1, PETTYPE_HIREABLE, 0).unwrap();
    let e = fx.entry(PETTYPE_HIREABLE);
    assert_eq!((e.count, e.max), (2, 0));
    // Row 0 is in range for the set max.
    set_max(&mut fx, P1, 0, 4).unwrap();
    assert_eq!(fx.entry(0).max, 4);
    // Out of range: nothing.
    set_max(&mut fx, P1, COUNT, 4).unwrap();
    set_max(&mut fx, P1, -1, 4).unwrap();
    assert!(fx.log.is_empty());
    // P null: nothing; no player data: fatal.
    fx.units.get_mut(&P2).unwrap().lists = None;
    set_max(&mut fx, P2, SKELETON, 4).unwrap();
    fx.units.get_mut(&P2).unwrap().data = false;
    assert_eq!(
        set_max(&mut fx, P2, SKELETON, 4),
        Err(PetError::NoPlayerData)
    );
}

// ---- append ---------------------------------------------------

#[test]
fn append_max_zero_resyncs_then_dismisses() {
    let mut fx = Fx::new();
    let m = fx.spawn(10, 1);
    assert_eq!(append(&mut fx, P1, GOLEM, Some(m), [0; 3]), Ok(false));
    let mut want = vec![Ev::Resync(P1)];
    want.extend(dismissed(m));
    assert_eq!(fx.effects(), want);
    assert_eq!(fx.entry(GOLEM).count, 0);
    // Through add (append failure): no add broadcast, only the
    // dismiss's remove.
    let mut fx = Fx::new();
    let m = fx.spawn(10, 1);
    add(&mut fx, Some(P1), Some(m), GOLEM, 0).unwrap();
    assert_eq!(fx.sends(), to_all(rm(10), 1));
}

#[test]
fn append_after_resync_raises_max() {
    let mut fx = Fx::new();
    fx.resync_sets = Some((P1, GOLEM, 2));
    let m = fx.spawn(10, 1);
    assert_eq!(append(&mut fx, P1, GOLEM, Some(m), [4, 5, 6]), Ok(true));
    let e = fx.entry(GOLEM);
    assert_eq!((e.count, e.max), (1, 2));
    assert_eq!(e.nodes[0].extra, [4, 5, 6]);
    assert_eq!(fx.effects(), [Ev::Resync(P1)]);
}

#[test]
fn append_fatal_assertions() {
    // Full without a head.
    let mut fx = Fx::new();
    {
        let e = &mut fx.lists(P1).entries[GOLEM as usize];
        e.count = 1;
        e.max = 1;
    }
    assert_eq!(
        append(&mut fx, P1, GOLEM, None, [0; 3]),
        Err(PetError::Fatal("append: full list without a head"))
    );
    // Count already above max: the new node pushes it further.
    let mut fx = Fx::new();
    for g in 10..13 {
        fx.link(GOLEM, g);
    }
    fx.lists(P1).entries[GOLEM as usize].max = 2;
    assert_eq!(
        append(&mut fx, P1, GOLEM, None, [0; 3]),
        Err(PetError::Fatal("append: count above max"))
    );
}

// ---- remove and unlink ----------------------------------------

#[test]
fn remove_without_kill_clears_pet_flag() {
    let mut fx = Fx::new();
    let m = fx.spawn(10, 1);
    fx.link(SKELETON, 10);
    remove(&mut fx, P1, 10, false).unwrap();
    assert_eq!(fx.entry(SKELETON).count, 0);
    assert_eq!(fx.effects(), [Ev::Flags(m, 0)]);
    // Edge case 1: two broadcasts.
    assert_eq!(fx.sends(), to_all(rm(10), 2));
}

#[test]
fn remove_unlisted_guid_still_dismisses_and_broadcasts() {
    // Edge case 2.
    let mut fx = Fx::new();
    let m = fx.spawn(10, 1);
    remove(&mut fx, P1, 10, true).unwrap();
    assert_eq!(fx.effects(), dismissed(m));
    assert_eq!(fx.sends(), to_all(rm(10), 2));
    // Without kill: only Remove's broadcast.
    fx.log.clear();
    remove(&mut fx, P1, 10, false).unwrap();
    assert!(fx.effects().is_empty());
    assert_eq!(fx.sends(), to_all(rm(10), 1));
}

#[test]
fn remove_guards() {
    let mut fx = Fx::new();
    fx.spawn(10, 1);
    fx.units.get_mut(&P1).unwrap().lists = None;
    remove(&mut fx, P1, 10, true).unwrap();
    assert!(fx.log.is_empty());
    fx.units.get_mut(&P1).unwrap().data = false;
    assert_eq!(remove(&mut fx, P1, 10, true), Err(PetError::NoPlayerData));
}

#[test]
fn unlink_first_match_only_and_count_assertion() {
    let mut fx = Fx::new();
    fx.link(SKELETON, 10);
    fx.link(SKELETON, 11);
    fx.link(SKELETON, 10);
    unlink(&mut fx, P1, SKELETON, 10, false).unwrap();
    assert_eq!(fx.guids(SKELETON), [11, 10]);
    assert_eq!(fx.entry(SKELETON).count, 2);
    // Monster gone: no flag write, still one broadcast.
    assert!(fx.effects().is_empty());
    assert_eq!(fx.sends(), to_all(rm(10), 1));
    // Not found: nothing.
    fx.log.clear();
    unlink(&mut fx, P1, SKELETON, 99, true).unwrap();
    assert!(fx.log.is_empty());
    // Count below 0.
    fx.lists(P1).entries[SKELETON as usize].count = 0;
    assert_eq!(
        unlink(&mut fx, P1, SKELETON, 11, false),
        Err(PetError::Fatal("unlink: count below 0"))
    );
}

// ---- dismiss --------------------------------------------------

#[test]
fn dismiss_not_killable_requests_death_mode_at_owner() {
    let mut fx = Fx::new();
    let m = fx.spawn(10, 1);
    fx.units.get_mut(&m).unwrap().killable = false;
    dismiss(&mut fx, 10).unwrap();
    assert_eq!(
        fx.effects(),
        [
            Ev::Flags(m, FLAG_PET | FLAG_NO_EXPERIENCE),
            Ev::Mode(m, Some(P1))
        ]
    );
    assert_eq!(fx.sends(), to_all(rm(10), 1));
}

#[test]
fn dismiss_missing_unit_and_c8_assertion() {
    let mut fx = Fx::new();
    dismiss(&mut fx, 10).unwrap();
    assert!(fx.log.is_empty());
    let m = fx.spawn(10, 1);
    fx.units.get_mut(&m).unwrap().c8_bit8 = true;
    assert_eq!(
        dismiss(&mut fx, 10),
        Err(PetError::Fatal("dismiss: unit +0xC8 bit 8 set"))
    );
    assert!(fx.log.is_empty());
}

// ---- broadcast ------------------------------------------------

#[test]
fn pet_action_bytes() {
    // Test vector 4 (add, pet GUID 5, owner 1, class 363, type 4):
    // owner @5, pet @9.
    let msg = added(4, 363, 1, 5);
    assert_eq!(
        msg.bytes(),
        Some([0x7A, 0x01, 0x04, 0x6B, 0x01, 0x01, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00])
    );
    // A remove: type, class and owner 0, the GUID @9.
    assert_eq!(
        rm(5).bytes(),
        Some([0x7A, 0, 0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0])
    );
    // −1 GUID.
    assert_eq!(rm(-1).bytes().unwrap()[9..13], [0xFF; 4]);
}

#[test]
fn broadcast_skips_state_7_in_player_order() {
    let mut fx = Fx::new();
    fx.units.insert(
        3,
        FakeUnit {
            guid: 3,
            player: true,
            data: true,
            ..FakeUnit::default()
        },
    );
    fx.units.get_mut(&P2).unwrap().state7 = true;
    broadcast_remove(&mut fx, 9);
    assert_eq!(fx.sends(), [(P1, rm(9)), (3, rm(9))]);
}

#[test]
fn broadcast_add_record_selects_0x7a_or_0x81() {
    let mut fx = Fx::new();
    let mut r = AddRecord {
        pet_guid: 5,
        owner_guid: 1,
        class: 338,
        pet_type: PETTYPE_HIREABLE,
        x10: 0,
        x14: 0,
    };
    broadcast_add(&mut fx, &r);
    assert_eq!(fx.sends(), to_all(added(7, 338, 1, 5), 1));
    for (x10, x14) in [(1, 0), (0, 1)] {
        fx.log.clear();
        r.x10 = x10;
        r.x14 = x14;
        broadcast_add(&mut fx, &r);
        let m = PetMsg::AssignMerc { record: r };
        assert_eq!(fx.sends(), to_all(m, 1));
        assert_eq!(m.bytes(), None);
    }
}

// ---- lookup ---------------------------------------------------

#[test]
fn lookup_hireling_is_7() {
    // Test vector 3.
    let mut fx = Fx::new();
    fx.link(SKELETON, 10);
    fx.link(PETTYPE_HIREABLE, 50);
    assert_eq!(lookup(&mut fx, P1, 50), Ok(7));
    assert_eq!(lookup(&mut fx, P1, 10), Ok(SKELETON));
    assert_eq!(lookup(&mut fx, P1, 99), Ok(0));
}

#[test]
fn lookup_order_row_zero_and_guards() {
    let mut fx = Fx::new();
    // Row 0 is not searched.
    fx.link(0, 10);
    assert_eq!(lookup(&mut fx, P1, 10), Ok(0));
    // The lowest row wins.
    fx.link(GOLEM, 10);
    fx.link(WOLF, 10);
    assert_eq!(lookup(&mut fx, P1, 10), Ok(WOLF));
    fx.units.get_mut(&P1).unwrap().lists = None;
    assert_eq!(lookup(&mut fx, P1, 10), Ok(0));
    fx.units.get_mut(&P1).unwrap().data = false;
    assert_eq!(lookup(&mut fx, P1, 10), Err(PetError::NoPlayerData));
}

// ---- creation, free, death and resync -------------------------

fn maxes(fx: &mut Fx) -> Vec<i32> {
    fx.lists(P1).entries.iter().map(|e| e.max).collect()
}

#[test]
fn create_sets_basemax_and_resync_rules() {
    let mut fx = Fx::new();
    create(&mut fx, P1).unwrap();
    // Only single (1) and hireable (7) start with max 1.
    assert_eq!(maxes(&mut fx), [0, 1, 0, 0, 0, 0, 0, 1, 0, 0]);
    // Creating again frees the old lists first (pets dismissed).
    let a = fx.spawn(10, 7);
    fx.link(SKELETON, 10);
    fx.lists(P1).entries[SKELETON as usize].max = 3;
    create(&mut fx, P1).unwrap();
    assert!(fx.effects().contains(&Ev::Kill(a)));
    assert_eq!(maxes(&mut fx), [0, 1, 0, 0, 0, 0, 0, 1, 0, 0]);
    assert_eq!(fx.entry(SKELETON).count, 0);

    // Resync: the first skill met raises the maximum at once; a later,
    // lower one does not lower it; the others return to basemax.
    fx.skills = Some(vec![
        ResyncSkill {
            pettype: SKELETON as i8,
            petmax: 4,
        },
        ResyncSkill {
            pettype: SKELETON as i8,
            petmax: 2,
        },
        ResyncSkill {
            pettype: WOLF as i8,
            petmax: 0,
        },
        ResyncSkill {
            pettype: 0,
            petmax: 9,
        },
        ResyncSkill {
            pettype: COUNT as i8,
            petmax: 9,
        },
    ]);
    fx.lists(P1).entries[GOLEM as usize].max = 5;
    resync_max(&mut fx, P1).unwrap();
    assert_eq!(maxes(&mut fx), [0, 1, 4, 1, 0, 0, 0, 1, 0, 0]);
    // A lower petmax met first trims, a higher later one does not restore.
    for g in [20, 21, 22] {
        fx.spawn(g, 7);
        fx.link(SKELETON, g);
    }
    fx.skills = Some(vec![
        ResyncSkill {
            pettype: SKELETON as i8,
            petmax: 1,
        },
        ResyncSkill {
            pettype: SKELETON as i8,
            petmax: 3,
        },
    ]);
    resync_max(&mut fx, P1).unwrap();
    assert_eq!(fx.guids(SKELETON), [22]);
    assert_eq!(fx.entry(SKELETON).max, 3);
    // No skill list or no lists: nothing.
    fx.skills = None;
    fx.lists(P1).entries[GOLEM as usize].max = 5;
    resync_max(&mut fx, P1).unwrap();
    assert_eq!(fx.entry(GOLEM).max, 5);
    // Not a player: nothing.
    let m = fx.spawn(30, 1);
    resync_max(&mut fx, m).unwrap();
}

#[test]
fn free_and_player_death_drain_lists() {
    let mut fx = Fx::new();
    create(&mut fx, P1).unwrap();
    let a = fx.spawn(10, 7);
    let b = fx.spawn(11, 7);
    let h = fx.spawn(50, 9);
    fx.link(SKELETON, 10);
    fx.link(SKELETON, 11);
    fx.link(PETTYPE_HIREABLE, 50);
    fx.lists(P1).entries[SKELETON as usize].max = 2;
    player_death(&mut fx, P1).unwrap();
    // Pets dismissed in list order; the hireling is not touched.
    assert_eq!(
        fx.effects()
            .iter()
            .filter(|e| matches!(e, Ev::Kill(_)))
            .cloned()
            .collect::<Vec<_>>(),
        [Ev::Kill(a), Ev::Kill(b)]
    );
    assert!(fx.freed_hirelings.is_empty());
    let e = fx.entry(SKELETON);
    assert_eq!((e.count, e.nodes.len()), (0, 0));
    assert_eq!(fx.entry(PETTYPE_HIREABLE).count, 1);
    // Free: the hireling node's unit is announced removed and freed, the
    // lists are dropped.
    fx.log.clear();
    free_all(&mut fx, P1).unwrap();
    assert_eq!(fx.freed_hirelings, [h]);
    assert!(fx.units[&P1].lists.is_none());
    assert!(!fx.sends().is_empty());
    // P null: nothing; no player data: fatal.
    free_all(&mut fx, P1).unwrap();
    fx.units.get_mut(&P1).unwrap().data = false;
    assert_eq!(free_all(&mut fx, P1), Err(PetError::NoPlayerData));
    assert_eq!(create(&mut fx, P1), Err(PetError::NoPlayerData));
}
