//! The `autoplay-1` line protocol: the one vocabulary every tool that
//! drives the game step by step speaks. `autoplay-host` (the headless
//! client on a stepping clock) and `play --control` (the real game,
//! windowed or offscreen, [`super`]) both parse their command lines here
//! and answer the reads (`state`, `query`, `map`) with the same JSON.
//!
//! A command is one line; every reply is one JSON object on one line with
//! a `k` field (`hello`, `ok`, `state`, `query`, `map`, `shot`, `error`).
//! Input commands (`move`, `press`, `release`, `key`) go into the UI's
//! event queue exactly where the window's pointer and key events go, so
//! a click passes the panels, the hover pick and the world-click
//! dispatcher like a player's. Reads only read: the server snapshot of
//! `state-dump` and the DRLG's rooms, taken on the server thread between
//! two frames, and the client's UI and model (CLAUDE.md rule 6).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use bevy::prelude::*;
use d2_server::seams::Clock;
use serde_json::{json, Value};

use crate::app::server_thread::ThreadLink;
use crate::app::single_player::{self, Link};
use crate::bridge::items;
use crate::bridge::state::StateSource;
use crate::bridge::BridgeResource;
use crate::ui::edge;
use crate::ui::{FramePos, Point, PointerButton, UiEvent};
use crate::world_view::input_script::{self, play_key, vk_code, Step, UnitSel};
use crate::world_view::WorldViewUi;

/// The protocol's format word (`autoplay-1`).
pub const FORMAT: &str = "autoplay-1";

/// Message ids with counts (the C→S ids the client sent, the S→C ids it
/// received), for the `state` reply.
pub type MessageLog = Arc<Mutex<BTreeMap<u8, u32>>>;

/// Counts the caller keeps for the `state` reply.
pub struct Counts<'a> {
    /// Client passes run so far.
    pub passes: u64,
    pub sent: &'a MessageLog,
    pub received: &'a MessageLog,
}

/// Adds one message to `log` under its first byte.
pub fn count(log: &MessageLog, msg: &[u8]) {
    if let Some(&id) = msg.first() {
        *log.lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(id)
            .or_default() += 1;
    }
}

/// The GUID of the local player, once it exists on the server.
pub fn local_player<C: Clock + Send + 'static>(server: &Mutex<ThreadLink<Link<C>>>) -> Option<u32> {
    server
        .lock()
        .expect("server lock")
        .with(|l| single_player::local_player(&l.host().game).map(|(_, g)| g))
        .ok()
        .flatten()
}

/// Queues pointer events as the window path does (`ui_input`): the
/// cursor first, then the event; the cursor the click view reads.
pub fn pointer(ui: &mut WorldViewUi, at: Point, e: Option<UiEvent>) {
    if ui.cursor() != Some(FramePos::Inside(at)) {
        ui.queue.0.push(UiEvent::CursorMoved(at));
        ui.set_cursor(FramePos::Inside(at));
    }
    ui.queue.0.extend(e);
}

/// The window key that types virtual key `vk` (at most one).
pub fn key_codes(vk: u32) -> Vec<KeyCode> {
    let Some(k) = play_key(vk) else {
        return Vec::new();
    };
    edge::KEY_CODES
        .iter()
        .filter(|(_, e)| *e == k)
        .map(|(c, _)| *c)
        .take(1)
        .collect()
}

/// A key press as `script_input` delivers a `key` step: the bound action
/// in the UI's key mode, then the typed character. Returns the window
/// keys the caller holds down for the next pass (the death and hardcore
/// screens read the raw key state).
pub fn key(ui: &mut WorldViewUi, vk: u32) -> Vec<KeyCode> {
    let codes = key_codes(vk);
    if let Some(b) = ui.bindings.clone() {
        let mode = ui.original.as_ref().map_or(1, |o| o.key_mode());
        let actions = edge::key_actions_in_mode(&b, &codes, mode);
        ui.queue.0.extend(actions);
    }
    let chars = edge::key_chars(&codes);
    ui.queue.0.extend(chars);
    codes
}

