//! A saved hireling restored at the join, on the user's install, against
//! what the game does with the same save (a reference run,
//! 2026-10-09: equal in every compared field for 80
//! frames): the load allocates it before game entry populates the town,
//! so it takes GUID 1 and the first unit seed, with its monster init
//! (seed stepped 9 times by frame 2); the join follow places it at the
//! player and its AI walks from frame 3.

use d2_client::app::single_player::Character;
use d2_client::app::state_dump::{self, DumpArgs, DumpGame, RunInfo};
use d2_formats::d2s::{self, Body, D2s, Golem, Header, StatEntry, Stats};
use d2_server::adapters::character::LoadContext;

use crate::app_support;

/// ScnBar of the check: expansion Barbarian, level 30, hireling row `Id`
/// 0 (Rogue Scout, Fire - Normal), name index 21, seed 0x12345678,
/// 39,482 experience.
fn scn_bar() -> Character {
    let mut header = Header::default();
    header.set_name(b"ScnBar").unwrap();
    header.class = 4;
    header.level = 30;
    header.status = d2s::status::EXPANSION;
    header.towns = [0x80, 0, 0];
    header.hireling.seed = 0x1234_5678;
    header.hireling.name_index = 21;
    header.hireling.id = 0;
    header.hireling.experience = 39_482;
    let stats = [(0, 30), (1, 10), (2, 20), (3, 25), (12, 30)]
        .map(|(id, value)| StatEntry {
            id,
            layer: 0,
            value,
        })
        .to_vec();
    let save = D2s {
        header,
        body: Some(Body {
            stats: Stats::Bits(stats),
            skills: vec![0; 30],
            hireling_items: Some(Some(vec![])),
            golem: Some(Golem {
                flag: 0,
                item: None,
            }),
            ..Body::default()
        }),
    };
    Character::Save(
        Box::new(save),
        LoadContext {
            difficulty: 0,
            map_seed_applies: false,
        },
    )
}

fn dump(ticks: u32) -> String {
    let args = DumpArgs {
        tick_rate: d2_sim::tick::TickRate::BASE,
        save: None,
        seed: Some(1234),
        difficulty: 0,
        ticks,
        every: 1,
        out: "unused".into(),
        game_dir: None,
        date: Some("2026-10-09".into()),
        pokes: Vec::new(),
        sends: Vec::new(),
        no_own_c2s: Vec::new(),
        send_clock: None,
        player_seed: None,
        gfx_ready: None,
        input: None,
        packets: None,
        rng: None,
        save_out: None,
        client_out: None,
        client_grid: false,
        early: Vec::new(),
        no_ping: false,
    };
    let mut game = DumpGame::resolve(&args, app_support::game_data(), None).unwrap();
    game.character = scn_bar();
    let info = RunInfo {
        tool: "d2-client state-dump test".into(),
        date: "2026-10-09".into(),
        command: "test".into(),
        save: Some("ScnBar.d2s".into()),
    };
    let mut out = Vec::new();
    state_dump::dump(game, ticks, 1, &info, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

/// The unit record of (type, GUID) in the snapshot of frame `f`.
fn unit(text: &str, f: u32, ty: u32, guid: u32) -> Option<&str> {
    let snap = text
        .lines()
        .find(|l| l.starts_with(&format!(r#"{{"k":"snap","f":{f},"#)))?;
    let key = format!(r#"{{"ut":{ty},"g":{guid},"#);
    let at = snap.find(&key)?;
    let rest = &snap[at..];
    Some(&rest[..=rest.find('}')?])
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_saved_hireling_is_the_first_monster_and_follows_from_frame_3() {
    let text = dump(3);
    let merc = unit(&text, 2, 1, 1).expect("monster GUID 1 at frame 2");
    // The reference run (-seed 1234): class 271 at the player's sub-tile,
    // level 7 from 39,482 experience, life 81 · 256, seed stepped 9 times
    // from the allocation's {108806926, 666}.
    for want in [
        r#""cl":271,"m":1,"x":4873,"y":4228,"#,
        r#""s":[3077027668,710430533],"#,
        r#""hp":20736,"hpx":20736,"#,
        r#""str":40,"#,
        r#""dex":53,"#,
        r#""lvl":7}"#,
    ] {
        assert!(merc.contains(want), "{want} not in {merc}");
    }
    let player = unit(&text, 2, 0, 1).expect("player at frame 2");
    assert!(player.contains(r#""x":4873,"y":4228,"#), "{player}");
    // The town NPCs follow it: GUID 2 is the first town NPC (class 152).
    let next = unit(&text, 2, 1, 2).expect("monster GUID 2 at frame 2");
    assert!(
        next.contains(r#""cl":152,"m":1,"x":4867,"y":4276,"#),
        "{next}"
    );
    // Frame 3: the follow AI walks (mode 2) toward 4877, 4229.
    let merc = unit(&text, 3, 1, 1).expect("monster GUID 1 at frame 3");
    assert!(
        merc.contains(r#""m":2,"x":4873,"y":4228,"xf":32768,"yf":32768,"tx":4877,"ty":4229,"#),
        "{merc}"
    );
    assert!(merc.contains(r#""s":[3740604328,1024117406],"#), "{merc}");
}
