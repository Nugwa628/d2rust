//! The waypoint menu (ui 0x14) installed in [`OriginalUi`]: the
//! [`WaypointPanel`] press / release rules over the rows rebuilt from the
//! record S→C 0x63 stored (`msg_ui`) and the levels' waypoint indexes
//! ([`OriginalUi::set_waypoint_map`]). A row's release leaves as C→S 0x49
//! through the root (the client decides nothing: the server validates and
//! warps).
//!
//! The tabs read the client quest flags (S→C 0x29): the open's tab and a
//! tab click walk down the setter's quest records 7 / 15 / 23 / 28, the
//! drawn tabs test 7 / 15 / 23 / 26. A press anywhere above the control
//! panel reaches the menu, so one outside the left half closes it. The
//! close button's hover draws the filled rectangle and "Cancel".
//!
//! The row text is the `levels` `LevelName` key looked up in the string
//! table.

use d2_sim::world::waypoints::{WaypointMap, WaypointRecord};

use super::{OriginalUi, SharedRef};
use crate::ui::draw::UiDrawSink;
use crate::ui::geom::{Point, Rect};
use crate::ui::panel::{Panel, PanelId, UiCtx, UiEvent, UiResponse, WidgetId};
#[cfg(doc)]
use crate::ui::panels::waypoint::{row_hit, tab_hit};
use crate::ui::panels::waypoint::{
    WaypointPanel, WaypointView, WpEffect, WpEnv, WpRow, WpRows, MAX_ROWS, ROW_HY, UI_WAYPOINT,
};
use crate::ui::panels::waypoint_rows::{self, TabCache, WaypointData};
use crate::ui::panels::{PanelOutput, TextMeasure};
use crate::ui::root::UiRoot;
use crate::ui::ui_ids::UiElement;
use crate::ui::PointerButton;

/// What the open menu reads: the stored 0x63 (GUID, record) and the
/// level the local player stood in when it opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WaypointOpen {
    pub guid: u32,
    pub record: WaypointRecord,
    /// The level of the player's room at the open.
    pub current: u32,
    /// Bumped by every 0x63, so the panel resets its latches.
    pub seq: u32,
    /// The tab the open chose (the act of the player's level, walked down
    /// through the quest gates).
    pub tab: u8,
}

/// The record's known bits over the level map.
struct Known<'a> {
    map: &'a WaypointMap,
    record: &'a WaypointRecord,
}

impl WaypointData for Known<'_> {
    fn index_of_level(&self, level: u32) -> Option<u8> {
        self.map.index_of_level(level)
    }
    fn known(&self, index: u8) -> bool {
        self.record.test(u32::from(index)).unwrap_or(false)
    }
    fn level_of_index(&self, index: u8) -> Option<u32> {
        self.map.level_of_index(u32::from(index))
    }
}

/// The rows of `tab` for the open menu; none without the map.
pub fn rows_of(map: Option<&WaypointMap>, open: &WaypointOpen, tab: u8) -> Vec<WpRow> {
    let Some(map) = map else {
        return Vec::new();
    };
    let data = Known {
        map,
        record: &open.record,
    };
    let mut cache = [TabCache::default(); 5];
    waypoint_rows::rebuild(usize::from(tab), &mut cache, open.current, &data)
        .rows
        .into_iter()
        .map(|r| WpRow {
            level: r.level as u16,
            known: r.used,
            current: r.level == open.current,
            name: Vec::new(),
        })
        .collect()
}

/// The row table the hit tests read.
pub fn hit_rows(rows: &[WpRow]) -> WpRows {
    let mut out = WpRows::default();
    for (i, r) in rows.iter().take(MAX_ROWS).enumerate() {
        out.used[i] = r.known && !r.current;
        out.level[i] = u32::from(r.level);
    }
    out
}

struct View {
    rows: Vec<WpRow>,
    quest: [u8; 96],
}

impl WaypointView for View {
    fn rows(&self, _tab: u8) -> &[WpRow] {
        &self.rows
    }
    fn tab_reachable(&self, tab: u8) -> bool {
        // The draw reads 26 for tab 4, not the setter's 28.
        crate::ui::panels::waypoint::TAB_DRAW_QUEST_RECORD
            .get(usize::from(tab))
            .is_some_and(|&r| crate::bridge::objects::quest_bit(&self.quest, r as u8, 0))
    }
    fn any_other_known(&self) -> bool {
        self.rows.iter().any(|r| r.known && !r.current)
    }
}

struct NoMeasure;

impl TextMeasure for NoMeasure {
    fn width(&self, _: u16, _: &[u16]) -> Option<i32> {
        None
    }
}

/// The waypoint menu adapter (ui 0x14, left slot).
pub(super) struct WaypointUi {
    pub(super) sh: SharedRef,
    pub(super) panel: WaypointPanel,
    pub(super) seq: u32,
}

/// The panel state a new 0x63 starts from: its guid and the open's tab.
fn fresh_panel(open: &WaypointOpen) -> WaypointPanel {
    let mut panel = WaypointPanel::new();
    panel.guid = open.guid;
    panel.tab = open.tab;
    panel
}

