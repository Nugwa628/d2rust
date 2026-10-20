//! Item moves and transactions use the same client messages as player input.
use super::content::{Catalog, Item};
use super::*;
use crate::assets::path::FileSource;
use crate::ui::{layout::OptionKind, panels::npc::NpcMenus};
use crate::world_view::ground_items::flippy_archive_name;
use d2_data::tables::Objects;
use d2_sim::world::{
    cube::{self, CubeData},
    npc,
};

/// The quest flag bits a ground item needs before the player may pick it up
/// (`d2_sim::items::moves::ground::can_pick`: the Scroll of Malah is pickable
/// once its quest has granted it and before it is read).
const PICKUP_QUEST_BITS: &[(&str, &[(u8, u8)])] = &[("tr2", &[(37, 8)])];

fn spawn(item: &Item) -> String {
    let quest: String = PICKUP_QUEST_BITS
        .iter()
        .filter(|(code, _)| *code == item.code)
        .flat_map(|(_, bits)| bits.iter())
        .map(|(slot, bit)| format!("at 0 dev quest {slot} {bit}\n"))
        .collect();
    format!("{quest}at 0 poke item {} @x @y quality normal ilvl 99\nat 4 msg PickItem type=4 id=@4:{} cursor=1\nat 8 check unit(item,{}).mode_id == 4\n", item.code, item.class, item.class)
}
pub(super) fn generate(live: &LiveData, g: &mut Generated) -> Result<()> {
    let catalog = Catalog::load(live)?;
    let objects = live.tables.rows::<Objects>()?;
    // The client's stash object class; row 5's operatefn 4 is a different
    // object and never opens the stash panel.
    let stash = Some(usize::from(d2_sim::world::objects::STASH_CLASS))
        .filter(|&class| objects.get(class).is_some());
    let item_tables = d2_sim::items::tables::ItemTables::from_fixed(&live.tables.fixed)?;
    for item in &catalog.items {
        if !catalog.valid(item) || item.width > 10 || item.height > 4 {
            g.skip("items:invalid-code-type-size-or-level");
            continue;
        }
        // A row whose ground art is in no archive (the unused monster-part
        // rows) can never lie on the floor of this game.
        if !item.flippy.is_empty()
            && live
                .archives
                .read_file(&flippy_archive_name(&item.flippy))
                .is_none()
        {
            g.skip("items:ground-art-not-in-archives");
            continue;
        }
        // Gold is consumed by its own pickup path rather than held on the cursor.
        if item.code == "gld" {
            g.skip("items:gold-is-not-a-cursor-item");
            continue;
        }
        let class = catalog.types[usize::from(item.ty)].class.min(6);
        // Cases create at item level 99, so the character must meet the level
        // requirement that carries; the item row's own requirement is still
        // part of the loaded tables but ilvl 99 dominates it.
        let header = format!(
            "char class {class}\nchar level 99\nchar stat 0 {}\nchar stat 2 {}",
            item.strength.clamp(1, 1023),
            item.dexterity.clamp(1, 1023)
        );
        let start = spawn(item);
        let id = item.class;
        g.add("items", format!("item-{id}-pickup-drop"), &header,
            &format!("{start}at 12 msg InsertItemInBuffer item=@4:{id} x=0 y=0 page=0\nat 16 check unit(item,{id}).mode_id == 0\nat 20 msg RemoveItemFromBuffer item=@4:{id}\nat 24 msg DropItem item=@4:{id}\nat 28 check unit(item,{id}).mode_id == 3"), 32)?;
        if let Some(body) = (1..=10).find(|&body| catalog.fits(item, class, body)) {
            g.add("items", format!("item-{id}-equip"), &header,
                &format!("{start}at 12 msg InsertItemInBuffer item=@4:{id} x=0 y=0 page=0\nat 16 dev identify\nat 20 msg RemoveItemFromBuffer item=@4:{id}\nat 24 msg EquipItem item=@4:{id} bodyloc={body}\nat 28 check unit(item,{id}).mode_id == 1\nat 32 msg RemoveBodyItem bodyloc={body}\nat 36 check unit(item,{id}).mode_id == 4\nat 40 msg DropItem item=@4:{id}"), 44)?;
        } else {
            g.skip("items:not-equippable");
        }
        if let Some(stash) = stash {
            if item.width <= 6 && item.height <= 8 {
                g.add("items", format!("item-{id}-stash"), &header,
                    &format!("{start}at 12 poke object {stash} @x+3 @y\nat 14 poke goto unit 2:{stash}\nat 16 msg InteractWithEntity type=2 id=@2:{stash}\nat 24 check ui.open(stash) == true\nat 28 msg InsertItemInBuffer item=@4:{id} x=0 y=0 page=4\nat 32 check unit(item,{id}).mode_id == 0\nat 36 msg RemoveItemFromBuffer item=@4:{id}\nat 40 check unit(item,{id}).mode_id == 4\nat 44 msg DropItem item=@4:{id}\nat 48 input action clear_screen"), 52)?;
            } else {
                g.skip("items:too-large-for-stash");
            }
        } else {
            g.skip("items:no-stash-object");
        }
        // Belt eligibility is the loaded type hierarchy, not a list of item codes.
        if item.usable && catalog.types[usize::from(item.ty)].beltable != 0 {
            g.add("items", format!("item-{id}-belt-use"), &header,
                &format!("{start}at 12 msg ItemToBelt item=@4:{id} slot=0\nat 16 check unit(item,{id}).mode_id == 2\nat 20 msg UseBeltItem item=@4:{id} on_merc=0\nby 32 check count(item,{id}) == 0"), 36)?;
        } else {
            g.skip("items:not-a-belt-consumable");
        }
        if item.sockets > 0 {
            sockets(&item_tables, &catalog, item, &header, g)?;
        }
        // Repair requires a durable item. Actual damage is setup; the vendor repairs it.
        if item.durability > 0 {
            repair(live, item, &header, g)?;
        }
    }
    recipes(live, &catalog, g)?;
    trade(live, g)?;
    Ok(())
}

