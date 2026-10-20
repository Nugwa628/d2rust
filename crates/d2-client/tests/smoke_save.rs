//! Save smoke tests (q-smoke-save): the real play path end to end, no
//! window. A character joins the synthetic single-player game through the
//! bridge and the in-process server, is played (levels, stat and skill
//! points, items in the inventory, stash and cube, waypoints, quests,
//! gold), leaves through the Esc menu's "Save and Exit Game" (C→S 0x69:
//! the server's leave writes the file, then the app exits), and the written
//! `.d2s` is read and joined again: every live value the save holds must
//! come back exactly, and saving the reloaded character must give the
//! same file. Then the same at Nightmare, a hardcore death and the
//! dead-hardcore load refusal, and the `.bak` of a second save.
//!
//! Everything here is D2RUST's own, unverified: the synthetic tables are
//! fixtures, and no outside byte layout is compared.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use bevy::prelude::*;
use d2_client::app::play::{add_client_data, add_game, add_preview, send_create_game_for};
use d2_client::app::save::{self, Live, SaveHandle, SharedLink};
use d2_client::app::single_player::{self, Character, Sim, DEFAULT_SEED};
use d2_client::app::ui::{add_original_ui, UiParts};
use d2_client::assets::path::MemorySource;
use d2_client::bridge::BridgeResource;
use d2_client::controls::Action;
use d2_client::ui::{ActionId, Point, PointerButton, UiEvent};
use d2_client::world_view::tile_assets::TileAssets;
use d2_client::world_view::WorldViewUi;
use d2_data::tables::{Record, Skills};
use d2_formats::d2s::{self, D2s, ReadOptions, StatSave};
use d2_server::adapters::character::LoadContext;
use d2_server::seams::Clock;
use d2_sim::items::inventory::{page, UnitKind};
use d2_sim::items::moves::InventoryOps;
use d2_sim::items::ItemRequest;
use d2_sim::skills::list::ListOwner;
use d2_sim::units::UnitId;
use d2_sim::wiring::economy::ItemSpawn;

use crate::app_support;

struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

type Server = SharedLink<StepClock>;

/// The save tables of the synthetic game: 32-bit stats (the synthetic
/// `itemstatcost` has no save columns), the synthetic item tables for the
/// item stream, every present hireling restorable.
struct Tables {
    items: d2_sim::items::ItemTables,
}

impl Tables {
    fn new() -> Self {
        Self {
            items: app_support::live().tables.item_tables().unwrap(),
        }
    }
}

impl d2s::SaveTables for Tables {
    fn stat_save(&self, id: u16) -> Option<StatSave> {
        (id < 400).then_some(StatSave {
            bits: 32,
            param: 0,
            signed: true,
        })
    }
    fn item_entry_len(&self, buf: &[u8]) -> Result<usize, String> {
        d2_sim::items::bitstream::read::read_save_entry(buf, &self.items)
            .map(|e| e.len)
            .map_err(|e| e.to_string())
    }
    fn hireling_restored(&self, h: &d2s::Hireling) -> bool {
        h.is_present()
    }
}

fn blank<T: Record>() -> T {
    T::decode(&vec![0u8; T::SIZE])
}

/// Skill rows 0..=`SKILLS` with ten class skills per class (ids
/// 1 + 10·class ..), so the save's class-list order exists.
const SKILLS: usize = 71;

fn skill_rows() -> Vec<Skills> {
    (0..SKILLS)
        .map(|i| {
            let mut s: Skills = blank();
            s.charclass = if i == 0 { 0xFF } else { ((i - 1) / 10) as _ };
            s.ingame = true;
            s.skpoints = d2_sim::skills::levels::NO_CALC;
            (s.reqskill1, s.reqskill2, s.reqskill3) = (0xFFFF, 0xFFFF, 0xFFFF);
            s.maxlvl = 20;
            s
        })
        .collect()
}

/// A DC6 of one direction with `frames` frames of 2 × 2 pixels
/// (`app_original_ui.rs`' fixture).
fn dc6(frames: u32) -> Vec<u8> {
    let rows = [2u8, 1, 2, 0x80, 2, 3, 4, 0x80];
    let mut d = Vec::new();
    for v in [6i32, 1, 0] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.extend_from_slice(&[0xEE; 4]);
    d.extend_from_slice(&1u32.to_le_bytes());
    d.extend_from_slice(&frames.to_le_bytes());
    let mut at = d.len() + 4 * frames as usize;
    let mut body = Vec::new();
    for _ in 0..frames {
        d.extend_from_slice(&(at as u32).to_le_bytes());
        for v in [0u32, 2, 2, 0, 0, 0, 0, rows.len() as u32] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        body.extend_from_slice(&rows);
        body.extend_from_slice(&[0xEE; 3]);
        at += 32 + rows.len() + 3;
    }
    d.extend(body);
    d
}