impl WaypointUi {
    /// The open record and its rows, the panel reset on a new 0x63.
    fn sync(&mut self) -> Option<Vec<WpRow>> {
        let sh = self.sh.borrow();
        let open = sh.waypoint_open?;
        if open.seq != self.seq {
            self.seq = open.seq;
            self.panel = fresh_panel(&open);
        }
        Some(rows_of(sh.waypoint_map.as_ref(), &open, self.panel.tab))
    }
}

impl Panel for WaypointUi {
    fn id(&self) -> PanelId {
        PanelId(u16::from(UI_WAYPOINT))
    }

    /// The whole screen: `mouse_down` closes on a press outside the left
    /// half and takes every press above the control panel.
    fn rect(&self) -> Rect {
        self.sh.borrow().config.screen.frame_rect()
    }

    fn draw(&self, ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        let sh = self.sh.borrow();
        let Some(open) = sh.waypoint_open else {
            return;
        };
        // A 0x63 no event has synced yet draws from its fresh state.
        let fresh;
        let panel = if open.seq != self.seq {
            fresh = fresh_panel(&open);
            &fresh
        } else {
            &self.panel
        };
        let mut rows = rows_of(sh.waypoint_map.as_ref(), &open, panel.tab);
        for r in &mut rows {
            if let Some(t) = sh
                .level_names
                .get(usize::from(r.level))
                .and_then(|k| ctx.strings.get(k))
            {
                r.name = t.to_vec();
            }
        }
        let view = View {
            rows,
            quest: sh.client_quest,
        };
        // The title is centred on its width: drawn with the
        // fonts bound, else not at all.
        let measure: &dyn TextMeasure = match sh.fonts.as_ref() {
            Some(f) => f,
            None => &NoMeasure,
        };
        panel.draw(&sh.tables, &sh.env(), &view, ctx.strings, measure, out);
        // After the rows: the close hover's filled rectangle and
        // the "Cancel" tip, `s` = half its width A.
        let s = sh.config.screen;
        if let Some(h) = crate::ui::panels::waypoint::close_hover(&s, sh.mouse) {
            let text = ctx
                .strings
                .get_id(h.tooltip)
                .map(<[u16]>::to_vec)
                .unwrap_or_default();
            let half = measure.width(1, &text).unwrap_or(0) / 2;
            let (sx, sy) = (s.sx(), (s.sy() - s.panel_dy()));
            out.push(crate::ui::UiDraw::Rect(crate::ui::draw::RectRequest {
                x0: sx + 287 - half,
                y0: 370 - sy,
                x1: sx + 294 + half,
                y1: 387 - sy,
                color: 0,
                mode: 2,
            }));
            out.push(crate::ui::panels::text(
                text,
                sx + 292 - half,
                h.tooltip_y,
                1,
                0,
            ));
        }
    }

    fn hit(&self, _p: Point) -> Option<WidgetId> {
        None
    }

    /// `waypoint.tab(N)` (act N, 1 to 5), `waypoint.row(N)` (the rows a
    /// click warps through, N from 0 top down, as listed on the open tab)
    /// and `waypoint.close`, at the areas the press rules hit-test
    /// ([`tab_hit`], [`row_hit`], the close button).
    fn ui_elements(&self, _ctx: &UiCtx) -> Vec<UiElement> {
        let sh = self.sh.borrow();
        let Some(open) = sh.waypoint_open else {
            return Vec::new();
        };
        let tab = if open.seq != self.seq {
            fresh_panel(&open).tab
        } else {
            self.panel.tab
        };
        let env = WpEnv::new(&sh.config.screen, sh.env().exp);
        waypoint_elements(
            &env,
            &hit_rows(&rows_of(sh.waypoint_map.as_ref(), &open, tab)),
        )
    }

    fn event(&mut self, e: UiEvent, _ctx: &UiCtx) -> UiResponse {
        let (press, at) = match e {
            UiEvent::Press {
                button: PointerButton::Left,
                at,
            } => (true, at),
            UiEvent::Release {
                button: PointerButton::Left,
                at,
            } => (false, at),
            _ => return UiResponse::Ignored,
        };
        let Some(rows) = self.sync() else {
            return UiResponse::Ignored;
        };
        let hit = hit_rows(&rows);
        let mut sh = self.sh.borrow_mut();
        // The tab hit steps by the game's tab count.
        let env = WpEnv::new(&sh.config.screen, sh.env().exp);
        // A tab click walks down the setter's quest records.
        let quest = sh.client_quest;
        let gate = |r: u32| crate::bridge::objects::quest_bit(&quest, r as u8, 0);
        let (effects, consumed) = if press {
            self.panel.mouse_down(&env, at.x, at.y, &hit, false, &gate)
        } else {
            self.panel.mouse_up(&env, at.x, at.y, &hit, false)
        };
        for e in effects {
            match e {
                WpEffect::Out(o) => sh.outputs.push(o),
                WpEffect::Sound(id) => sh.outputs.push(PanelOutput::Sound(id as i32)),
                WpEffect::RebuildRows => {}
            }
        }
        if consumed {
            UiResponse::Consumed
        } else {
            UiResponse::Ignored
        }
    }
}

