//! Coverage tests for item rules no other test pinned;
//! synthetic tables.

use super::*;
use crate::items::affixes::roll_affix;
use crate::items::create::normal;
use crate::items::ItemRequest;

/// The base-game `books.txt` rows are `tsc`/`tbk`, `isc`/`ibk` and a third; the
/// index goes in suffix slot 0 of `scro` and `book` items, the row count
/// when no row matches. A check that reads suffix slots as
/// affix ids skips that slot: the group test of the roller does.
#[test]
fn books_index_in_suffix_slot_zero() {
    let mut t = tables();
    t.books = vec![
        (*b"tsc ", *b"tbk "),
        (*b"isc ", *b"ibk "),
        (*b"xsc ", *b"xbk "),
    ];
    let mut ids = Vec::new();
    for (ty, code) in [
        (ty::SCRO, b"tsc "),
        (ty::BOOK, b"tbk "),
        (ty::SCRO, b"isc "),
        (ty::BOOK, b"ibk "),
        (ty::SCRO, b"0sc "),
    ] {
        ids.push(push_item(&mut t, item_rec(ty, code)));
    }
    for (i, want) in ids.iter().zip([0u16, 0, 1, 1, 3]) {
        let mut it = item(*i, 8);
        normal(&t, &mut it, &ItemRequest::default()).unwrap();
        assert_eq!(it.suffix[0], want, "item {i}");
    }

    // Suffix rows (ids 1 and 2, groups 1 and 2) that only a preferred
    // pick passes. Slot 0 of a scroll holds the books index 1, which is
    // not affix 1: its group must not block affix 1. On a ring the same
    // slot is affix 1 and its group is taken.
    let ring = push_item(&mut t, item_rec(RING, b"rng "));
    let roll = |itype: u16, record: usize| {
        let mut t = t.clone();
        t.magic = (1..=2)
            .map(|g| {
                let mut r = affix_row(itype, g);
                r.level = 99;
                r
            })
            .collect();
        t.n_suffix = 2;
        t.n_prefix = 0;
        let mut it = item(record, 6);
        it.suffix[0] = 1;
        roll_affix(&t, &mut it, true, true, false, false, 1, 0)
    };
    assert_eq!(roll(ty::SCRO, ids[2]), 1);
    assert_eq!(roll(RING, ring), 0);
}

fn forced_rq(i: usize, flags1: u32) -> crate::items::ItemRequest {
    crate::items::ItemRequest {
        item: i as i32,
        format: 101,
        force: true,
        quality: crate::items::q::NORMAL,
        seed: 77,
        item_seed: 5,
        flags1,
        ..Default::default()
    }
}

fn create(
    t: &ItemTables,
    rq: &mut crate::items::ItemRequest,
) -> Result<crate::items::Created<FakeStats>, crate::items::CreateError> {
    crate::items::create_item(
        t,
        &mut FakeGame::default(),
        rq,
        false,
        FakeStats::default(),
        0,
    )
}

#[test]
fn forced_flag_copies_and_format_zero_sockets() {
    use crate::items::{flag, stat};
    let mut t = tables();
    let mut r = item_rec(ty::HELM, b"cap ");
    r.durability = 12;
    r.minac = 3;
    r.maxac = 5;
    r.level = 1;
    r.gemsockets = 4;
    r.hasinv = 1;
    r.invwidth = 2;
    r.invheight = 2;
    let helm = push_item(&mut t, r.clone());
    t.itemtypes[ty::HELM as usize].maxsock1 = 4;
    let mut none = r.clone();
    none.gemsockets = 0;
    let nosock = push_item(&mut t, none);
    let mut tors = r;
    tors.type_ = ty::TORS as i16;
    let torso = push_item(&mut t, tors);
    t.itemtypes[ty::TORS as usize].maxsock1 = 4;
    // A version-0 ratio row for the format-0 quality roll (the request's
    // quality wins).
    let mut ratio = t.itemratio[0].clone();
    ratio.version = 0;
    (ratio.uniquedivisor, ratio.raredivisor, ratio.setdivisor) = (1, 1, 1);
    (
        ratio.magicdivisor,
        ratio.hiqualitydivisor,
        ratio.normaldivisor,
    ) = (1, 1, 1);
    t.itemratio.push(ratio);
    let want = flag::SOCKETED | flag::BROKEN | flag::STARTITEM;

    // The three flag copies run for every forced request, whatever the
    // format; a flag absent from flags1 is cleared.
    let mut rq = forced_rq(helm, want);
    let c = create(&t, &mut rq).unwrap();
    assert_eq!(c.item.flags & want, want);
    let mut rq = forced_rq(helm, 0);
    let c = create(&t, &mut rq).unwrap();
    assert_eq!(c.item.flags & want, 0);

    // Format 0: the socket count := start seed mod m + 1 (5 mod 4 + 1 = 2),
    // no draw beyond the normal creation.
    let mut rq = forced_rq(helm, flag::SOCKETED);
    rq.format = 0;
    rq.flags2 = crate::items::req::NO_SOCKETS;
    let c = create(&t, &mut rq).unwrap();
    assert_eq!(c.item.stats.base(stat::NUMSOCKETS, 0), 2);
    assert_ne!(c.item.flags & flag::SOCKETED, 0);
    // Format 101 does not run the socket step.
    let mut rq = forced_rq(helm, flag::SOCKETED);
    rq.flags2 = crate::items::req::NO_SOCKETS;
    let c = create(&t, &mut rq).unwrap();
    assert_eq!(c.item.stats.base(stat::NUMSOCKETS, 0), 0);
    // m = 0: no sockets; type `tors`: the step is skipped.
    for i in [nosock, torso] {
        let mut rq = forced_rq(i, flag::SOCKETED);
        rq.format = 0;
        rq.flags2 = crate::items::req::NO_SOCKETS;
        let c = create(&t, &mut rq).unwrap();
        assert_eq!(c.item.stats.base(stat::NUMSOCKETS, 0), 0, "item {i}");
    }
}

