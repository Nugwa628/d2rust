//! The client's belt test against the server's, over every item of the
//! user's game tables: `fits_belt` of the art rows == the sim's
//! `beltable` and 1 x 1 test. Run with `D2_GAME_DIR`.

use crate::app_support;

use d2_client::app::items::item_parts;
use d2_client::ui::panels::inv_items::fits_belt;
use d2_sim::items::inventory::belt;
use d2_sim::items::inventory::tables::InvTables;

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_clients_belt_test_equals_the_sims_over_every_item() {
    let live = app_support::live();
    let parts = item_parts(live.archives.as_ref()).unwrap();
    let t = InvTables::from_fixed(&live.tables.fixed).unwrap();
    let (mut checked, mut belted) = (0, 0);
    for (i, r) in t.items.iter().enumerate() {
        let sim = belt::beltable(&t, i) && t.size(i) == Some((1, 1));
        assert_eq!(fits_belt(&parts.art, Some(r.code)), sim, "{:?}", r.code);
        checked += 1;
        belted += usize::from(sim);
    }
    assert!(checked > 500, "{checked} items");
    // The scrolls and the potions are in; the old list had no scrolls.
    for code in [*b"isc ", *b"tsc ", *b"hp1 ", *b"mp5 ", *b"rvs "] {
        assert!(fits_belt(&parts.art, Some(code)), "{code:?}");
    }
    assert!(!fits_belt(&parts.art, Some(*b"qui ")));
    assert!(belted > 10);
}