fn panel_files() -> MemorySource {
    let mut s = MemorySource::default();
    for (name, frames) in [
        ("panel\\invchar", 8),
        ("panel\\buysellbtn", 12),
        ("panel\\800borderframe", 10),
        ("panel\\800ctrlpnl7", 6),
        ("panel\\goldcoinbtn", 2),
        // The cursor cels, drawn every frame.
        ("cursor\\gaunt", 1),
        ("cursor\\grasp", 8),
        ("cursor\\ohand", 8),
        ("cursor\\orotate", 8),
        ("cursor\\ppress", 8),
        ("cursor\\protate", 8),
        ("cursor\\buysell", 10),
    ] {
        s.insert(&format!("data\\global\\ui\\{name}.dc6"), dc6(frames));
    }
    s
}

/// One running game: the app (bridge, world view, original UI with the
/// Esc menu, hardcore wiring), its server, the save handle.
struct Run {
    app: App,
    server: Server,
    saver: SaveHandle,
    ms: Arc<AtomicU32>,
}

impl Run {
    /// `play::run`'s wiring without the window: the save handle shares
    /// the link, the app joins `character` (`hardcore`: `play --new
    /// --hardcore`; a save's own status bit makes it hardcore too).
    fn start(character: &Character, path: &std::path::Path) -> Run {
        Run::start_hc(character, path, false)
    }

    fn start_hc(character: &Character, path: &std::path::Path, hardcore: bool) -> Run {
        let data = app_support::game_data();
        let ms = Arc::new(AtomicU32::new(1000));
        let mut base = save::base_save(character);
        let hardcore = hardcore || base.header.status & d2s::status::HARDCORE != 0;
        if hardcore {
            base.header.status |= d2s::status::HARDCORE;
        }
        let (mut link, _) = single_player::start_with(
            data.clone(),
            DEFAULT_SEED,
            character.clone(),
            StepClock(ms.clone()),
        )
        .unwrap();
        link.with(move |l| {
            let a = &mut l.host_mut().game.events.action;
            Arc::make_mut(&mut a.hooks().tables).skills.skills = skill_rows();
            a.hooks().x.hardcore = hardcore;
        })
        .unwrap();
        let (server, saver) =
            save::share(link, base, Arc::new(Tables::new()), None, path.into()).unwrap();
        let mut app = App::new();
        app.insert_resource(d2_client::bridge::mirror::ScriptedClock(ms.clone()));
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Image>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>();
        app.insert_resource(saver.clone());
        add_game(&mut app, Box::new(server.clone()), false).unwrap();
        send_create_game_for(&mut app, character).unwrap();
        let levels = single_player::client_level_rows(&data);
        add_client_data(
            &mut app,
            single_player::client_drlg_source(&data),
            levels.clone(),
        );
        add_preview(&mut app, levels, TileAssets::default());
        add_original_ui(
            &mut app,
            UiParts {
                source: Arc::new(panel_files()),
                inv_areas: None,
                expansion_installed: false,
                fonts: None,
                resist_penalties: None,
            },
        )
        .unwrap();
        d2_client::app::death::add_death(&mut app);
        d2_client::app::hardcore::add_hardcore(&mut app, hardcore);
        let mut run = Run {
            app,
            server,
            saver,
            ms,
        };
        run.step(10);
        assert!(run.player().is_some(), "the character joined");
        run
    }

    fn step(&mut self, n: usize) {
        for _ in 0..n {
            self.app.update();
            self.ms.fetch_add(40, Ordering::SeqCst);
        }
    }

