//! The hire list's row stats for the play app ([`OriginalUi::set_hire_stats`]):
//! the offer of a slot is a pure function of its seed, so the client asks
//! the same `hire_init` the server ran, over the `hireling`
//! rows. The server prices the hire with these values, so the row's level
//! and cost are the ones it charges.
//!
//! The act is the one of the slot's name (the first `hireling` row of the
//! game's version whose name range holds the name), the difficulty the game's,
//! and Level / Life / Def / Cost are words 1, 2, 7, 5 of the offer.

use d2_sim::world::hirelings::HirelingRows;

use crate::ui::panels::npc_menu::HireStats;
use crate::world_view::WorldViewUi;

/// The stats of the offer `(name, seed)` for a player of `level` on
/// `difficulty` (0–2).
pub fn hire_stats(
    rows: &HirelingRows,
    expansion: bool,
    difficulty: u8,
    name: u16,
    seed: u32,
    level: u32,
) -> Option<HireStats> {
    let act0 = rows.act_of_name(expansion, name);
    let o = rows.offer(
        expansion,
        i32::try_from(level).unwrap_or(i32::MAX),
        seed,
        act0,
        u32::from(difficulty),
    )?;
    Some(HireStats {
        level: o.level.max(0) as u32,
        hp: o.life.max(0) as u32,
        ac: o.defense.max(0) as u32,
        cost: o.price.max(0) as u32,
    })
}

/// Installs the stats closure over `rows` on the play UI; nothing
/// without it.
pub fn install_hire_stats(
    app: &mut impl crate::app::session::Setup,
    rows: HirelingRows,
    expansion: bool,
    difficulty: u8,
) {
    if let Some(mut ui) = app.world_mut().get_non_send_mut::<WorldViewUi>() {
        if let Some(o) = ui.original.as_mut() {
            o.set_hire_stats(Box::new(move |name, seed, level| {
                hire_stats(&rows, expansion, difficulty, name, seed, level)
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_sim::world::hirelings::HirelingRow;

    fn row(act: u32, diff: u32, first: u16, last: u16, gold: i32) -> HirelingRow {
        HirelingRow {
            version: 100,
            id: act * 10 + diff,
            class: 271,
            act,
            difficulty: diff,
            gold,
            level: 1,
            hp: 100,
            hp_lvl: 10,
            defense: 50,
            def_lvl: 5,
            name_first: first,
            name_last: last,
            ..HirelingRow::default()
        }
    }

    #[test]
    fn a_nightmare_act_2_offer_shows_the_servers_words() {
        let rows = HirelingRows::new(vec![
            row(1, 1, 100, 104, 100),
            row(2, 1, 200, 204, 300),
            row(2, 2, 200, 204, 900),
        ]);
        let s = hire_stats(&rows, true, 1, 202, 7, 10).expect("act 2 owns name 202");
        let o = rows.offer(true, 10, 7, 1, 1).unwrap();
        assert_eq!(rows.rows[o.row].difficulty, 2);
        assert_eq!(
            (s.level, s.hp, s.ac, s.cost),
            (
                o.level as u32,
                o.life as u32,
                o.defense as u32,
                o.price as u32
            )
        );
        assert!(s.cost >= 900 && s.hp >= 100 && s.ac >= 50);
        // The difficulty is the game's: Normal prices the Normal row.
        let n = hire_stats(&rows, true, 0, 202, 7, 10).unwrap();
        assert!(n.cost >= 300 && n.cost < 900);
        // A name no row owns falls to act 1 (none → 0).
        let u = hire_stats(&rows, true, 0, 999, 7, 10).unwrap();
        let o1 = rows.offer(true, 10, 7, 0, 0).unwrap();
        assert_eq!(u.cost, o1.price as u32);
    }
}
