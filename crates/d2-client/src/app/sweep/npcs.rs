//! Town NPC interactions from loaded monster rows and the client's menu rules.
use super::*;
use crate::ui::{layout::OptionKind, panels::npc::NpcMenus};
use d2_data::tables::{Leveldefs, Monstats};
use d2_sim::world::{npc, vendors};

pub(super) fn generate(live: &LiveData, g: &mut Generated) -> Result<()> {
    let catalog = super::content::Catalog::load(live)?;
    let actors = live.tables.rows::<Monstats>()?;
    let areas = live.tables.rows::<Leveldefs>()?;
    let roles = npc::parse_npc_table(npc::VENDORS_TSV)?;
    let mut menus = NpcMenus::load()?;
    menus.reset_for_interaction();
    menus.apply_builder(12);
    menus.apply_resurrect(false, true);
    for (class, row) in actors.iter().enumerate() {
        if !row.enabled || !row.npc || !row.interact || !row.intown {
            g.skip("npcs:not-enabled-town-interaction");
            continue;
        }
        let Some(menu) = menus.menu(class as u32) else {
            g.skip("npcs:no-client-menu-definition");
            continue;
        };
        let act = roles
            .iter()
            .find(|r| usize::from(r.class) == class)
            .map_or(0, |r| r.act);
        let town = vendors::TOWNS[usize::from(act)];
        if areas.get(usize::from(town)).is_none_or(|a| a.drlgtype == 0) {
            g.skip("npcs:missing-town");
            continue;
        }
        let header =
            format!("char class 0\nchar level 12\nchar area {act} {town}\nchar stat 14 100000");
        // `walk` and `interact` move when the case needs setup first: the
        // identify case picks up its item before the player leaves its tile.
        let open_units = |walk: u32, interact: u32| open_steps(class, walk, interact);
        // Advance only an active greeting; extra Space presses close the menu.
        let mut greeting = String::new();
        for tick in (12..80).step_by(4) {
            greeting.push_str(&format!(
                "at {tick} key-if 32 client.dialog_lines != null\n"
            ));
        }
        greeting.push_str("at 84 check ui.open(npc_menu) == true\n");
        let open = format!("{}{greeting}", open_units(2, 4));
        let close = "at 112 input action clear_screen\nat 116 input action clear_screen\nat 120 input action clear_screen\nat 128 check ui.open(npc_menu) == false\nat 128 check ui.open(npc_shop) == false";
        g.add(
            "npcs",
            format!("npc-{class}-talk-close"),
            &header,
            &format!("{open}{close}"),
            132,
        )?;
        for (slot, option) in menu
            .options
            .iter()
            .take(menu.count.saturating_sub(1) as usize)
            .enumerate()
        {
            let Some(option) = option else {
                g.skip("npcs:empty-menu-slot");
                continue;
            };
            let expectation = match option.kind {
                OptionKind::Trade | OptionKind::Gamble => "at 108 check ui.open(npc_shop) == true",
                OptionKind::Identify => "by 108 check sent(0x34) > 0",
                OptionKind::Talk | OptionKind::Hire => "at 108 check ui.open(npc_menu) == true",
                OptionKind::Resurrect => {
                    g.skip("npcs:resurrection-needs-dead-hireling");
                    continue;
                }
                OptionKind::TravelWest | OptionKind::SailWest => {
                    g.skip("npcs:travel-needs-quest-state");
                    continue;
                }
                OptionKind::Imbue => {
                    g.skip("npcs:imbue-needs-quest-state");
                    continue;
                }
            };
            let mut case_open = open.clone();
            if option.kind == OptionKind::Identify {
                let Some(item) = catalog
                    .items
                    .iter()
                    .find(|i| catalog.valid(i) && catalog.is_type(i, 10))
                else {
                    g.skip("npcs:identify-needs-generatable-unidentified-item");
                    continue;
                };
                // The item is picked up and stored before the player walks
                // to the NPC: the pickup is processed on the tick after it is
                // sent, so the walk must not start until the item is in the
                // inventory (the menu counts unidentified items at its open).
                case_open = format!(
                    "{}{greeting}",
                    identify_setup(&item.code, item.class, class)
                );
                // Merge setup and interaction by tick, preserving equal-tick order.
                let mut lines: Vec<_> = case_open.lines().collect();
                lines.sort_by_key(|line| {
                    line.split_whitespace()
                        .nth(1)
                        .and_then(|n| n.parse::<u32>().ok())
                        .unwrap_or(0)
                });
                case_open = lines.join("\n") + "\n";
            }
            if option.kind == OptionKind::Hire {
                if live
                    .hirelings
                    .rows
                    .candidates(true, u32::from(act), 0)
                    .is_empty()
                {
                    g.skip("npcs:hire-needs-loaded-offer");
                } else {
                    g.add("npcs",format!("npc-{class}-hire-first-offer"),&header,
                        &format!("{open}at 92 input click ui:npc.menu({slot})\nat 100 input click ui:hire.offer(0)\nby 108 check sent(0x36) > 0\nby 108 check hud.gold < 100000\nat 112 msg TerminateEntityChat id=@1:{class}"),124)?;
                }
            }
            g.add(
                "npcs",
                format!(
                    "npc-{class}-menu-{slot}-{}",
                    format!("{:?}", option.kind).to_ascii_lowercase()
                ),
                &header,
                &format!(
                    "{case_open}at 92 input click ui:npc.menu({slot})\n{expectation}\n{close}"
                ),
                132,
            )?;
        }
        // Repair-all is a vendor request, including the valid empty-inventory case.
        if roles.iter().any(|r| {
            usize::from(r.class) == class && d2_sim::world::vendors::REPAIRERS.contains(&r.class)
        }) {
            g.add(
                "npcs",
                format!("npc-{class}-repair-all"),
                &header,
                &format!(
                    "{open}at 92 msg Repair npc=@1:{class} item=0 unread=0 repair_flags=1\n{close}"
                ),
                132,
            )?;
        }
    }
    Ok(())
}