/// The `query` reply for `names`, read from a `state` reply.
pub fn query_reply(names: &[crate::app::repro::query::Name], state: &Value) -> Value {
    json!({
        "k": "query",
        "server_frame": state["snap"]["f"],
        "values": crate::app::repro::query::query(names, state),
    })
}

/// One command line of the protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// `step N`: N client passes (frames). On `autoplay-host` each pass
    /// moves the clock one game tick, using rational rate deadlines.
    Step(u32),
    /// `step-ticks N`: run until N more server ticks have run, then hold
    /// the game paused. On `autoplay-host` it is `step N`.
    StepTicks(u32),
    /// `pause`: hold the game between two server ticks; the picture,
    /// the UI and the reads go on.
    Pause,
    /// `resume`: let the server tick again.
    Resume,
    /// `shot PATH`: a PNG of the presented picture (a `play` run only).
    Shot(std::path::PathBuf),
    /// `move X Y`.
    Move(Point),
    /// `press L|R X Y` / `release L|R X Y`.
    Press(PointerButton, Point),
    Release(PointerButton, Point),
    /// `key K`: a key press through the bindings (`script_input`).
    Key(u32),
    /// `rawkey K`: a key through the window's own input path only
    /// (`world_view::present::ui_input` reading the key state of a
    /// primary window), none of `key`'s direct queueing.
    RawKey(u32),
    /// `query NAME, ...`: the named values (`repro::query`) as one
    /// object.
    Query(Vec<crate::app::repro::query::Name>),
    /// `click ui:ID`: a left click on the element with that id on screen
    /// now ([`crate::ui::ui_ids`]); an unknown id is an error listing the
    /// ids on screen.
    ClickUi(String),
    /// T2 `clickunit T C[,C..]|* [DX DY]` (or `rclickunit`): resolve the
    /// unit through the client model, hover for one pass, then click.
    ClickUnit(UnitSel),
    /// `interactunit T C[,C..]|*`: approach and request a unit interaction.
    /// It walks to the selected unit even when it is outside the frame.
    InteractUnit(UnitSel),
    /// `useunit T C[,C..]|*`: request the client's interact sender now.
    /// The server still checks whether the player is in range.
    UseUnit(UnitSel),
    /// `walklevel N`: walk through the map into a connected level.
    WalkLevel(u32),
    /// `ui`: the ids on screen, with their rectangles.
    Ui,
    /// `text WORD`: the characters of WORD typed (the front end's name
    /// box).
    Text(String),
    /// `state`, `map`, `quit`.
    State,
    Map,
    Quit,
}