    fn with<R: Send + 'static>(&self, f: impl FnOnce(&mut Sim) -> R + Send + 'static) -> R {
        self.server.with(|l| f(&mut l.host_mut().game)).unwrap()
    }

    fn player(&self) -> Option<(UnitId, u32)> {
        self.with(|s| single_player::local_player(s))
    }

    fn live(&self) -> Live {
        self.with(|s| save::read_live(s).unwrap())
    }

    fn log(&self) -> Vec<String> {
        self.with(|s| s.events.action.hooks().x.log.clone())
    }

    fn ui_event(&mut self, e: UiEvent) {
        self.app
            .world_mut()
            .non_send_mut::<WorldViewUi>()
            .queue
            .0
            .push(e);
        self.step(1);
    }

    /// Esc, then a click on "Save and Exit Game" (the game menu's second
    /// row): C→S 0x69, the server's leave
    /// writes the file before its 0x05, and
    /// the app stops on the server's answer.
    fn save_and_exit(mut self) {
        self.ui_event(UiEvent::Action(ActionId(Action::GameMenu.index() as u16)));
        let at = Point::new(400, 185 + 50 + 20);
        let b = PointerButton::Left;
        self.ui_event(UiEvent::Press { button: b, at });
        self.ui_event(UiEvent::Release { button: b, at });
        for _ in 0..10 {
            if self.app.should_exit().is_some() {
                break;
            }
            self.step(1);
        }
        assert!(
            self.app.should_exit().is_some(),
            "Save and Exit Game stops the app on the server's answer"
        );
        let (gone, faults) = self.with(|s| {
            let faults = s.session().map(|f| format!("{:?}", f.faults));
            (s.client_list().is_empty(), faults.unwrap_or_default())
        });
        assert!(gone, "the server's leave removed the client");
        assert!(!faults.contains("Save"), "the leave saved: {faults}");
        assert!(self.saver.path().exists(), "the leave wrote the file");
    }
}

/// What a session of play changes on the server.
struct Played {
    level: i32,
    gold: i32,
    bank: i32,
}

/// Plays a while on the server: levels, stats, unspent and spent points,
/// skill levels, a waypoint and a quest flag per difficulty, gold, items
/// in the inventory, the cube page and the stash.
fn play(run: &mut Run, seed: i32) -> Played {
    let played = Played {
        level: 7 + seed,
        gold: 1234 + seed,
        bank: 56789 + seed,
    };
    let (level, gold, bank) = (played.level, played.gold, played.bank);
    run.with(move |s| {
        let (p, _) = single_player::local_player(s).unwrap();
        let class = s.events.action.sys.units.get(p).unwrap().class as i32;
        let a = &mut s.events.action;
        {
            let sys = &mut a.sys;
            for (id, v) in [
                (0u16, 30 + seed),
                (1, 20 + seed),
                (2, 25),
                (3, 40),
                (4, 5),
                (5, 2),
                // Life, mana and stamina at their maxima: the regeneration
                // of the ticks between two reads changes nothing. The
                // maximum first: a changed maximum rescales the current
                // value, so the current value set
                // before it would not stay at the maximum.
                (7, 150 << 8),
                (6, 150 << 8),
                (9, 60 << 8),
                (8, 60 << 8),
                (11, 90 << 8),
                (10, 90 << 8),
                (12, level),
                (13, 20_000 + seed),
                (14, gold),
                (15, bank),
            ] {
                sys.stats.unit_set(&mut sys.hooks, p, id, v, 0);
            }
        }
        let rows = a.hooks().tables.skills.skills.clone();
        let list = a.hooks().skill_lists.entry(p).or_default();
        for k in 0..3 {
            list.set_base(
                &rows,
                ListOwner {
                    monster: false,
                    class,
                },
                1 + 10 * class + k,
                (k + 1 + seed) as u8,
            );
        }
        for d in 0..3usize {
            let w = a.hooks().waypoints.entry(p).or_default();
            w.get_mut(d as u8).set(3 + d as u32).unwrap();
        }
        let q = s.world.rest.quests.get_mut(&p).unwrap();
        for d in 0..3 {
            q.flags[d].set(1 + d as u8, 0);
            q.flags[d].set(2, 12);
        }
        // (5, 0): clear of the new character's charstats start items.
        give(s, p, *b"hp1 ", page::INVENTORY, (5, 0));
        give(s, p, *b"mp1 ", page::INVENTORY, (3, 1));
        give(s, p, *b"rin ", page::STASH, (0, 0));
        give(s, p, *b"amu ", page::STASH, (4, 2));
        give(s, p, *b"tsc ", page::CUBE, (1, 1));
        // A ring on the right ring finger (body location 6): no start kit wears one.
        equip(s, p, *b"rin ", 6);
    });
    run.step(4);
    played
}