fn repair(live: &LiveData, item: &Item, header: &str, g: &mut Generated) -> Result<()> {
    let roles = npc::parse_npc_table(npc::VENDORS_TSV)?;
    let monsters = live.tables.rows::<d2_data::tables::Monstats>()?;
    let Some(vendor) = roles.iter().find(|r| {
        r.act == 0
            && d2_sim::world::vendors::REPAIRERS.contains(&r.class)
            && monsters
                .get(usize::from(r.class))
                .is_some_and(|m| m.enabled && m.npc)
    }) else {
        g.skip("items:no-repair-vendor");
        return Ok(());
    };
    let id = item.class;
    let npc = vendor.class;
    g.add("items", format!("item-{id}-repair"), &format!("{header}\nchar stat 14 10000"),
        &format!("{}at 12 msg InsertItemInBuffer item=@4:{id} x=0 y=0 page=0\nat 16 poke stat @4:{id} 72 0 1\nat 20 spawn {npc} @x+3 @y normal\nat 22 poke goto unit 1:{npc}\nat 24 poke talk @1:{npc} trade\nat 32 msg Repair npc=@1:{npc} item=@4:{id} unread=0 repair_flags=0\nat 40 msg TerminateEntityChat id=@1:{npc}" ,spawn(item)), 44)
}