/// Parses one command line.
pub fn parse_command(line: &str) -> Result<Command, String> {
    let w: Vec<&str> = line.split_whitespace().collect();
    let num =
        |s: &str| -> Result<i32, String> { s.parse().map_err(|_| format!("`{s}`: not a number")) };
    let point = |x: &str, y: &str| -> Result<Point, String> {
        let (x, y) = (num(x)?, num(y)?);
        let f = crate::scene::Rect::world_clip();
        let (w, h) = (f.width as i32, f.height as i32);
        if !(0..w).contains(&x) || !(0..h).contains(&y) {
            return Err(format!("({x}, {y}) is outside the {w} × {h} frame"));
        }
        Ok(Point { x, y })
    };
    let button = |b: &str| match b {
        "L" | "l" => Ok(PointerButton::Left),
        "R" | "r" => Ok(PointerButton::Right),
        _ => Err(format!("`{b}`: L or R")),
    };
    match w.as_slice() {
        ["step", n] => Ok(Command::Step(
            n.parse().map_err(|_| format!("`{n}`: not a count"))?,
        )),
        ["step-ticks", n] => Ok(Command::StepTicks(
            n.parse().map_err(|_| format!("`{n}`: not a count"))?,
        )),
        ["pause"] => Ok(Command::Pause),
        ["resume"] => Ok(Command::Resume),
        ["shot", ..] => match line.trim_start().strip_prefix("shot").map(str::trim) {
            Some(p) if !p.is_empty() => Ok(Command::Shot(p.into())),
            _ => Err("shot needs a PATH".into()),
        },
        ["walklevel", n] => n
            .parse::<u32>()
            .ok()
            .filter(|n| (1..=136).contains(n))
            .map(Command::WalkLevel)
            .ok_or_else(|| "walklevel: level must be 1..136".into()),
        ["move", x, y] => Ok(Command::Move(point(x, y)?)),
        ["press", b, x, y] => Ok(Command::Press(button(b)?, point(x, y)?)),
        ["release", b, x, y] => Ok(Command::Release(button(b)?, point(x, y)?)),
        ["key", k] => vk_code(k)
            .filter(|&vk| play_key(vk).is_some())
            .map(Command::Key)
            .ok_or_else(|| format!("unknown key `{k}`")),
        ["rawkey", k] => vk_code(k)
            .filter(|&vk| play_key(vk).is_some())
            .map(Command::RawKey)
            .ok_or_else(|| format!("unknown key `{k}`")),
        ["query", ..] => crate::app::repro::query::parse_names(
            line.trim_start().strip_prefix("query").unwrap_or_default(),
        )
        .map(Command::Query),
        ["click", id] => crate::ui::ui_ids::parse_ref(id)
            .map(|id| Command::ClickUi(id.to_owned()))
            .ok_or_else(|| format!("`{line}`: click ui:ID")),
        [op @ ("interactunit" | "useunit"), t, classes] => {
            let text = format!("clickunit {t} {classes}");
            match input_script::parse_step(&text, (800, 600))? {
                Step::ClickUnit(sel) => Ok(if *op == "useunit" {
                    Command::UseUnit(sel)
                } else {
                    Command::InteractUnit(sel)
                }),
                _ => Err("expected a unit selector".into()),
            }
        }
        ["clickunit" | "rclickunit", ..] => {
            let size = crate::rules::camera::FrameSize::play();
            match input_script::parse_step(line, (size.width, size.height))? {
                Step::ClickUnit(sel) => Ok(Command::ClickUnit(sel)),
                _ => Err("expected a unit click".into()),
            }
        }
        ["ui"] => Ok(Command::Ui),
        ["text", t] => Ok(Command::Text((*t).to_owned())),
        ["state"] => Ok(Command::State),
        ["map"] => Ok(Command::Map),
        ["quit"] => Ok(Command::Quit),
        _ => Err(format!("unknown command `{line}`")),
    }
}

