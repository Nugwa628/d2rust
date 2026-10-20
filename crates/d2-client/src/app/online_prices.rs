//! The shop prices of an online game (the `online` feature): no server
//! runs in the process to publish them, so the client computes the
//! prices it shows from its own game data ([`super::shop_prices`]) and
//! writes them into the shop panel's board (contracts 3.7 v2). The
//! server prices every transaction itself.

use std::sync::Arc;

use bevy::prelude::*;

use super::session::Screen;
use super::shop_prices::{self, PriceTables, ShopView};
use crate::bridge::items;
use crate::bridge::mirror::BridgeResource;
use crate::bridge::world::{ClientWorld, ItemData, KindData, UnitKey};
use crate::ui::original::ShopPrices;
use crate::world_view::WorldViewUi;

/// The online game's price tables and the board the shop panel reads
/// (the one `install_online` hands the live client).
#[derive(Resource, Clone)]
pub struct OnlinePrices {
    pub tables: Arc<PriceTables>,
    pub board: ShopPrices,
}

/// What the last pricing read: the shop, the player's quest record and
/// price stats, and the items it priced.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Inputs {
    shop: ShopView,
    quest: [u8; 96],
    difficulty: u8,
    player: [i32; 2],
    items: Vec<(UnitKey, ItemData)>,
}

impl Inputs {
    fn read(w: &ClientWorld, shop: ShopView, quest: [u8; 96]) -> Self {
        let player = w.local_player.map_or([0; 2], |p| {
            [
                w.total(p, d2_sim::world::vendors::stat::REDUCED_PRICES, 0),
                w.total(p, d2_sim::world::vendors::stat::LEVEL, 0),
            ]
        });
        let items = items::items(w)
            .into_iter()
            .filter(|i| i.store || (i.owner == w.local_player && i.mode == items::mode::BODY))
            .filter_map(|i| match &w.units.get(&i.key)?.kind {
                KindData::Item(d) => Some((i.key, d.clone())),
                _ => None,
            })
            .collect();
        Self {
            shop,
            quest,
            difficulty: w.difficulty,
            player,
            items,
        }
    }
}

/// The pricing of online games (registered with the live client).
pub(crate) fn add_prices(app: &mut App) {
    app.add_systems(
        Update,
        publish_prices
            .run_if(in_state(Screen::Game))
            .run_if(resource_exists::<OnlinePrices>),
    );
}

/// While a shop is open, prices its items again whenever what the price
/// reads changed (a new store, a repair, a level).
fn publish_prices(
    prices: Res<OnlinePrices>,
    bridge: Option<Res<BridgeResource>>,
    ui: Option<NonSend<WorldViewUi>>,
    mut last: Local<Option<Inputs>>,
) {
    let (Some(bridge), Some(ui)) = (bridge, ui) else {
        return;
    };
    let Some(o) = ui.original.as_ref() else {
        return;
    };
    let shop = {
        let st = o.shop_state();
        st.open.map(|open| ShopView {
            npc_class: open.npc_class as u16,
            gamble: st.gamble(),
        })
    };
    let Some(shop) = shop else {
        *last = None;
        return;
    };
    let w = bridge.0.world();
    let quest = o.more().client_quest;
    let now = Inputs::read(w, shop, quest);
    if last.as_ref() == Some(&now) {
        return;
    }
    shop_prices::publish(&prices.tables, w, shop, Some(&quest), &prices.board);
    *last = Some(now);
}