fn trade(live: &LiveData, g: &mut Generated) -> Result<()> {
    let monsters = live.tables.rows::<d2_data::tables::Monstats>()?;
    let mut menus = NpcMenus::load()?;
    menus.reset_for_interaction();
    menus.apply_builder(12);
    menus.apply_resurrect(false, true);
    for vendor in npc::parse_npc_table(npc::VENDORS_TSV)? {
        if !vendor.trade_action
            || !monsters
                .get(usize::from(vendor.class))
                .is_some_and(|m| m.enabled && m.npc)
        {
            g.skip("items:not-loaded-trade-vendor");
            continue;
        }
        let npc = vendor.class;
        let act = vendor.act;
        let town = d2_sim::world::vendors::TOWNS[usize::from(act)];
        // The player's way in: walk up, talk, skip the greeting, pick the
        // menu's Trade line, then click the first stocked item and confirm.
        let Some(menu) = menus.menu(u32::from(npc)) else {
            g.skip("items:vendor-without-client-menu");
            continue;
        };
        let Some(slot) = menu
            .options
            .iter()
            .take(menu.count.saturating_sub(1) as usize)
            .position(|o| o.as_ref().is_some_and(|o| o.kind == OptionKind::Trade))
        else {
            g.skip("items:vendor-menu-without-trade");
            continue;
        };
        let mut greeting = String::new();
        for tick in (12..80).step_by(4) {
            greeting.push_str(&format!(
                "at {tick} key-if 32 client.dialog_lines != null\n"
            ));
        }
        g.add("items", format!("vendor-{npc}-buy"), &format!("char class 0\nchar level 12\nchar area {act} {town}\nchar stat 14 100000"),
            &format!("at 0 spawn {npc} @x+3 @y normal\nat 2 poke goto unit 1:{npc}\nat 4 msg InteractWithEntity type=1 id=@1:{npc}\n{greeting}at 84 check ui.open(npc_menu) == true\nat 92 input click ui:npc.menu({slot})\nat 100 check ui.open(npc_shop) == true\nat 104 input click ui:shop.item(0)\nat 108 input click ui:shop.confirm.yes\nby 124 check sent(0x32) > 0\nby 124 check hud.gold < 100000\nat 128 input action clear_screen"), 132)?;
    }
    let catalog = Catalog::load(live)?;
    if let Some(item) = catalog
        .items
        .iter()
        .find(|i| catalog.valid(i) && catalog.fits(i, 0, 1))
    {
        let id = item.class;
        if monsters.get(148).is_some_and(|m| m.enabled && m.npc) {
            g.add("items", format!("item-{id}-sell"), "char class 0\nchar stat 14 0", &format!("{}at 12 msg InsertItemInBuffer item=@4:{id} x=0 y=0 page=0\nat 16 spawn 148 @x+3 @y normal\nat 18 poke goto unit 1:148\nat 20 poke talk @1:148 trade\nat 22 key-if 32 client.dialog_lines != null\nat 24 key-if 32 client.dialog_lines != null\nat 26 key-if 32 client.dialog_lines != null\nat 28 msg SellItem npc=@1:148 item=@4:{id} item_mode=0 client_price=0\nby 40 check hud.gold > 0\nat 44 input action clear_screen",spawn(item)),48)?;
        } else {
            g.skip("items:no-loaded-sale-vendor");
        }
    }
    Ok(())
}