/// Makes the install's item of `code` on the cursor of `owner` (an
/// inventory made for it when it has none, as the load does) and runs
/// `put` on the inventory desk with the item (a macro: the desk's hook
/// type is the host's own).
macro_rules! make_then {
    ($s:expr, $owner:expr, $code:expr, |$d:ident, $item:ident| $put:expr) => {{
        let (s, owner, code): (&mut Sim, UnitId, [u8; 4]) = ($s, $owner, $code);
        s.world
            .with_economy(&mut s.game, &mut s.events, |econ, parts| {
                // The install's items row of the code (the item index
                // runs weapons, armor, misc).
                let record = econ
                    .tables
                    .items
                    .iter()
                    .position(|r| r.code == code)
                    .expect("an items row of the code");
                let mut rq = ItemRequest {
                    item: record as i32,
                    ilvl: 1,
                    quality: 2,
                    format: 1,
                    ..ItemRequest::default()
                };
                let held = ItemSpawn {
                    room: None,
                    mode: 4,
                    init_flags: 1,
                };
                let $item = econ.create_item(&mut rq, false, held).expect("item");
                let inv = parts.inventory.as_deref_mut().expect("inventory model");
                let (ty, class, guid) = {
                    let u = econ.units.get(owner).unwrap();
                    (u.ty, u.class, u.guid)
                };
                if !inv.state.inventories.contains_key(&owner) {
                    let kind = if ty == d2_sim::units::UnitType::Monster {
                        UnitKind::Monster { class }
                    } else {
                        UnitKind::Player { class: class as u8 }
                    };
                    inv.state.add_inventory(owner, kind, guid);
                }
                let mut desk = inv.desk(econ);
                let $d = &mut desk;
                $d.sync_in();
                if let Some(i) = $d.state.items.get_mut(&$item) {
                    i.mode = d2_sim::items::inventory::mode::CURSOR;
                }
                $d.sync_out();
                $put
            })
    }};
}

/// Makes the install's item of `code` and stores it at `at` on `pg` of
/// `p`'s inventory (as the item moves leave a stored item).
fn give(s: &mut Sim, p: UnitId, code: [u8; 4], pg: u8, at: (i32, i32)) {
    let placed = make_then!(s, p, code, |d, item| {
        if let Some(i) = d.state.items.get_mut(&item) {
            i.page = pg;
        }
        d.sync_out();
        d.place(p, item, at, false, true)
    });
    assert!(placed, "item {code:?} stored on page {pg} at {at:?}");
}

/// Makes the install's item of `code` and equips it on `owner` at body
/// location `loc` from the cursor (the item moves' equip).
fn equip(s: &mut Sim, owner: UnitId, code: [u8; 4], loc: u8) {
    let worn = make_then!(s, owner, code, |d, item| {
        match (d.owner_of(owner), d.guid_of(item)) {
            (Some(o), g) => d.equip_from_cursor(o, g, loc, true).0,
            _ => false,
        }
    });
    assert!(worn, "item {code:?} worn at body location {loc}");
}

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("smoke-save-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn read(path: &std::path::Path, difficulty: u8) -> Result<D2s, d2s::D2sError> {
    let bytes = std::fs::read(path).unwrap();
    let opts = ReadOptions {
        expansion: true,
        game: Some(d2s::GameContext {
            client_name: single_player::save_name(&bytes).to_vec(),
            expansion: true,
            hardcore: d2_client::app::hardcore::save_is_hardcore(&bytes),
            difficulty,
        }),
    };
    d2s::read(&bytes, &opts, &Tables::new())
}

fn loaded(save: D2s, difficulty: u8) -> Character {
    Character::Save(
        Box::new(save),
        LoadContext {
            difficulty,
            map_seed_applies: false,
        },
    )
}

/// `live` as a load leaves it: every item's 0x2000 (instore) cleared, in
/// the player, hireling and corpse lists alike
/// ("a file's 0x2000 never survives a load"). The
/// flags are the 32 bits after `JM`; 0x2000 is bit 5 of byte 3.
fn loaded_live(mut live: Live) -> Live {
    let clear = |items: &mut Vec<d2s::ItemEntry>| {
        for e in items {
            e.bytes[3] &= !0x20;
        }
    };
    if let Some(items) = &mut live.extra.items {
        clear(items);
    }
    if let Some(items) = &mut live.gaps.hireling_items {
        clear(items);
    }
    for c in live.extra.corpses.iter_mut().flatten() {
        clear(&mut c.items);
    }
    live
}

