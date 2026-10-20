//! A killed boss stays dead, on the user's install (`d2-client
//! state-dump`, task q-fix-boss-damage): Andariel, hit by a Fire Bolt at
//! 1 hp, goes to DT (mode 0) on the hit and to DD (mode 12) when DT's
//! event function runs, and never leaves the death modes. Before the fix
//! the DT start kept her pending AI think (no think function yet), which
//! fired during the death animation and set an attack mode: the
//! "monsters stand back up, hp 0" report.

use d2_client::app::single_player::Character;
use d2_client::app::state_dump::{self, DumpArgs, DumpGame, RunInfo};
use d2_server::adapters::character::LoadContext;

use crate::app_support;

const ANDARIEL: i64 = 156;

fn run_with(ticks: u32, pokes: &[&str]) -> Vec<u8> {
    let args = DumpArgs {
        tick_rate: d2_sim::tick::TickRate::BASE,
        save: None,
        seed: Some(1),
        difficulty: 0,
        ticks,
        every: 1,
        out: "unused".into(),
        game_dir: None,
        date: Some("2026-10-09".into()),
        pokes: pokes
            .iter()
            .map(|p| d2_client::app::poke::parse_poke_arg(p).unwrap())
            .collect(),
        input: None,
        sends: Vec::new(),
        no_own_c2s: Vec::new(),
        send_clock: None,
        player_seed: None,
        gfx_ready: None,
        packets: None,
        rng: None,
        save_out: None,
        client_out: None,
        client_grid: false,
        early: Vec::new(),
        no_ping: false,
    };
    let mut game = DumpGame::resolve(&args, app_support::game_data(), None).unwrap();
    let save = d2_formats::d2s::D2s::new_stub(b"BossSor", 1, 0x20, 0).unwrap();
    game.character = Character::Save(
        Box::new(save),
        LoadContext {
            difficulty: 0,
            map_seed_applies: false,
        },
    );
    let info = RunInfo {
        tool: "d2-client state-dump test".into(),
        date: "2026-10-09".into(),
        command: "test".into(),
        save: None,
    };
    let mut out = Vec::new();
    state_dump::dump(game, ticks, 1, &info, &mut out).unwrap();
    out
}

/// (frame, mode, hp) at each change of the first monster of `class`.
fn track(dump: &[u8], class: i64) -> Vec<(u64, i64, i64)> {
    let mut out: Vec<(u64, i64, i64)> = Vec::new();
    for line in std::str::from_utf8(dump).unwrap().lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        if v["k"] != "snap" {
            continue;
        }
        let Some(u) = v["units"]
            .as_array()
            .unwrap()
            .iter()
            .find(|u| u["ut"] == 1 && u["cl"] == class)
        else {
            continue;
        };
        let s = (u["m"].as_i64().unwrap(), u["hp"].as_i64().unwrap());
        if out.last().is_none_or(|&(_, m, h)| (m, h) != s) {
            out.push((v["f"].as_u64().unwrap(), s.0, s.1));
        }
    }
    out
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn andariel_killed_by_a_fire_bolt_goes_to_mode_12_and_stays_dead() {
    // traces/playthrough/act1.play `andariel-killed`: Catacombs Level 4,
    // the walk next to her, her life at 1 (256), one Fire Bolt.
    let pokes = [
        "5 warp 37",
        "20 goto unit 156",
        "30 pos @1:156 @x+2 @y",
        "30 stat @1:156 6 0 256",
        "40 missile 58 @x @y @x+2 @y skill 36 1",
    ];
    let t = track(&run_with(200, &pokes), ANDARIEL);
    let hit = t
        .iter()
        .position(|&(_, m, h)| m == 0 && h == 0)
        .unwrap_or_else(|| panic!("never in DT with life 0: {t:?}"));
    let after = &t[hit..];
    assert_eq!(
        after.iter().map(|&(_, m, h)| (m, h)).collect::<Vec<_>>(),
        [(0, 0), (12, 0)],
        "after the hit only DT then DD, life 0: {t:?}"
    );
    assert!(t[hit].0 <= 45, "the bolt kills at once: {t:?}");
}