fn recipes(live: &LiveData, catalog: &Catalog, g: &mut Generated) -> Result<()> {
    let cube_data = CubeData::from_fixed(&live.tables.fixed)?;
    let Some(box_item) = catalog.items.iter().find(|i| i.code == "box") else {
        g.skip("items:no-cube");
        return Ok(());
    };
    for (index, recipe) in cube_data.recipes.iter().enumerate() {
        if recipe.enabled == 0 {
            g.skip("items:cube-disabled-recipe");
            continue;
        }
        if recipe.op != 0 || recipe.min_diff > 0 || recipe.ladder != 0 {
            g.skip("items:cube-needs-condition-difficulty-or-ladder");
            continue;
        }
        let mut inputs = Vec::new();
        let mut valid = true;
        for input in &recipe.inputs {
            if input.quantity == 0 {
                continue;
            }
            if input.flags & !cube::input_flags::ITEMCODE != 0
                || input.special != 0
                || !matches!(input.quality, 0 | 2)
            {
                valid = false;
                break;
            }
            let found = catalog.items.iter().find(|i| {
                catalog.valid(i)
                    && if input.flags & cube::input_flags::ITEMCODE != 0 {
                        i.class == usize::from(input.item)
                    } else {
                        catalog.is_type(i, i32::from(input.item))
                    }
            });
            if let Some(item) = found {
                inputs.extend(std::iter::repeat_n(item, usize::from(input.quantity)));
            } else {
                valid = false;
                break;
            }
        }
        let output = recipe
            .outputs
            .iter()
            .find(|o| o.kind == cube::kind::ITEMCODE)
            .and_then(|o| catalog.items.get(usize::from(o.item)));
        if !valid
            || inputs.len() != usize::from(recipe.numinputs)
            || inputs.is_empty()
            || output.is_none()
        {
            g.skip("items:cube-needs-special-input-or-nonitem-output");
            continue;
        }
        let mut occupied = [[false; 3]; 4];
        let mut positions = Vec::new();
        for item in &inputs {
            let position = (0..4usize)
                .flat_map(|y| (0..3usize).map(move |x| (x, y)))
                .find(|&(x, y)| {
                    x + usize::from(item.width) <= 3
                        && y + usize::from(item.height) <= 4
                        && (y..y + usize::from(item.height))
                            .all(|yy| (x..x + usize::from(item.width)).all(|xx| !occupied[yy][xx]))
                });
            if let Some((x, y)) = position {
                for row in occupied.iter_mut().skip(y).take(usize::from(item.height)) {
                    for cell in row.iter_mut().skip(x).take(usize::from(item.width)) {
                        *cell = true;
                    }
                }
                positions.push((x, y));
            } else {
                valid = false;
                break;
            }
        }
        if !valid {
            g.skip("items:cube-inputs-do-not-fit");
            continue;
        }
        let mut steps=format!("{}at 12 msg InsertItemInBuffer item=@4:{} x=0 y=0 page=0\nat 16 msg UseGridItem item=@4:{} x=@x y=@y\nat 20 check ui.open(cube) == true\n",spawn(box_item),box_item.class,box_item.class);
        let mut occurrences = BTreeMap::<usize, usize>::new();
        for (n, (item, (x, y))) in inputs.iter().zip(positions).enumerate() {
            let occurrence = occurrences.entry(item.class).or_default();
            let id = item.class;
            let t = 24 + n * 12;
            let reference = super::content::reference(id, *occurrence);
            steps.push_str(&format!("at {t} poke item {} @x @y quality normal ilvl 99\nat {} msg PickItem type=4 id={reference} cursor=1\nat {} msg InsertItemInBuffer item={reference} x={x} y={y} page=3\n",item.code,t+4,t+8));
            *occurrence += 1;
        }
        let t = 24 + inputs.len() * 12;
        let output = output.expect("checked output");
        steps.push_str(&format!("at {t} msg ClickButton button=24 p1=0 p2=0\nby {} check count(item,{}) > 0\nat {} input action clear_screen",t+16,output.class,t+20));
        g.add(
            "items",
            format!("recipe-{index}"),
            &format!("char class {}\nchar level 99", recipe.class.min(6)),
            &steps,
            (t + 24) as u32,
        )?;
    }
    Ok(())
}