/// The save without its time stamp and the checksum over it (the second
/// save is a later one).
fn timeless(mut s: D2s) -> D2s {
    s.header.save_time = 0;
    s.header.checksum = 0;
    s
}

/// Plays `class`, saves through the Esc menu, loads the file, and checks
/// the round trip: the live state after the load is the state before, and
/// a save of the reloaded character is the same file.
fn round_trip(class: &str, name: &str, difficulty: u8, seed: i32) {
    let dir = temp(name);
    let file = dir.join(format!("{name}.d2s"));
    let character = single_player::new_character(class, name)
        .unwrap()
        .with_difficulty(difficulty);
    let mut run = Run::start(&character, &file);
    // The new character's start items (the install's charstats kit; no
    // start cube, q-fix-real-start-cube) are what the join leaves.
    let kit = run.live().extra.items.as_ref().map_or(0, Vec::len);
    let played = play(&mut run, seed);
    let before = run.live();
    assert!(
        // The six of `play` on top of the kit.
        before
            .extra
            .items
            .as_ref()
            .is_some_and(|i| i.len() == kit + 6),
        "{name}: {kit} + 6 items before the save: {:?}",
        before.extra.items
    );
    run.save_and_exit();

    let first = read(&file, difficulty).unwrap();
    let h = &first.header;
    assert_eq!(h.name_bytes(), name.as_bytes());
    assert_eq!(i32::from(h.level), played.level);
    let stat = |s: &D2s, id: u16| {
        s.body
            .as_ref()
            .unwrap()
            .stats
            .entries()
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.value)
    };
    assert_eq!(stat(&first, 14), Some(played.gold));
    assert_eq!(stat(&first, 15), Some(played.bank));

    let file2 = dir.join(format!("{name}-again.d2s"));
    let again = Run::start(&loaded(first.clone(), difficulty), &file2);
    let log = again.log();
    // The loader's known unapplied steps (`ActionCharacter`) are listed
    // in the log; a failed load or a dropped item is a break.
    let broken: Vec<_> = log
        .iter()
        .filter(|l| l.contains("load failed") || l.contains("items:") || l.contains("quests "))
        .collect();
    assert!(broken.is_empty(), "{name}: the load reported: {broken:?}");
    let after = again.live();
    assert_eq!(
        after,
        loaded_live(before),
        "{name}: the live state after the load"
    );
    again.save_and_exit();
    let second = read(&file2, difficulty).unwrap();
    let mut first = timeless(first);
    if let Some(b) = &mut first.body {
        for e in &mut b.items {
            e.bytes[3] &= !0x20;
        }
    }
    assert_eq!(timeless(second), first, "{name}: the saved file");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn every_class_round_trips_through_save_and_exit() {
    for (i, (class, name)) in [
        ("amazon", "Ama"),
        ("sorceress", "Sorc"),
        ("necromancer", "Necro"),
        ("paladin", "Pala"),
        ("barbarian", "Barb"),
        ("druid", "Dru"),
        ("assassin", "Sin"),
    ]
    .into_iter()
    .enumerate()
    {
        round_trip(class, name, 0, i as i32);
    }
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_nightmare_character_round_trips() {
    round_trip("barbarian", "NmBarb", 1, 3);
    round_trip("sorceress", "NmSorc", 1, 5);
}

/// The mercenary and its gear: a character whose save holds a hireling
/// (the restore at the join makes it), the hammer put in its hand, saved
/// through the Esc menu; the reload restores the same hireling block and
/// the same item on it.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_mercenary_with_gear_round_trips() {
    let dir = temp("merc");
    let file = dir.join("Merc.d2s");
    let mut base = save::base_save(&single_player::new_character("paladin", "Merc").unwrap());
    base.header.hireling = d2s::Hireling {
        flags: 0,
        seed: 0x1234,
        name_index: 2,
        id: 1,
        experience: 5000,
        rest: [0; 16],
    };
    let mut run = Run::start(&loaded(base, 0), &file);
    run.step(10);
    let merc = run.with(|s| {
        let (p, _) = single_player::local_player(s).unwrap();
        s.world.hireling_unit(&s.game, p)
    });
    let merc = merc.expect("the save's hireling was restored at the join");
    play(&mut run, 1);
    run.with(move |s| equip(s, merc, *b"cap ", 1));
    run.step(2);
    let before = run.live();
    assert_eq!(
        before.gaps.hireling_items.as_ref().map(Vec::len),
        Some(1),
        "the hammer on the mercenary"
    );
    run.save_and_exit();
    let first = read(&file, 0).unwrap();
    assert_eq!(first.header.hireling.seed, 0x1234);
    let again = Run::start(&loaded(first, 0), &dir.join("Merc-again.d2s"));
    let broken: Vec<_> = again
        .log()
        .into_iter()
        // "hireling rule 8" (refresh, life := max after the items) is a
        // known unapplied step.
        .filter(|l| l.contains("hireling items") || l.contains("items:") || l.contains("failed"))
        .collect();
    assert!(broken.is_empty(), "the load reported: {broken:?}");
    assert_eq!(again.live(), loaded_live(before));
}