/// The interaction setup: spawn the NPC beside the player, walk to it at
/// `walk`, interact at `interact`.
fn open_steps(class: usize, walk: u32, interact: u32) -> String {
    format!(
        "at 0 dev god on\nat 0 spawn {class} @x+3 @y normal\nat {walk} poke goto unit 1:{class}\nat {interact} msg InteractWithEntity type=1 id=@1:{class}\n"
    )
}

/// The identify case's item setup and interaction. The unidentified item
/// is poked at the player's feet, picked up and stored before the walk
/// starts: a sent message is handled on the next server tick, so a walk at
/// tick 2 would move the player away before the pickup ran.
fn identify_setup(code: &str, item_class: usize, npc_class: usize) -> String {
    // The pickup waits for the item's ground placement to settle (as the
    // item cases do), then the insert stores it before the walk.
    format!(
        "at 0 poke item {code} @x @y quality magic ilvl 12\nat 4 msg PickItem type=4 id=@4:{item_class} cursor=1\nat 5 msg InsertItemInBuffer item=@4:{item_class} x=0 y=0 page=0\n{}",
        open_steps(npc_class, 8, 10)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick_of(setup: &str, marker: &str) -> u32 {
        setup
            .lines()
            .find(|l| l.contains(marker))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|t| t.parse().ok())
            .unwrap_or_else(|| panic!("{marker} not in {setup}"))
    }

    #[test]
    fn the_identify_setup_picks_up_before_the_walk() {
        let setup = identify_setup("rin ", 522, 244);
        let insert = tick_of(&setup, "InsertItemInBuffer");
        let walk = tick_of(&setup, "poke goto");
        let interact = tick_of(&setup, "InteractWithEntity");
        assert!(insert < walk, "insert {insert}, walk {walk}");
        assert!(walk < interact, "walk {walk}, interact {interact}");
    }
}