/// `state`: the server snapshot and the client's view, read between
/// two passes. `counts`: the passes and message counts of the caller.
pub fn state<C: Clock + Send + 'static>(
    world: &World,
    server: &Mutex<ThreadLink<Link<C>>>,
    counts: &Counts,
) -> Result<Value> {
    let snap = server
        .lock()
        .expect("server lock")
        .state_snapshot()
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let snap: Value = serde_json::from_str(&snap.to_json_line())?;
    let local = local_player(server);
    let w = &world.resource::<BridgeResource>().0.world().clone();
    let ui = world.non_send::<WorldViewUi>();
    // The drawn frame's camera anchor: the local player's position the
    // world view and the hover pick read (the walk prediction).
    let camera = world
        .get_resource::<crate::world_view::WorldViewState>()
        .and_then(|s| s.anchor)
        .map(|a| match a.player {
            crate::rules::camera::UnitPosition::Moving { x16, y16 } => {
                json!({"x16": x16, "y16": y16, "x": x16 >> 16, "y": y16 >> 16, "shake": [a.shake.0, a.shake.1]})
            }
            crate::rules::camera::UnitPosition::Static { sx, sy } => {
                json!({"x": sx, "y": sy, "shake": [a.shake.0, a.shake.1]})
            }
        });
    let o = ui.original.as_ref();
    let open: Vec<u8> = (0u8..0x30)
        .filter(|&i| o.is_some_and(|o| o.is_open(i)))
        .collect();
    let npc = o.and_then(|o| o.npc_menu()).map(|m| {
        let rows: Vec<Value> = m
            .rows
            .iter()
            .enumerate()
            .map(|(k, r)| {
                let at = o.and_then(|o| o.npc_menu_row_point(k));
                json!({
                    "string": r.string,
                    "kind": r.kind.map(|k| format!("{k:?}")),
                    "at": at.map(|p| [p.x, p.y]),
                })
            })
            .collect();
        json!({"guid": m.guid, "class": m.class, "talking": m.talking, "rows": rows})
    });
    let topics = o.and_then(|o| o.npc_topics()).map(|t| {
        let pts: Vec<Value> = (0..t.len())
            .map(|k| json!(o.and_then(|o| o.npc_topic_point(k)).map(|p| [p.x, p.y])))
            .collect();
        json!({
            "text": t.iter().map(|s| String::from_utf16_lossy(s)).collect::<Vec<_>>(),
            "at": pts,
            "cancel": o.and_then(|o| o.npc_topic_cancel_point()).map(|p| [p.x, p.y]),
        })
    });
    let dialog = o.and_then(|o| o.npc_dialog_lines()).map(|l| l.len());
    let waypoint: Vec<Value> = o
        .map(|o| o.waypoint_rows())
        .unwrap_or_default()
        .iter()
        .map(|r| json!({"level": r.level, "known": r.known, "current": r.current}))
        .collect();
    let belt: Vec<Value> = items::belt(w)
        .values()
        .map(|i| json!({"slot": i.x, "code": code_str(i.code)}))
        .collect();
    // Each ground item with the text its Show Items label draws.
    let labels = o.map(|o| o.ground_label_texts(w)).unwrap_or_default();
    let ground: Vec<Value> = items::ground_items(w)
        .iter()
        .map(|i| {
            json!({"g": i.key.guid, "code": code_str(i.code),
                   "x": i.x, "y": i.y, "gold": i.gold,
                   "label": labels.get(&i.key.guid)})
        })
        .collect();
    // The local player's worn items: body location and code.
    let worn: Vec<Value> = items::local_items(w)
        .iter()
        .filter(|i| i.mode == items::mode::BODY)
        .map(|i| json!({"body": i.body, "code": code_str(i.code)}))
        .collect();
    // The item the client model holds on the local player's cursor.
    let cursor = items::cursor_item(w).map(|i| json!({"g": i.key.guid, "code": code_str(i.code)}));
    let me = w
        .local()
        .map(|p| json!({"g": p.key.guid, "pos": p.position.map(|(x, y)| [x, y]), "mode": p.mode}));
    // The mouse skills the model holds (the pick of the skill select
    // reaches them through S→C 0x23) and the hover the next press sees.
    let mouse_skills = w.local().and_then(|p| p.skills.as_ref()).map(|l| {
        let id = |i: Option<usize>| i.and_then(|i| l.entries.get(i)).map(|e| e.skill);
        json!({"left": id(l.left), "right": id(l.right)})
    });
    let tiles: Vec<Value> = w
        .units
        .iter()
        .filter(|(k, _)| k.unit_type == crate::bridge::world::TILE)
        .map(
            |(k, u)| json!({"g": k.guid, "class": u.class, "pos": u.position.map(|(x, y)| [x, y])}),
        )
        .collect();
    // The objects near the local player (12 cells): class, mode, cell.
    let near = |pos: Option<(u16, u16)>| {
        let me = w.local().and_then(|p| p.position);
        matches!((pos, me), (Some((x, y)), Some((mx, my)))
            if x.abs_diff(mx) <= 12 && y.abs_diff(my) <= 12)
    };
    let objects: Vec<Value> = w
        .units
        .iter()
        .filter(|(k, u)| k.unit_type == crate::bridge::world::OBJECT && near(u.position))
        .map(|(k, u)| json!({"g": k.guid, "class": u.class, "mode": u.mode, "pos": u.position.map(|(x, y)| [x, y])}))
        .collect();
    // The superunique monsters the client knows (type flag 0x02): the
    // `superuniques` row they were spawned as, so a check can follow one
    // boss among the minions and ordinary monsters of its class. A unit
    // whose GUID has not arrived yet (GUID 0, mode 0) is a placeholder, not
    // a dead boss.
    let superuniques: Vec<Value> = w
        .units
        .iter()
        .filter(|(k, _)| k.unit_type == crate::bridge::world::MONSTER && k.guid != 0)
        .filter_map(|(k, u)| match &u.kind {
            crate::bridge::world::KindData::Monster(m) if m.flags & 0x02 != 0 => Some(json!({
                "g": k.guid,
                "row": m.hc_idx,
                "class": u.class,
                "mode": u.mode,
                "pos": u.position.map(|(x, y)| [x, y]),
            })),
            _ => None,
        })
        .collect();
    let view_state = world.get_resource::<crate::world_view::WorldViewState>();
    let hover = view_state
        .and_then(|s| s.prev_hover)
        .map(|k| json!([k.unit_type, k.guid]));
    // How fast each unit's animation is drawn (`world_view::anim_clock`).
    let anim = view_state.map(|s| s.drawn_anim.json());
    // The world view's last built frame: the server tick it shows, its
    // draw items and map tiles, and how many frames it skipped.
    let view = view_state.map(|s| {
        let tiles = s
            .last_tags
            .iter()
            .filter(|t| matches!(t, crate::scene::ItemTag::Tile { .. }))
            .count();
        json!({
            "tick": s.last.as_ref().map(|l| l.server_tick),
            "items": s.last.as_ref().map(|l| l.items),
            "tiles": tiles,
            "skipped": s.skipped_frames,
        })
    });
    // The numbers the HUD draws from: the local player's stat totals
    // in the client model, life, mana and stamina as whole points.
    let hud = w.local().map(|p| {
        let stat = |id: u16| w.total(p.key, id, 0);
        json!({
            "life": stat(6) >> 8, "life_max": stat(7) >> 8,
            "mana": stat(8) >> 8, "mana_max": stat(9) >> 8,
            "stamina": stat(10) >> 8, "stamina_max": stat(11) >> 8,
            "level": stat(12), "exp": stat(13), "gold": stat(14),
        })
    });
    Ok(json!({
        "k": "state",
        "format": FORMAT,
        "passes": counts.passes,
        "server_ticks": w.server_ticks,
        "local": local,
        "client": {
            "player": me,
            "camera": camera,
            "act": w.act.as_ref().map(|a| format!("{a:?}")),
            "open": open,
            "npc_menu": npc,
            "topics": topics,
            "dialog_lines": dialog,
            "waypoint_rows": waypoint,
            "belt": belt,
            "ground": ground,
            "mouse_skills": mouse_skills,
            "weapon_set": w.weapon_set,
            "worn": worn,
            "cursor": cursor,
            "received": counts.received.lock().unwrap_or_else(|e| e.into_inner()).iter()
                .map(|(k, v)| (format!("{k:02X}"), *v)).collect::<std::collections::BTreeMap<_, _>>(),
            "tiles": tiles,
            "objects": objects,
            "superuniques": superuniques,
            "hover": hover,
            "anim": anim,
            "view": view,
            "exit_requested": w.exit_requested,
            "sent": counts.sent.lock().unwrap_or_else(|e| e.into_inner()).iter()
                .map(|(k, v)| (format!("{k:02X}"), *v)).collect::<std::collections::BTreeMap<_, _>>(),
        },
        "hud": hud,
        "snap": snap,
    }))
}