/// The waypoint menu's elements for the screen `env` and the clickable
/// rows `rows`: the press rules work in panel coordinates (x − sx, y + sy),
/// so each area is moved back into the frame here.
fn waypoint_elements(env: &WpEnv, rows: &WpRows) -> Vec<UiElement> {
    let (sx, sy) = (env.sx, env.sy);
    // The tabs share the strip y' <= 30 left to right: 5 of 64 pixels in an
    // expansion game, else 4 of 80 ([`tab_hit`]).
    let (step, tabs) = if env.expansion_game { (64, 5) } else { (80, 4) };
    let mut out: Vec<UiElement> = (0..tabs)
        .map(|t| {
            UiElement::new(
                format!("waypoint.tab({})", t + 1),
                Rect::new(sx + step * t, -sy, step as u16, 31),
            )
        })
        .collect();
    // A row takes 17 < x' < 297 and its 29-pixel band below ROW_HY ([`row_hit`]).
    for r in (0..MAX_ROWS).filter(|&r| rows.used[r]) {
        out.push(UiElement::new(
            format!("waypoint.row({r})"),
            Rect::new(sx + 18, ROW_HY[r] + 1 - sy, 279, 29),
        ));
    }
    out.push(UiElement::new(
        "waypoint.close",
        Rect::new(sx + 273, 387 - sy, 36, 34),
    ));
    out
}

impl OriginalUi {
    /// Mirrors the UI flags on `root` ([`UiRoot::sync_states`]): the
    /// flags a delivered output set outside an event.
    pub fn sync_root(&self, root: &mut UiRoot) {
        root.sync_states(&self.shared.borrow().states);
    }

    /// The `levels` `LevelName` keys by level id, the rows' text.
    pub fn set_level_names(&mut self, names: Vec<String>) {
        self.shared.borrow_mut().level_names = names;
    }

    /// The levels' waypoint indexes the menu's rows read (`levels`
    /// `Waypoint`).
    pub fn set_waypoint_map(&mut self, map: WaypointMap) {
        self.shared.borrow_mut().waypoint_map = Some(map);
    }

    /// The rows of the open waypoint menu's tab (the one the open chose,
    /// [`WaypointOpen::tab`]); empty when closed.
    pub fn waypoint_rows(&self) -> Vec<WpRow> {
        let sh = self.shared.borrow();
        match sh.waypoint_open {
            Some(open) => rows_of(sh.waypoint_map.as_ref(), &open, open.tab),
            None => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The current level's row has used = 0: not known, so the draw skips
    // the first icon and only redraws frame 0.
    #[test]
    fn the_current_level_row_is_not_known() {
        // Levels 1, 3, 4 carry waypoint indexes 0, 1, 2 (act 0).
        let map = WaypointMap::from_pairs(vec![(255, 0), (0, 0), (255, 0), (1, 0), (2, 0)]);
        let mut record = WaypointRecord::allocate();
        record.set(1).unwrap();
        let open = WaypointOpen {
            guid: 1,
            record,
            current: 3,
            seq: 0,
            tab: 0,
        };
        let rows: Vec<_> = rows_of(Some(&map), &open, 0)
            .iter()
            .map(|r| (r.level, r.known, r.current))
            .collect();
        assert_eq!(
            rows,
            [(1, true, false), (3, false, true), (4, false, false)]
        );
    }

    #[test]
    fn each_element_is_hit_where_the_press_rules_look() {
        use crate::ui::panels::waypoint::{row_hit, tab_hit};
        let mut rows = WpRows::default();
        rows.used[0] = true;
        rows.used[3] = true;
        for (sx, sy, exp) in [(0, 0, true), (0, 0, false), (134, -60, true)] {
            let env = WpEnv {
                w: 800,
                h: 600,
                sx,
                sy,
                left_end: 400 + sx,
                right: 800 + 2 * sx,
                expansion_game: exp,
            };
            let els = waypoint_elements(&env, &rows);
            let ids: Vec<&str> = els.iter().map(|e| e.id.as_str()).collect();
            let tabs = if exp { 5 } else { 4 };
            assert_eq!(ids.len(), tabs + 3, "{ids:?}");
            assert!(ids.contains(&"waypoint.row(0)") && ids.contains(&"waypoint.row(3)"));
            for e in &els {
                let (xp, yp) = (e.at.x - sx, e.at.y + sy);
                if let Some(t) = e.id.strip_prefix("waypoint.tab(") {
                    let t: u8 = t.trim_end_matches(')').parse().unwrap();
                    assert_eq!(tab_hit(xp, yp, exp), Some(t - 1), "{}", e.id);
                } else if let Some(r) = e.id.strip_prefix("waypoint.row(") {
                    let r: usize = r.trim_end_matches(')').parse().unwrap();
                    assert_eq!(row_hit(xp, yp, &rows), Some(r), "{}", e.id);
                } else {
                    assert_eq!(e.id, "waypoint.close");
                    assert!((273..=308).contains(&xp) && (387..=420).contains(&yp));
                }
            }
        }
    }
}