/// Hardcore: played, then killed; the death saves the character with the
/// dead bit (and the corpse), Esc leaves, the save on the way out keeps
/// it dead, and the load refuses the file.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_dead_hardcore_character_is_saved_dead_and_refused() {
    let dir = temp("hc");
    let file = dir.join("Hc.d2s");
    let character = single_player::new_character("necromancer", "Hc").unwrap();
    let mut run = Run::start_hc(&character, &file, true);
    play(&mut run, 2);
    // A first save while alive (the Esc menu would close the game: the
    // handle saves as the death screen does).
    run.saver.save().unwrap();
    let alive = std::fs::read(&file).unwrap();
    assert!(read(&file, 0).is_ok(), "a living hardcore character loads");
    run.with(|s| {
        let (p, _) = single_player::local_player(s).unwrap();
        s.events.action.start_death(&mut s.game, p);
    });
    run.step(4);
    run.with(|s| {
        let (p, _) = single_player::local_player(s).unwrap();
        let sys = &mut s.events.action.sys;
        let mut u = d2_sim::units::hooks::Sim {
            game: &mut s.game,
            units: &mut sys.units,
            stats: &mut sys.stats,
            data: &sys.data,
        };
        d2_sim::units::modes::player_event1(&mut u, &mut sys.hooks, p).unwrap();
    });
    run.step(3);
    let dead = read(&file, 0);
    assert!(
        matches!(&dead, Err(d2s::D2sError::Load { code: 10, .. })),
        "the death's save is refused: {dead:?}"
    );
    // The previous save is the `.bak`.
    let mut bak = file.clone().into_os_string();
    bak.push(".bak");
    assert!(std::path::Path::new(&bak).exists());
    // Esc on the dead hardcore player leaves; the exit save keeps it dead.
    run.app
        .world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Escape);
    run.step(4);
    assert!(run.app.should_exit().is_some(), "the game closes");
    run.saver.save().unwrap();
    let opts = ReadOptions {
        expansion: true,
        game: None,
    };
    let bytes = std::fs::read(&file).unwrap();
    let raw = d2s::read(&bytes, &opts, &Tables::new()).unwrap();
    let both = d2s::status::HARDCORE | d2s::status::DEAD;
    assert_eq!(raw.header.status & both, both);
    assert_eq!(raw.header.name_bytes(), b"Hc");
    assert!(matches!(
        read(&file, 0),
        Err(d2s::D2sError::Load { code: 10, .. })
    ));
    assert_ne!(bytes, alive);
}

/// `stitch-save`'s `.bak`: every save first copies the file it replaces;
/// no temporary file is left; the `.bak` loads as the earlier character.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn each_save_keeps_the_previous_file_as_bak() {
    let dir = temp("bak");
    let file = dir.join("Bak.d2s");
    let character = single_player::new_character("druid", "Bak").unwrap();
    let mut run = Run::start(&character, &file);
    run.saver.save().unwrap();
    let first = std::fs::read(&file).unwrap();
    let mut bak = file.clone().into_os_string();
    bak.push(".bak");
    let bak = std::path::PathBuf::from(bak);
    assert!(!bak.exists(), "no .bak before a second save");
    play(&mut run, 4);
    run.save_and_exit();
    assert_eq!(std::fs::read(&bak).unwrap(), first);
    let second = std::fs::read(&file).unwrap();
    assert_ne!(second, first);
    let names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert!(
        !names.iter().any(|n| n.ends_with(".tmp")),
        "no temporary file left: {names:?}"
    );
    let earlier = read(&bak, 0).unwrap();
    assert_eq!(earlier.header.level, 1);
    // A third save rotates: the .bak is now the second file.
    let again = Run::start(&loaded(read(&file, 0).unwrap(), 0), &file);
    again.save_and_exit();
    assert_eq!(std::fs::read(&bak).unwrap(), second);
}