/// `map`: the allocated levels of the local player's act, with their
/// rooms (sub-tile rects), near lists and warp links.
pub fn map<C: Clock + Send + 'static>(server: &Mutex<ThreadLink<Link<C>>>) -> Result<Value> {
    let v = server
        .lock()
        .expect("server lock")
        .with(|l| {
            let sim = &mut l.host_mut().game;
            let (p, _) = single_player::local_player(sim)?;
            let act = sim.events.action.sys.units.get(p)?.act;
            let g = &mut sim.events;
            g.action
                .hooks()
                .drlg
                .with_act(act, &mut sim.game.lists, |d, svc| {
                    let mut levels = Vec::new();
                    for id in 1..=136u32 {
                        let Some(li) = d.find_level(id) else { continue };
                        let lv = d.level(li);
                        let rooms: Vec<Value> = d
                            .level_rooms(li)
                            .into_iter()
                            .map(|r| {
                                let room = d.room(r);
                                let t = room.rect;
                                let grid = room.active().map(|a| {
                                    let c = &a.collision;
                                    let cells: String = c.masks.iter().map(|&m| grid_char(m)).collect();
                                    json!({"rect": [c.rect.x, c.rect.y, c.rect.w, c.rect.h], "cells": cells})
                                });
                                json!({
                                    "grid": grid,
                                    "presets": svc.types.preset_units(d, r).into_iter().map(|u| json!({
                                        "ut": u.unit_type, "cl": u.class, "g": 0, "m": 1,
                                        "x": t.x * 5 + u.x, "y": t.y * 5 + u.y,
                                    })).collect::<Vec<_>>(),
                                    "id": r.0,
                                    "rect": [t.x * 5, t.y * 5, t.w * 5, t.h * 5],
                                    "near": room.near().map(|n| n.iter().map(|x| x.0).collect::<Vec<_>>()),
                                    "warps": room.warp_links.iter().map(|w| json!({
                                        "to": w.target.0, "on": w.enabled, "row": w.lvlwarp_row,
                                    })).collect::<Vec<_>>(),
                                    "level": id,
                                })
                            })
                            .collect();
                        let r = lv.rect;
                        levels.push(json!({
                            "id": id,
                            "type": lv.drlg_type,
                            "rect": [r.x * 5, r.y * 5, r.w * 5, r.h * 5],
                            "warp_centres": lv.warp_centres,
                            "vis": d.vis_array(svc.data, id).ok(),
                            "rooms": rooms,
                        }));
                    }
                    json!({"k": "map", "act": act, "levels": levels})
                })
        })
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    v.context("no local player or act")
}