// Socket fixtures use a seed selected by the loaded creation rules. The live
// game still creates and sockets the items through its ordinary handlers.
fn sockets(
    tables: &d2_sim::items::tables::ItemTables,
    catalog: &Catalog,
    target: &Item,
    header: &str,
    g: &mut Generated,
) -> Result<()> {
    use d2_sim::{
        items::{create, flag, ItemGame, ItemRequest},
        rng::Seed,
        wiring::economy::game_fields::GameFields,
    };
    let Some(filler) = catalog
        .items
        .iter()
        .find(|i| catalog.valid(i) && (catalog.is_type(i, 20) || catalog.is_type(i, 74)))
    else {
        g.skip("items:no-socket-filler");
        return Ok(());
    };
    let seed = (1..=256).find(|&seed| {
        let mut game = GameFields::new(Seed { lo: seed, hi: 666 }, true);
        let mut request = ItemRequest {
            item: target.class as i32,
            ilvl: 99,
            quality: 2,
            format: game.item_format(),
            ..Default::default()
        };
        create::create_item(
            tables,
            &mut game,
            &mut request,
            false,
            ProbeStats::default(),
            0,
        )
        .is_ok_and(|c| c.item.flags & flag::SOCKETED != 0)
    });
    let Some(seed) = seed else {
        g.skip("items:no-socketed-seed-in-bounded-search");
        return Ok(());
    };
    let id = target.class;
    let gem = filler.class;
    g.add("items",format!("item-{id}-socket"),header,&format!("at 0 poke seed-game {seed} 666\nat 0 poke item {} @x @y quality normal ilvl 99\nat 4 msg PickItem type=4 id=@4:{id} cursor=0\nat 8 poke item {} @x @y quality normal ilvl 99\nat 12 msg PickItem type=4 id=@4:{gem} cursor=1\nat 16 msg SocketItem socketable=@4:{gem} target=@4:{id}\nat 24 check unit(item,{gem}).mode_id == 6",target.code,filler.code),28)
}
#[derive(Default)]
struct ProbeStats(
    BTreeMap<(u16, u16), i32>,
    BTreeMap<(d2_sim::items::ListKey, u16, u16), i32>,
);
impl d2_sim::items::ItemStats for ProbeStats {
    fn has_stats(&self) -> bool {
        !self.0.is_empty() || !self.1.is_empty()
    }
    fn base(&self, id: u16, layer: u16) -> i32 {
        self.0.get(&(id, layer)).copied().unwrap_or(0)
    }
    fn stat(&self, id: u16, layer: u16) -> i32 {
        self.base(id, layer)
            + self
                .1
                .iter()
                .filter(|((_, i, l), _)| *i == id && *l == layer)
                .map(|(_, v)| *v)
                .sum::<i32>()
    }
    fn set_base(&mut self, id: u16, layer: u16, value: i32) {
        self.0.insert((id, layer), value);
    }
    fn has_list(&self, key: d2_sim::items::ListKey) -> bool {
        self.1.keys().any(|(k, _, _)| *k == key)
    }
    fn list_set(&mut self, key: d2_sim::items::ListKey, id: u16, layer: u16, value: i32) {
        self.1.insert((key, id, layer), value);
    }
    fn list_get(&self, key: d2_sim::items::ListKey, id: u16, layer: u16) -> i32 {
        self.1.get(&(key, id, layer)).copied().unwrap_or(0)
    }
    fn list_add(&mut self, key: d2_sim::items::ListKey, id: u16, layer: u16, value: i32) {
        *self.1.entry((key, id, layer)).or_default() += value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(code: &str) -> Item {
        Item {
            class: 646,
            code: code.into(),
            flippy: String::new(),
            ty: 0,
            ty2: 0,
            level: 0,
            strength: 0,
            dexterity: 0,
            width: 1,
            height: 1,
            durability: 0,
            sockets: 0,
            stackable: false,
            usable: false,
            two_handed: false,
        }
    }

    // The Scroll of Malah can only be picked up once its quest granted it, so
    // its case sets that bit before the pickup; other items add no setup.
    #[test]
    fn a_quest_gated_pickup_sets_its_quest_bit_first() {
        let tr2 = spawn(&item("tr2"));
        assert!(
            tr2.starts_with("at 0 dev quest 37 8\nat 0 poke item tr2 "),
            "{tr2}"
        );
        let cap = spawn(&item("cap"));
        assert!(cap.starts_with("at 0 poke item cap "), "{cap}");
    }
}