/// Softcore death: the corpse takes the worn items, Esc respawns the
/// player, Save and Exit writes the corpse section; the reload makes the
/// corpse again with the items in it, so the
/// next save keeps it.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_corpse_with_its_items_survives_save_and_reload() {
    let dir = temp("corpse");
    let file = dir.join("Corpse.d2s");
    let character = single_player::new_character("sorceress", "Corpse").unwrap();
    let mut run = Run::start(&character, &file);
    play(&mut run, 0);
    // What the corpse takes: every worn item (the start kit's and the
    // ring `play` wears); the item's parent field (bits 58-60 of the
    // entry) is 1 for a worn one.
    let worn = run
        .live()
        .extra
        .items
        .unwrap()
        .iter()
        .filter(|e| (e.bytes[7] >> 2) & 7 == 1)
        .count();
    assert!(worn >= 2, "the kit's weapon and the ring are worn: {worn}");
    run.with(|s| {
        let (p, _) = single_player::local_player(s).unwrap();
        s.events.action.start_death(&mut s.game, p);
    });
    run.step(4);
    run.with(|s| {
        let (p, _) = single_player::local_player(s).unwrap();
        let sys = &mut s.events.action.sys;
        let mut u = d2_sim::units::hooks::Sim {
            game: &mut s.game,
            units: &mut sys.units,
            stats: &mut sys.stats,
            data: &sys.data,
        };
        d2_sim::units::modes::player_event1(&mut u, &mut sys.hooks, p).unwrap();
    });
    run.step(3);
    run.app
        .world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Escape);
    run.step(6);
    run.app
        .world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::Escape);
    run.step(2);
    let before = run.live();
    let corpses = before.extra.corpses.clone().unwrap();
    assert_eq!(corpses.len(), 1, "one corpse with items");
    assert_eq!(
        corpses[0].items.len(),
        worn,
        "the worn items are on the corpse"
    );
    assert_eq!(before.status_set, 0x08, "the death starts set the dead bit");
    run.save_and_exit();
    let first = read(&file, 0).unwrap();
    assert_eq!(first.body.as_ref().unwrap().corpses.len(), 1);
    // a softcore character that died and respawned saves 0x0028
    // (expansion | dead), the save's status word.
    assert_eq!(first.header.status, 0x0028, "status after a softcore death");
    let file2 = dir.join("Corpse-again.d2s");
    let again = Run::start(&loaded(first.clone(), 0), &file2);
    let broken: Vec<_> = again
        .log()
        .into_iter()
        .filter(|l| l.contains("corpse") || l.contains("items:"))
        .collect();
    assert!(broken.is_empty(), "the load reported: {broken:?}");
    let after = again.live();
    assert_eq!(after.extra.corpses, loaded_live(before).extra.corpses);
    again.save_and_exit();
    let second = read(&file2, 0).unwrap();
    // No code clears the bit: the reloaded character keeps it.
    assert_eq!(second.header.status, 0x0028);
    let mut want = first.body.unwrap().corpses;
    for c in &mut want {
        for e in &mut c.items {
            e.bytes[3] &= !0x20;
        }
    }
    assert_eq!(
        second.body.unwrap().corpses,
        want,
        "the corpse section of the next save"
    );
}

/// The app stops without Save and Exit (window close, `--frames`) and
/// with no `WorldViewState` (q-fix-play-exit-resource): `play::after_run`
/// still leaves through the server first, so the server's leave writes
/// the file, and nothing panics.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_window_close_leaves_through_the_server_first() {
    let dir = temp("close");
    let file = dir.join("Close.d2s");
    let character = single_player::new_character("sorceress", "Close").unwrap();
    let mut run = Run::start(&character, &file);
    assert!(!file.exists());
    let world = run.app.world_mut();
    assert!(world
        .remove_resource::<d2_client::world_view::WorldViewState>()
        .is_some());
    assert!(d2_client::app::play::after_run(world).unwrap());
    assert!(file.exists(), "the server's leave wrote the file");
    assert!(run.with(|s| s.client_list().is_empty()), "the client left");
    // A second call has nobody in game to leave.
    assert!(d2_client::app::play::after_run(run.app.world_mut()).unwrap());
    let faults = run.with(|s| {
        s.session()
            .map(|f| format!("{:?}", f.faults))
            .unwrap_or_default()
    });
    assert!(!faults.contains("Save"), "{faults}");
}