/// One sub-tile of a `map` grid: `#` blocks a walking player
/// (the player move mask 0x1C09 without the door bit), `D` a door, `.`
/// free.
fn grid_char(mask: u16) -> char {
    use d2_sim::drlg::collision::bits;
    if mask & bits::DOOR != 0 {
        'D'
    } else if mask & (0x1C09 & !bits::DOOR) != 0 {
        '#'
    } else {
        '.'
    }
}

/// An item code as text (`"hp1"`), empty when absent.
fn code_str(code: Option<[u8; 4]>) -> String {
    code.map(|c| String::from_utf8_lossy(&c).trim_end().to_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_commands_parse() {
        assert_eq!(parse_command("step-ticks 10"), Ok(Command::StepTicks(10)));
        assert_eq!(parse_command("pause"), Ok(Command::Pause));
        assert_eq!(parse_command("resume"), Ok(Command::Resume));
        assert_eq!(
            parse_command("shot E:/work/artifacts/a b.png"),
            Ok(Command::Shot("E:/work/artifacts/a b.png".into()))
        );
        for bad in [
            "step-ticks",
            "step-ticks -2",
            "step-ticks x",
            "shot",
            "shot  ",
            "pause 1",
        ] {
            assert!(parse_command(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn message_counts_go_by_first_byte() {
        let log = MessageLog::default();
        count(&log, &[0x03, 1, 2]);
        count(&log, &[0x03]);
        count(&log, &[]);
        count(&log, &[0x0C, 0]);
        let got: Vec<(u8, u32)> = log.lock().unwrap().iter().map(|(k, v)| (*k, *v)).collect();
        assert_eq!(got, vec![(0x03, 2), (0x0C, 1)]);
    }
}