#[test]
fn personalized_flag_sets_the_name_only() {
    use crate::items::create::personalize;
    use crate::items::{CreateError, Fatal, ItemRequest, PlayerInfo, RequestUnit};
    let name = |s: &[u8]| {
        let mut n = [0u8; 16];
        n[..s.len()].copy_from_slice(s);
        n
    };
    // Forced: the request's name; nothing else changes.
    let mut it = item(0, 1);
    let before = it.clone();
    let rq = ItemRequest {
        force: true,
        name: name(b"forced"),
        ..Default::default()
    };
    personalize(&mut it, &rq).unwrap();
    assert_eq!(&it.name[..6], b"forced");
    it.name = before.name;
    assert_eq!(it, before);
    // Not forced: the request unit's player name.
    let rq = ItemRequest {
        unit: Some(RequestUnit {
            class: 1,
            player: Some(PlayerInfo {
                name: name(b"alice"),
                level: 9,
                hardcore: None,
            }),
        }),
        ..Default::default()
    };
    personalize(&mut it, &rq).unwrap();
    assert_eq!(&it.name[..5], b"alice");
    // No player data: a fatal error.
    let rq = ItemRequest::default();
    assert_eq!(
        personalize(&mut it, &rq),
        Err(CreateError::from(Fatal::NoPlayerData))
    );
}

#[test]
fn code_lookup() {
    let mut t = tables();
    let a = push_item(&mut t, item_rec(ty::HELM, b"cap "));
    let b = push_item(&mut t, item_rec(ty::HELM, b"skp "));
    assert_eq!(t.find_code(*b"cap "), Some(a));
    assert_eq!(t.find_code(*b"skp "), Some(b));
    // Space-padded exact 4 bytes; unknown codes (the `0` placeholder
    // included) are not found.
    assert_eq!(t.find_code(*b"cap\0"), None);
    assert_eq!(t.find_code(*b"0   "), None);
}

#[test]
fn create_from_index_request() {
    use crate::items::create::create_from_index;
    use crate::items::{flag, req, ItemRequest};
    let mut t = tables();
    let mut r = item_rec(ty::HELM, b"cap ");
    r.durability = 12;
    r.minac = 3;
    r.maxac = 5;
    r.level = 1;
    let i = push_item(&mut t, r);
    let mut g1 = FakeGame::default();
    let mut g2 = FakeGame::default();
    // ilvl ≤ 0 becomes 1; the seeds are used when asked; identified.
    let c = create_from_index(
        &t,
        &mut g1,
        None,
        i as i32,
        crate::items::q::NORMAL,
        true,
        true,
        0,
        true,
        77,
        88,
        FakeStats::default(),
        0,
    )
    .unwrap();
    let mut rq = ItemRequest {
        item: i as i32,
        quality: crate::items::q::NORMAL,
        format: 101,
        ilvl: 1,
        seed: 77,
        item_seed: 88,
        flags2: req::NO_SOCKETS | req::NEVER_ETHEREAL,
        ..Default::default()
    };
    let d = crate::items::create_item(&t, &mut g2, &mut rq, true, FakeStats::default(), 0).unwrap();
    assert_eq!(c.item.ilvl, 1);
    assert_eq!((c.item.init_seed, c.item.start_seed), (77, 88));
    assert_ne!(c.item.flags & flag::IDENTIFIED, 0);
    let mut same = d.item.clone();
    same.flags |= flag::IDENTIFIED;
    assert_eq!(c.item, same);
    assert_eq!(g1.seed, g2.seed);
    // An unknown index fails.
    let e = create_from_index(
        &t,
        &mut FakeGame::default(),
        None,
        99,
        crate::items::q::NORMAL,
        false,
        false,
        5,
        false,
        0,
        0,
        FakeStats::default(),
        0,
    );
    assert!(e.is_err());
}