/// A hot key bound in play (C→S 0x51) is saved from the client slot
/// (code with the left
/// flag, item index), and the reload puts it back in the client slot;
/// unbound slots save as `FF FF 00 00`.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_hotkey_bound_in_play_round_trips() {
    let dir = temp("hotkey");
    let file = dir.join("Hotkey.d2s");
    let character = single_player::new_character("sorceress", "Hotkey").unwrap();
    let mut run = Run::start(&character, &file);
    // Slot 3: the first skill of the player's list (a native one), left
    // hand, no item (GUID −1).
    let skill = run.with(|s| {
        let (p, _) = single_player::local_player(s).unwrap();
        let l = &s.events.action.hooks().skill_lists[&p];
        l.entries.iter().find(|e| e.owner == -1).unwrap().skill
    });
    let code = skill as u16 | 0x8000;
    let mut bind = vec![0x51];
    bind.extend_from_slice(&code.to_le_bytes());
    bind.extend_from_slice(&[3, 0]);
    bind.extend_from_slice(&u32::MAX.to_le_bytes());
    run.app
        .world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .send_bytes(&bind)
        .unwrap();
    run.step(3);
    let key = run.with(|s| s.hotkeys(s.client_list()[0])[3]);
    assert_eq!(
        (key.skill, key.left),
        (skill as i16, true),
        "0x51 stored the slot"
    );
    run.save_and_exit();
    let saved = read(&file, 0).unwrap();
    assert_eq!(
        saved.header.hotkeys[3],
        d2s::Slot {
            code: 0x8000,
            item: 0
        }
    );
    for (i, s) in saved.header.hotkeys.iter().enumerate() {
        if i != 3 {
            assert_eq!(*s, d2s::Slot::NONE, "slot {i} unbound");
        }
    }
    let again = Run::start(&loaded(saved, 0), &dir.join("Hotkey-again.d2s"));
    let key = again.with(|s| s.hotkeys(s.client_list()[0])[3]);
    assert_eq!(
        (key.skill, key.left),
        (0, true),
        "the load restored the slot"
    );
    let unbound = again.with(|s| s.hotkeys(s.client_list()[0])[0]);
    assert_eq!(unbound.skill, -1);
}

/// The NPC fields: A (first talk) and B (introduced) are saved from the
/// player's NPC record and read back. Kashya (class 150, bit 3) heard in
/// Normal is A = `08 00 …`.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn npc_fields_round_trip() {
    let dir = temp("npcs");
    let file = dir.join("Npcs.d2s");
    let character = single_player::new_character("sorceress", "Npcs").unwrap();
    let run = Run::start(&character, &file);
    run.with(|s| {
        let (p, _) = single_player::local_player(s).unwrap();
        let q = s.world.rest.quests.get_mut(&p).unwrap();
        q.hear(0, 150);
        // Akara (148, bit 2) introduced in Normal; Warriv (155, bit 4)
        // heard in Hell.
        q.intro[0].insert(148);
        q.hear(2, 155);
    });
    run.save_and_exit();
    let saved = read(&file, 0).unwrap();
    let npcs = &saved.body.as_ref().unwrap().npcs;
    assert_eq!(npcs.a[0], [0x08, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(npcs.a[2], [0x10, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(npcs.b[0], [0x04, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!((npcs.a[1], npcs.b[1], npcs.b[2]), ([0; 8], [0; 8], [0; 8]));
    let again = Run::start(&loaded(saved, 0), &dir.join("Npcs-again.d2s"));
    let (heard, intro) = again.with(|s| {
        let (p, _) = single_player::local_player(s).unwrap();
        let q = &s.world.rest.quests[&p];
        (
            [q.heard(0, 150), q.heard(0, 148), q.heard(2, 155)],
            q.intro[0].clone(),
        )
    });
    assert_eq!(heard, [true, false, true]);
    assert_eq!(intro, std::collections::BTreeSet::from([148]));
    let log = again.log();
    assert!(
        !log.iter().any(|l| l.contains("npc fields")),
        "the load applied the NPC fields: {log:?}"
    );
}
