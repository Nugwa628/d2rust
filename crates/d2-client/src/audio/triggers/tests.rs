//! Synthetic test vectors of the sound triggers against a fake of the
//! sound layer (a log of its calls + scripted rolls).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use d2_data::tables::{Monsounds, Record};

use super::events::{player_event, server_event, EventExtra, Followup};
use super::modes::{convert_mode, impact, mode_set, mode_sound};
use super::movement::{
    circular_distance, flee, footstep, footstep_called, footstep_material, init_voice, neutral,
    Floor,
};
use super::npc::{
    dialog_close, dialog_line, greet, interact_greeting, set_npc_speech_option, DialogState,
    GreetMode, GreetingRecords, NpcGreeting,
};
use super::objects::object_mode;
use super::skills::{
    charge_start, item_drop, jab_do, missile_prog_fn29, missile_prog_fn47, missile_prog_fn51,
    missile_prog_fn9, skill_start, ItemRows, ItemSounds, SkillStart,
};
use super::tables::{
    NpcGreetings, NpcSpeech, ObjectSounds, TableError, NPC_GREETINGS_TSV, NPC_SPEECH_TSV,
    OBJECT_SOUNDS_TSV,
};
use super::ui::{
    diablo_appears, impact_overlay, ui_action, waypoint_row_chosen, InifussPanel, QuakeLoop,
    UI_SOUNDS,
};
use super::*;
use crate::audio::calls::Handle;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Call {
    Req(i32, Option<UnitKey>, u32, u32),
    Vol(Handle, i32),
    Fade(Handle, i32, u32, u32),
    Detach(Handle, UnitKey, bool),
    StopSpeech,
    Other(&'static str),
    Roll(i32, u32),
    Variant(i32),
}

#[derive(Default)]
struct Fake {
    rate: d2_sim::tick::TickRate,
    log: Vec<Call>,
    rolls: VecDeque<u32>,
    variants: VecDeque<i32>,
    next: Handle,
    speaking: BTreeSet<UnitKey>,
    active: BTreeSet<Handle>,
    unit_reqs: BTreeMap<UnitKey, Vec<(Handle, i32)>>,
    bases: BTreeMap<i32, i32>,
    /// `Group Size` by group base (default 1).
    sizes: BTreeMap<i32, i32>,
    loops: BTreeSet<i32>,
    counts: BTreeMap<Handle, usize>,
    /// Volumes of live requests (`request_volume`).
    vols: BTreeMap<Handle, i32>,
    off: bool,
}

impl Fake {
    fn rolls(mut self, r: &[u32]) -> Self {
        self.rolls = r.iter().copied().collect();
        self
    }
    fn reqs(&self) -> Vec<(i32, Option<UnitKey>, u32)> {
        self.log
            .iter()
            .filter_map(|c| match c {
                Call::Req(id, u, d, _) => Some((*id, *u, *d)),
                _ => None,
            })
            .collect()
    }
    /// Requests with id ≥ 1 (the ones the sound layer makes).
    fn made(&self) -> Vec<(i32, Option<UnitKey>, u32)> {
        self.reqs().into_iter().filter(|r| r.0 >= 1).collect()
    }
    fn draws(&self) -> Vec<i32> {
        self.log
            .iter()
            .filter_map(|c| match c {
                Call::Roll(n, _) => Some(*n),
                _ => None,
            })
            .collect()
    }
}

impl SoundCalls for Fake {
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        self.rate
    }

    fn request(
        &mut self,
        id: i32,
        unit: Option<UnitKey>,
        delay: u32,
        flags: u32,
        _: u32,
    ) -> Handle {
        self.log.push(Call::Req(id, unit, delay, flags));
        if id < 1 || self.off {
            return 0;
        }
        self.next += 1;
        self.active.insert(self.next);
        self.next
    }
    fn set_volume(&mut self, h: Handle, v: i32) {
        if self.active.contains(&h) {
            self.vols.insert(h, v);
        }
        self.log.push(Call::Vol(h, v));
    }
    fn fade(&mut self, h: Handle, t: i32, d: u32, l: u32) {
        self.log.push(Call::Fade(h, t, d, l));
    }
    fn detach(&mut self, h: Handle, u: UnitKey, f: bool) {
        self.log.push(Call::Detach(h, u, f));
    }
    fn stop_handle(&mut self, h: Handle) {
        self.active.remove(&h);
        self.vols.remove(&h);
        self.log.push(Call::Other("stop_handle"));
    }
    fn stop_id(&mut self, _: i32) {
        self.log.push(Call::Other("stop_id"));
    }
    fn stop_songs(&mut self) {
        self.log.push(Call::Other("stop_songs"));
    }
    fn stop_52_71_except(&mut self, _: i32, _: i32) {
        self.log.push(Call::Other("stop_52_71"));
    }
    fn stop_72_201_except(&mut self, _: i32, _: i32) {
        self.log.push(Call::Other("stop_72_201"));
    }
    fn stop_speech(&mut self) {
        self.log.push(Call::StopSpeech);
    }
    fn speaking(&self, u: UnitKey) -> bool {
        self.speaking.contains(&u)
    }
    fn any_speech(&self) -> bool {
        !self.speaking.is_empty()
    }
    fn is_active(&self, h: Handle) -> bool {
        self.active.contains(&h)
    }
    fn sound_tick(&self) -> u32 {
        0
    }
    fn roll(&mut self, n: i32) -> u32 {
        let r = self.rolls.pop_front().expect("unscripted roll");
        assert!(
            (r as i32) < n.max(1),
            "scripted roll {r} out of range for {n}"
        );
        self.log.push(Call::Roll(n, r));
        r
    }
}

impl TriggerSound for Fake {
    fn sound_on(&self) -> bool {
        !self.off
    }
    fn group_base(&self, id: i32) -> i32 {
        self.bases.get(&id).copied().unwrap_or(id)
    }
    fn group_size(&self, base: i32) -> i32 {
        self.sizes.get(&base).copied().unwrap_or(1)
    }
    fn looping(&self, id: i32) -> bool {
        self.loops.contains(&id)
    }
    fn unit_requests(&self, u: UnitKey) -> Vec<(Handle, i32)> {
        self.unit_reqs.get(&u).cloned().unwrap_or_default()
    }
    fn unit_count(&self, h: Handle) -> usize {
        self.counts.get(&h).copied().unwrap_or(1)
    }
    fn variant(&mut self, id: i32) -> i32 {
        self.log.push(Call::Variant(id));
        self.variants.pop_front().unwrap_or(id)
    }
    fn request_volume(&self, h: Handle) -> Option<i32> {
        self.vols.get(&h).copied()
    }
}

const P: UnitKey = UnitKey::new(PLAYER, 1);
const M: UnitKey = UnitKey::new(MONSTER, 7);
const O: UnitKey = UnitKey::new(OBJECT, 9);

fn player(class: i32) -> Unit<'static> {
    let mut u = Unit::new(P, class);
    u.is_local = true;
    u
}

fn zero_sounds() -> Monsounds {
    Monsounds::decode(&[0u8; Monsounds::SIZE])
}

fn monster(r: &Monsounds) -> Unit<'_> {
    let mut u = Unit::new(M, 100);
    u.monsounds = Some(r);
    u
}

fn run<T>(f: &mut Fake, g: &mut Globals, c: u32, body: impl FnOnce(&mut Ctx) -> T) -> T {
    let mut cx = Ctx::new(f, g, c);
    body(&mut cx)
}

// ---- table checks ----

// ---- tables ----

#[test]
fn spec_tsvs_parse_with_their_counts() {
    let o = ObjectSounds::spec();
    assert_eq!(o.rows().len(), 453);
    // The 115 distinct records of the Test vectors are record addresses
    // (real check); several hold equal contents, so the TSV shows fewer.
    let distinct: BTreeSet<_> = o.rows().iter().map(|r| r.record()).collect();
    assert!(distinct.len() <= 115);
    assert!(o.rows().iter().filter(|r| r.ordered).count() > 1);
    let n = NpcSpeech::spec();
    assert_eq!(n.rows().len(), 863);
    let keys: BTreeSet<i32> = n.rows().iter().map(|r| r.key).collect();
    assert_eq!(keys.len(), 862, "exactly one duplicate key");
    // First wins.
    let dup = n
        .rows()
        .iter()
        .find(|r| n.rows().iter().filter(|x| x.key == r.key).count() == 2)
        .unwrap();
    assert_eq!(n.sound(dup.key), dup.sound);
    assert_eq!(n.sound(14), 3488);
    assert_eq!(n.sound(-5), 0);
}

/// Every id the audio tables hand out, for the sounds-without-audio check.
fn table_sound_ids() -> BTreeSet<i32> {
    let mut ids = BTreeSet::new();
    for r in NpcSpeech::spec().rows() {
        ids.insert(r.sound);
    }
    for r in NpcGreetings::spec().rows() {
        ids.extend([r.greet, r.inactive, r.ret]);
        if r.time != 0 {
            // The time-of-day pick is `time + 0..=2`.
            ids.extend(r.time..=r.time + 2);
        }
    }
    for r in ObjectSounds::spec().rows() {
        ids.extend(r.modes);
        ids.extend([r.loop_a, r.loop_b]);
    }
    ids
}

#[test]
fn no_table_names_a_sound_without_audio() {
    for id in d2_data::sounds::SOUNDS_WITHOUT_AUDIO {
        assert!(!table_sound_ids().contains(&(id as i32)), "sound {id}");
    }
    // Ormus has no line for the dropped gossip text; Tyrael greets by his
    // normal lines at every time of day.
    assert_eq!(NpcSpeech::spec().sound(517), 0);
    let mut recs = GreetingRecords::spec();
    for class in [251, 521] {
        assert_eq!(recs.for_class(class).unwrap().time, 0);
    }
}

#[test]
fn tyrael_greets_with_his_normal_lines_at_every_time_of_day() {
    let mut recs = GreetingRecords::spec();
    let mut g = Globals::default();
    for day in 0..=4u8 {
        let mut picks = BTreeSet::new();
        for i in 0..40u32 {
            // The one roll of a pick: greet line (1) or inactive line (0).
            let mut f = Fake::default().rolls(&[i % 2]);
            let rec = recs.for_class(251).unwrap();
            let p = run(&mut f, &mut g, 0, |cx| greet(cx, rec, GreetMode::Idle, day));
            picks.insert(p);
            recs.for_class(251).unwrap().last = 0;
        }
        assert_eq!(
            picks,
            BTreeSet::from([4303, 4305]),
            "day phase {day}: {picks:?}"
        );
    }
}

#[test]
fn monsounds_rows_lose_sounds_without_audio() {
    use crate::audio::unit_feed::clear_sounds_without_audio;
    let mut r = zero_sounds();
    r.neutral = 1595;
    r.taunt = 4640;
    r.attack1 = 1598;
    r.hitsound = 777;
    clear_sounds_without_audio(&mut r);
    assert_eq!((r.neutral, r.taunt, r.attack1, r.hitsound), (0, 0, 0, 777));
    // The cleared idle voice requests nothing.
    let mut g = Globals::default();
    let mut f = Fake::default();
    let mut us = UnitSound::default();
    let m = monster(&r);
    run(&mut f, &mut g, 0, |cx| neutral(cx, &m, &mut us));
    assert!(f.reqs().is_empty());
}

#[test]
fn tsv_parsers_report_perturbations() {
    // Each perturbation is reported at its line and column.
    let bad = OBJECT_SOUNDS_TSV.replacen("27\t2569\t2568", "27\t2569\t25x8", 1);
    let line = OBJECT_SOUNDS_TSV
        .lines()
        .position(|l| l.starts_with("27\t"))
        .unwrap()
        + 1;
    assert_eq!(
        ObjectSounds::parse(&bad),
        Err(TableError::Value {
            table: "object-sounds.tsv",
            line,
            col: 2,
            value: "25x8".into()
        })
    );
    let bad = OBJECT_SOUNDS_TSV.replacen("\t2574\t1\t2574\t2\t0", "\t2574\t9\t2574\t2\t0", 1);
    assert!(matches!(
        ObjectSounds::parse(&bad),
        Err(TableError::Value { col: 10, .. })
    ));
    let bad = OBJECT_SOUNDS_TSV.replacen("\n27\t", "\n25\t", 1);
    assert!(matches!(
        ObjectSounds::parse(&bad),
        Err(TableError::Order { key: 25, .. })
    ));
    let bad = OBJECT_SOUNDS_TSV.replacen("ordered", "orderd", 1);
    assert!(matches!(
        ObjectSounds::parse(&bad),
        Err(TableError::Header { .. })
    ));
    let bad = NPC_SPEECH_TSV.replacen("\n3\t17\t3491", "\n3\t17\t4699", 1);
    assert_eq!(
        NpcSpeech::parse(&bad),
        Err(TableError::Value {
            table: "npc-speech.tsv",
            line: 5,
            col: 2,
            value: "4699".into()
        })
    );
    let bad = NPC_SPEECH_TSV.replacen("\n3\t17\t3491", "\n3\t17", 1);
    assert!(matches!(
        NpcSpeech::parse(&bad),
        Err(TableError::Cells { line: 5, .. })
    ));
    let bad = NPC_SPEECH_TSV.replacen("\n3\t17\t3491", "\n4\t17\t3491", 1);
    assert!(matches!(
        NpcSpeech::parse(&bad),
        Err(TableError::Order { line: 5, .. })
    ));
}

// ---- server sound events, player event sounds ----

#[test]
fn speech_guard_vectors() {
    let mut f = Fake::default();
    let mut g = Globals::default();
    let u = player(0);
    let out = run(&mut f, &mut g, 1000, |cx| player_event(cx, &u, 21)).unwrap();
    assert_eq!(f.reqs(), vec![(2957, Some(P), 0)]);
    assert_eq!(out, vec![Followup::OverheadText(2957)]);
    assert_eq!((g.speech_time, g.speech_id), (1000, 2957));
    run(&mut f, &mut g, 1074, |cx| player_event(cx, &u, 21)).unwrap();
    assert_eq!(f.reqs().len(), 1);
    // Another player's identical line is also held (global guard).
    let mut other = Unit::new(UnitKey::new(PLAYER, 2), 0);
    other.is_local = false;
    run(&mut f, &mut g, 1074, |cx| player_event(cx, &other, 21)).unwrap();
    assert_eq!(f.reqs().len(), 1);
    run(&mut f, &mut g, 1075, |cx| player_event(cx, &u, 21)).unwrap();
    assert_eq!(f.reqs().len(), 2);
    // Speaking: nothing, guard unchanged.
    f.speaking.insert(P);
    run(&mut f, &mut g, 5000, |cx| player_event(cx, &u, 19)).unwrap();
    assert_eq!(f.reqs().len(), 2);
    assert_eq!(g.speech_time, 1075);
}

#[test]
fn chat_and_quest_lines() {
    let mut f = Fake::default();
    let mut g = Globals::default();
    let u = player(1);
    run(&mut f, &mut g, 0, |cx| player_event(cx, &u, 26)).unwrap();
    assert_eq!(f.reqs(), vec![(3251, Some(P), 0)]);
    let a = player(0);
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| player_event(cx, &a, 38)).unwrap();
    run(&mut f, &mut g, 0, |cx| player_event(cx, &a, 39)).unwrap();
    assert_eq!(f.reqs(), vec![(2966, Some(P), 0), (2967, Some(P), 12)]);
    // Stinger event: no request, even while speaking.
    f.speaking.insert(P);
    let out = run(&mut f, &mut g, 0, |cx| player_event(cx, &a, 33)).unwrap();
    assert_eq!(
        out,
        vec![Followup::QuestStinger {
            event: 33,
            id: 2961
        }]
    );
    // Chat while speaking: nothing.
    run(&mut f, &mut g, 0, |cx| player_event(cx, &a, 25)).unwrap();
    assert_eq!(f.reqs().len(), 2);
    // Local player dead: nothing.
    let mut dead = player(0);
    dead.mode = 17;
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| player_event(cx, &dead, 1)).unwrap();
    assert!(f.log.is_empty());
}

#[test]
fn fixed_player_events() {
    let mut f = Fake::default();
    let mut g = Globals::default();
    let u = player(3);
    for e in [1, 2, 10, 11, 12, 18] {
        run(&mut f, &mut g, 0, |cx| player_event(cx, &u, e)).unwrap();
    }
    assert_eq!(
        f.reqs(),
        vec![(235, Some(P), 0), (7, None, 0), (228, Some(P), 0)]
    );
}

fn extra() -> EventExtra<'static> {
    EventExtra {
        local: Some(P),
        event12_stsound: 0,
        greeting: None,
        day_phase: 1,
    }
}

#[test]
fn server_events() {
    let mut f = Fake::default();
    let mut g = Globals::default();
    let mut us = UnitSound::default();
    let rogue = Unit::new(M, events::ROGUEHIRE);
    let other = Unit::new(M, 300);
    run(&mut f, &mut g, 0, |cx| {
        server_event(cx, &rogue, &mut us, 10, extra()).unwrap();
        server_event(cx, &other, &mut us, 13, extra()).unwrap();
        server_event(cx, &other, &mut us, 15, extra()).unwrap();
        server_event(cx, &rogue, &mut us, 15, extra()).unwrap();
        server_event(cx, &other, &mut us, 91, extra()).unwrap();
    });
    assert_eq!(
        f.log[..2],
        [Call::Req(2673, Some(M), 0, 0), Call::Vol(1, 120)]
    );
    assert_eq!(
        f.made(),
        vec![
            (2673, Some(M), 0),
            (2634, Some(M), 0),
            (4290, Some(M), 0),
            (8, Some(P), 0)
        ]
    );
    let mut f = Fake::default();
    let out = run(&mut f, &mut g, 0, |cx| {
        let a = server_event(cx, &rogue, &mut us, 84, extra()).unwrap();
        let b = server_event(cx, &other, &mut us, 87, extra()).unwrap();
        let c = server_event(cx, &other, &mut us, 92, extra()).unwrap();
        // Event 25 on a monster: not a player, nothing.
        let d = server_event(cx, &other, &mut us, 25, extra()).unwrap();
        [a, b, c, d]
    });
    assert_eq!(f.made(), vec![(4615, Some(M), 0), (4622, Some(M), 0)]);
    assert_eq!(out[0], vec![Followup::OverheadText(4615)]);
    assert_eq!(out[2], vec![Followup::StingerRearm]);
    assert!(out[3].is_empty());
    // Event 25 on a player: the chat line (a server event that makes a player event).
    let mut f = Fake::default();
    let p = player(0);
    run(&mut f, &mut g, 0, |cx| {
        server_event(cx, &p, &mut us, 25, extra())
    })
    .unwrap();
    assert_eq!(f.made(), vec![(2937, Some(P), 0)]);
}

#[test]
fn server_events_monster_voices() {
    let mut r = zero_sounds();
    r.taunt = 900;
    r.flee = 901;
    let u = monster(&r);
    let mut us = UnitSound::default();
    let mut g = Globals::default();
    let mut f = Fake::default().rolls(&[2]);
    run(&mut f, &mut g, 10, |cx| {
        server_event(cx, &u, &mut us, 16, extra()).unwrap();
        server_event(cx, &u, &mut us, 17, extra()).unwrap();
        // Flee again within 4 updates of the last voice: nothing, no draw.
        cx.c = 13;
        server_event(cx, &u, &mut us, 17, extra()).unwrap();
        let mut e = extra();
        e.event12_stsound = 55;
        server_event(cx, &u, &mut us, 12, e).unwrap();
    });
    assert_eq!(
        f.made(),
        vec![(900, Some(M), 0), (901, Some(M), 8), (55, Some(M), 0)]
    );
    assert_eq!((us.last_voice, g.last_voice_any), (10, 10));
    // Event 18: greeting on P with flags 1.
    let mut gr = NpcGreeting {
        ret: 77,
        greet: 70,
        ..Default::default()
    };
    let mut f = Fake::default();
    let mut e = extra();
    e.greeting = Some(&mut gr);
    run(&mut f, &mut g, 3, |cx| server_event(cx, &u, &mut us, 18, e)).unwrap();
    assert_eq!(f.log, vec![Call::Variant(70), Call::Req(70, Some(P), 0, 1)]);
}

// ---- mode sounds ----

#[test]
fn player_attack_swing() {
    let mut f = Fake::default();
    let mut g = Globals::default();
    let mut us = UnitSound::default();
    let mut u = player(0);
    u.weapon_hit_class = 5;
    u.speed = 256;
    run(&mut f, &mut g, 40, |cx| mode_sound(cx, &u, &mut us, 7)).unwrap();
    assert_eq!(f.reqs(), vec![(280, Some(P), 8)]);
    assert_eq!(us.last_voice, 40);
    assert_eq!(g.last_voice_any, 0);
    u.weapon_hit_class = 2;
    u.speed = 0;
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| mode_sound(cx, &u, &mut us, 8)).unwrap();
    assert_eq!(f.reqs(), vec![(262, Some(P), 1664)]);
    u.weapon_hit_class = 14;
    let mut f = Fake::default();
    assert_eq!(
        run(&mut f, &mut g, 0, |cx| mode_sound(cx, &u, &mut us, 7)),
        Err(TriggerError::SwingIndex(14))
    );
    // S3 attack only for barbarians; kick delay 4 for assassins.
    let mut f = Fake::default();
    let mut sorc = player(1);
    sorc.weapon_hit_class = 1;
    let mut sin = player(6);
    sin.weapon_hit_class = 1;
    run(&mut f, &mut g, 0, |cx| {
        mode_sound(cx, &sorc, &mut us, 15).unwrap();
        mode_sound(cx, &sin, &mut us, 16).unwrap();
        mode_sound(cx, &sin, &mut us, 12).unwrap();
        mode_sound(cx, &sorc, &mut us, 12).unwrap();
    });
    assert_eq!(
        f.reqs(),
        vec![(251, Some(P), 1664), (255, Some(P), 4), (255, Some(P), 0)]
    );
}

#[test]
fn impact_vectors() {
    assert_eq!(impact(0x22, MONSTER, false, 3), (341, 356));
    assert_eq!(impact(0x30, PLAYER, false, 0), (0, 359));
    assert_eq!(impact(0x35, MONSTER, false, 0), (341, 0));
    assert_eq!(impact(0x35, MONSTER, false, 3), (341, 362));
    assert_eq!(impact(0x35, MONSTER, true, 3), (0, 0));
    assert_eq!(impact(0x10, PLAYER, false, 0), (0, 0));
    assert_eq!(impact(0x51, PLAYER, false, 0), (321, 0));
    assert_eq!(impact(0x51, OBJECT, false, 0), (321, 376));
    assert_eq!(impact(0xBC, PLAYER, false, 0), (325, 391));
}

#[test]
fn monster_hit_and_death() {
    let mut r = zero_sounds();
    r.hitsound = 600;
    r.hitdelay = 3;
    r.deathsound = 745;
    r.deadelay = 2;
    let mut u = monster(&r);
    let mut us = UnitSound {
        hit_class: 0x22,
        ..Default::default()
    };
    let mut g = Globals::default();
    let mut f = Fake::default();
    run(&mut f, &mut g, 50, |cx| mode_sound(cx, &u, &mut us, 3)).unwrap();
    assert_eq!(
        f.reqs(),
        vec![(341, Some(M), 0), (356, Some(M), 0), (600, Some(M), 3)]
    );
    assert_eq!((us.last_voice, g.last_voice_any), (50, 50));
    // Death: Diablo's extra lines, no speaking guard.
    let mut f = Fake::default();
    f.speaking.insert(M);
    us.hit_class = 0;
    run(&mut f, &mut g, 51, |cx| mode_sound(cx, &u, &mut us, 0)).unwrap();
    assert_eq!(
        f.made(),
        vec![(745, Some(M), 2), (746, Some(M), 106), (747, Some(M), 193)]
    );
    // Hit while speaking: impacts only, times still set.
    let mut f = Fake::default();
    f.speaking.insert(M);
    us.hit_class = 0x01;
    run(&mut f, &mut g, 52, |cx| mode_sound(cx, &u, &mut us, 13)).unwrap();
    assert_eq!(f.made(), vec![(321, Some(M), 0)]);
    assert_eq!(us.last_voice, 52);
    // Frozen: impacts only, no time update.
    u.frozen = true;
    let mut f = Fake::default();
    run(&mut f, &mut g, 60, |cx| mode_sound(cx, &u, &mut us, 3)).unwrap();
    assert_eq!(f.made(), vec![(321, Some(M), 0)]);
    assert_eq!(us.last_voice, 52);
    // State 146: nothing at all.
    u.state_146 = true;
    let mut f = Fake::default();
    run(&mut f, &mut g, 61, |cx| mode_sound(cx, &u, &mut us, 3)).unwrap();
    assert!(f.log.is_empty());
    // No record: nothing.
    let bare = Unit::new(M, 1);
    let mut f = Fake::default();
    run(&mut f, &mut g, 61, |cx| mode_sound(cx, &bare, &mut us, 3)).unwrap();
    assert!(f.log.is_empty());
}

#[test]
fn player_hit_death_and_monster_voice_stop() {
    let mut g = Globals::default();
    let mut us = UnitSound {
        hit_class: 0x01,
        ..Default::default()
    };
    let u = player(4);
    let mut f = Fake::default();
    run(&mut f, &mut g, 5, |cx| {
        mode_sound(cx, &u, &mut us, 4).unwrap();
        mode_sound(cx, &u, &mut us, 0).unwrap();
    });
    assert_eq!(
        f.log,
        vec![
            Call::Req(321, Some(P), 0, 0),
            Call::Req(0, Some(P), 0, 0),
            Call::Vol(1, 255),
            Call::Req(2902, Some(P), 2, 0),
            Call::Req(321, Some(P), 0, 0),
            Call::Req(0, Some(P), 0, 0),
            Call::Vol(3, 255),
            Call::Req(2907, Some(P), 1, 0),
        ]
    );
    // Monster death (m = 0) detaches its voices with force first.
    let mut r = zero_sounds();
    r.attack1 = 500;
    r.neutral = 510;
    let m = monster(&r);
    let mut f = Fake::default();
    f.bases.insert(502, 500);
    f.unit_reqs.insert(M, vec![(31, 502), (32, 999), (33, 510)]);
    us.hit_class = 0;
    run(&mut f, &mut g, 5, |cx| mode_sound(cx, &m, &mut us, 0)).unwrap();
    assert_eq!(
        f.log
            .iter()
            .filter(|c| matches!(c, Call::Detach(..)))
            .cloned()
            .collect::<Vec<_>>(),
        vec![Call::Detach(31, M, true), Call::Detach(33, M, true)]
    );
    // Mode 12 (DD) does the same.
    let mut f = Fake::default();
    f.unit_reqs.insert(M, vec![(33, 510)]);
    run(&mut f, &mut g, 5, |cx| mode_sound(cx, &m, &mut us, 12)).unwrap();
    assert_eq!(f.log, vec![Call::Detach(33, M, true)]);
}

#[test]
fn monster_attack_voice() {
    let mut r = zero_sounds();
    r.attack1 = 500;
    r.att1del = 4;
    r.att1prb = 50;
    r.weapon1 = 520;
    r.wea1del = 1;
    r.wea1vol = 99;
    r.neutral = 510;
    r.attack2 = 501;
    r.att2prb = 100;
    let mut u = monster(&r);
    u.speed = 256;
    let mut us = UnitSound::default();
    // Within 15 of the last voice: no draw, weapon only.
    let mut g = Globals {
        last_voice_any: 100,
        ..Default::default()
    };
    let mut f = Fake::default();
    run(&mut f, &mut g, 114, |cx| mode_sound(cx, &u, &mut us, 4)).unwrap();
    assert_eq!(f.log, vec![Call::Req(520, Some(M), 1, 0), Call::Vol(1, 99)]);
    // Allowed: draw 49 < 50 → neutral detach / fade, attack voice, weapon.
    let mut f = Fake::default().rolls(&[49]);
    f.unit_reqs.insert(M, vec![(7, 510), (8, 511)]);
    f.bases.insert(511, 510);
    f.counts.insert(8, 2);
    run(&mut f, &mut g, 115, |cx| mode_sound(cx, &u, &mut us, 4)).unwrap();
    assert_eq!(
        f.log,
        vec![
            Call::Roll(100, 49),
            Call::Fade(7, 0, 0, 6),
            Call::Detach(8, M, true),
            Call::Req(500, Some(M), 4, 0),
            Call::Req(520, Some(M), 1, 0),
            Call::Vol(2, 99),
        ]
    );
    assert_eq!((us.last_voice, g.last_voice_any), (115, 115));
    // Draw 50: no voice.
    let mut f = Fake::default().rolls(&[50]);
    run(&mut f, &mut g, 200, |cx| mode_sound(cx, &u, &mut us, 4)).unwrap();
    assert_eq!(f.made(), vec![(520, Some(M), 1)]);
    // Speaking with Prb < 100: no draw.
    let mut f = Fake::default();
    f.speaking.insert(M);
    run(&mut f, &mut g, 300, |cx| mode_sound(cx, &u, &mut us, 4)).unwrap();
    assert!(f.draws().is_empty());
    // Prb ≥ 100: draws even while speaking and within the gap.
    let mut f = Fake::default().rolls(&[99]);
    f.speaking.insert(M);
    run(&mut f, &mut g, 301, |cx| mode_sound(cx, &u, &mut us, 5)).unwrap();
    assert_eq!(f.draws(), vec![100]);
    assert_eq!(f.made(), vec![(501, Some(M), 0)]);
}

#[test]
fn mode_conversion_skill_voice_block() {
    let mut r = zero_sounds();
    r.cvtmo1 = 4;
    r.cvtsk1 = u32::MAX;
    r.cvttgt1 = 8;
    r.cvtmo2 = 8;
    r.cvtsk2 = 5; // not < 0: no conversion
    r.cvttgt2 = 3;
    r.skill1 = 2692;
    assert_eq!(convert_mode(&r, 4), 8);
    assert_eq!(convert_mode(&r, 5), 5);
    let u = monster(&r);
    let mut us = UnitSound::default();
    let mut g = Globals::default();
    let mut f = Fake::default();
    run(&mut f, &mut g, 24, |cx| mode_sound(cx, &u, &mut us, 4)).unwrap();
    assert!(f.log.is_empty(), "chicken within 25 of U+0x7C = 0");
    run(&mut f, &mut g, 25, |cx| mode_sound(cx, &u, &mut us, 4)).unwrap();
    assert_eq!(f.reqs(), vec![(2692, Some(M), 0)]);
    assert_eq!((us.last_voice, g.last_voice_any), (25, 25));
    // Block by hit class.
    let p = player(3);
    let mut f = Fake::default();
    for h in [1u8, 10, 12] {
        us.hit_class = h;
        run(&mut f, &mut g, 30, |cx| mode_sound(cx, &p, &mut us, 9)).unwrap();
    }
    assert_eq!(
        f.reqs(),
        vec![(398, Some(P), 0), (401, Some(P), 0), (0, Some(P), 0)]
    );
    // Objects are not handled by the mode dispatch.
    let o = Unit::new(O, 27);
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| mode_sound(cx, &o, &mut us, 0)).unwrap();
    assert!(f.log.is_empty());
}

// ---- footsteps ----

fn walker(r: &Monsounds) -> Unit<'_> {
    let mut u = monster(r);
    u.frame_count = 2048;
    u.speed = 256;
    u.frame = 1024;
    u.mode = 2;
    u
}

#[test]
fn footstep_vectors() {
    let mut r = zero_sounds();
    r.fscnt = 2;
    r.footstep = 700;
    let u = walker(&r);
    let mut g = Globals::default();
    let mut us = UnitSound {
        last_footstep: 100,
        ..Default::default()
    };
    // Elapsed 1 < ⌊8/3⌋ = 2: nothing.
    let mut f = Fake::default();
    run(&mut f, &mut g, 101, |cx| footstep(cx, &u, &mut us, 1)).unwrap();
    assert!(f.log.is_empty());
    // Elapsed 2: step, volume 200.
    run(&mut f, &mut g, 102, |cx| footstep(cx, &u, &mut us, 1)).unwrap();
    assert_eq!(
        f.log,
        vec![Call::Req(700, Some(M), 0, 0), Call::Vol(1, 200)]
    );
    assert_eq!(us.last_footstep, 102);
    // Elapsed 7 > 6: 200 → 125; material 3 → +8; running → +24.
    let mut f = Fake::default();
    let mut run_u = u;
    run_u.mode = 15;
    run(&mut f, &mut g, 109, |cx| footstep(cx, &run_u, &mut us, 3)).unwrap();
    assert_eq!(
        f.log,
        vec![Call::Req(732, Some(M), 0, 0), Call::Vol(1, 125)]
    );
    // Not at a step point (f = 1024 + 512): nothing.
    let mut off = u;
    off.frame = 1536;
    let mut f = Fake::default();
    run(&mut f, &mut g, 200, |cx| footstep(cx, &off, &mut us, 1)).unwrap();
    assert!(f.log.is_empty());
    // Player: class base, local volume 255 → 160.
    let mut p = player(4);
    p.frame_count = 2048;
    p.speed = 256;
    p.mode = 3;
    let mut us = UnitSound::default();
    let mut f = Fake::default();
    run(&mut f, &mut g, 1000, |cx| footstep(cx, &p, &mut us, 6)).unwrap();
    assert_eq!(
        f.log,
        vec![Call::Req(2816 + 20 + 24, Some(P), 0, 0), Call::Vol(1, 160)]
    );
    // Layer: roll(100) after the step, < FsPrb.
    r.footsteplayer = 750;
    r.fsprb = 30;
    let u = walker(&r);
    let mut us = UnitSound::default();
    let mut f = Fake::default().rolls(&[29]);
    run(&mut f, &mut g, 1000, |cx| footstep(cx, &u, &mut us, 0)).unwrap();
    assert_eq!(
        f.log,
        vec![
            Call::Req(700, Some(M), 0, 0),
            Call::Vol(1, 125),
            Call::Roll(100, 29),
            Call::Req(750, Some(M), 0, 0)
        ]
    );
    // Speed 0 or FsCnt 0: nothing.
    let mut z = u;
    z.speed = 0;
    let mut f = Fake::default();
    run(&mut f, &mut g, 1000, |cx| footstep(cx, &z, &mut us, 0)).unwrap();
    r.fscnt = 0;
    let z = walker(&r);
    run(&mut f, &mut g, 1000, |cx| footstep(cx, &z, &mut us, 0)).unwrap();
    assert!(f.log.is_empty());
}

#[test]
fn circular_distance_vectors() {
    assert_eq!(circular_distance(1024, 0, 1024), 0);
    assert_eq!(circular_distance(1280, 0, 1024), 256);
    assert_eq!(circular_distance(768, 0, 1024), 256);
    assert_eq!(circular_distance(10, 0, 12), 2);
    assert_eq!(circular_distance(1, 11, 12), 2);
}

#[test]
fn footstep_callers_and_material() {
    assert!(footstep_called(PLAYER, 0, 2));
    assert!(footstep_called(PLAYER, 0, 19));
    assert!(!footstep_called(PLAYER, 0, 1));
    assert!(footstep_called(MONSTER, 3, 15));
    assert!(!footstep_called(MONSTER, 3, 1));
    assert!(footstep_called(MONSTER, 15, 1));
    assert!(footstep_called(MONSTER, 110, 8));
    assert!(!footstep_called(MONSTER, 110, 2));
    assert_eq!(footstep_material(3, Floor::NoRoom), 0);
    assert_eq!(footstep_material(3, Floor::NotFound), 3);
    assert_eq!(footstep_material(9, Floor::NotFound), 1);
    assert_eq!(footstep_material(3, Floor::Tile(0x28)), 1);
    assert_eq!(footstep_material(3, Floor::Tile(0x408)), 2);
    assert_eq!(footstep_material(3, Floor::Tile(0x400)), 5);
    assert_eq!(footstep_material(3, Floor::Tile(0x80)), 6);
    assert_eq!(footstep_material(4, Floor::Tile(0x01)), 4);
}

// ---- monster idle voices ----

#[test]
fn neutral_voice() {
    let mut r = zero_sounds();
    r.neutral = 510;
    r.neutime = 10;
    let mut u = monster(&r);
    u.mode = 1;
    u.near_local = true;
    let mut us = UnitSound::default();
    let mut g = Globals::default();
    // All conditions hold: roll(15) = 0, request, then roll(61).
    let mut f = Fake::default().rolls(&[0, 20]);
    run(&mut f, &mut g, 100, |cx| neutral(cx, &u, &mut us));
    assert_eq!(
        f.log,
        vec![
            Call::Roll(15, 0),
            Call::Req(510, Some(M), 0, 0),
            Call::Roll(61, 20)
        ]
    );
    assert_eq!((g.last_idle_any, us.last_idle, g.idle_gap), (100, 100, 50));
    // Gap not reached: no draw.
    let mut f = Fake::default();
    run(&mut f, &mut g, 149, |cx| neutral(cx, &u, &mut us));
    assert!(f.log.is_empty());
    // Roll ≠ 0: no request.
    let mut f = Fake::default().rolls(&[3]);
    run(&mut f, &mut g, 150, |cx| neutral(cx, &u, &mut us));
    assert_eq!(f.log, vec![Call::Roll(15, 3)]);
    // Already a request in the group: no draw; but with the system off the
    // group test is false and the draw happens.
    let mut f = Fake::default();
    f.unit_reqs.insert(M, vec![(4, 510)]);
    run(&mut f, &mut g, 150, |cx| neutral(cx, &u, &mut us));
    assert!(f.log.is_empty());
    f.off = true;
    f.rolls = [1].into();
    run(&mut f, &mut g, 150, |cx| neutral(cx, &u, &mut us));
    assert_eq!(f.draws(), vec![15]);
    // Town, far away, wrong mode: no draw.
    for change in 0..3 {
        let mut v = u;
        match change {
            0 => v.in_town = true,
            1 => v.near_local = false,
            _ => v.mode = 8,
        }
        let mut f = Fake::default();
        run(&mut f, &mut g, 150, |cx| neutral(cx, &v, &mut us));
        assert!(f.log.is_empty());
    }
    // Mode 8 for base class 110.
    let mut v = u;
    v.mode = 8;
    v.base_class = 110;
    let mut f = Fake::default().rolls(&[1]);
    run(&mut f, &mut g, 150, |cx| neutral(cx, &v, &mut us));
    assert_eq!(f.draws(), vec![15]);
    assert!(within_700(699, 0));
    assert!(!within_700(0, 350));
}

#[test]
fn init_and_flee() {
    let mut r = zero_sounds();
    r.init = 530;
    r.skill1 = 540;
    r.flee = 550;
    let u = monster(&r);
    let mut us = UnitSound::default();
    let mut g = Globals::default();
    let mut f = Fake::default().rolls(&[0]);
    run(&mut f, &mut g, 7, |cx| init_voice(cx, &u, &mut us));
    assert_eq!(
        f.log,
        vec![Call::Req(530, Some(M), 0, 0), Call::Roll(61, 0)]
    );
    assert_eq!((us.last_idle, g.idle_gap), (7, 30));
    // Second time (U+0x80 ≠ 0), mode 8: skill voice.
    let mut v = u;
    v.mode = 8;
    let mut f = Fake::default();
    run(&mut f, &mut g, 8, |cx| init_voice(cx, &v, &mut us));
    assert_eq!(f.reqs(), vec![(540, Some(M), 0)]);
    // Flee: roll(3) before the request.
    let mut f = Fake::default().rolls(&[0]);
    run(&mut f, &mut g, 12, |cx| flee(cx, &u, &mut us));
    assert_eq!(f.log, vec![Call::Roll(3, 0), Call::Req(550, Some(M), 6, 0)]);
}

// ---- object mode sounds ----

#[test]
fn object_vectors() {
    let t = ObjectSounds::spec();
    let mut g = Globals::default();
    // Door 27: first call m 0 only sets the fields; m 1 → transition 2568.
    let mut door = Unit::new(O, 27);
    let mut us = UnitSound::default();
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| object_mode(cx, t, &door, &mut us)).unwrap();
    assert!(f.log.is_empty());
    door.mode = 1;
    run(&mut f, &mut g, 1, |cx| object_mode(cx, t, &door, &mut us)).unwrap();
    assert_eq!(f.reqs(), vec![(2568, Some(O), 0)]);
    // Same mode again: nothing.
    run(&mut f, &mut g, 2, |cx| object_mode(cx, t, &door, &mut us)).unwrap();
    assert_eq!(f.reqs().len(), 1);
    // Brazier 39, first call m 1: loop a, no transition.
    let mut br = Unit::new(O, 39);
    br.mode = 1;
    let mut us = UnitSound::default();
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| object_mode(cx, t, &br, &mut us)).unwrap();
    assert_eq!(f.reqs(), vec![(2574, Some(O), 0)]);
    // m 2 with the loop already playing: no new loop request.
    br.mode = 2;
    f.unit_reqs.insert(O, vec![(1, 2574)]);
    run(&mut f, &mut g, 1, |cx| object_mode(cx, t, &br, &mut us)).unwrap();
    assert_eq!(f.reqs().len(), 1);
    // m 0: no loop → detach the looping requests (no force).
    br.mode = 0;
    f.loops.insert(2574);
    f.unit_reqs.insert(O, vec![(1, 2574), (2, 9)]);
    run(&mut f, &mut g, 2, |cx| object_mode(cx, t, &br, &mut us)).unwrap();
    assert_eq!(f.log.last(), Some(&Call::Detach(1, O, false)));
    // Well (ordered): 2 → 1 gives no transition; 1 → 2 does.
    let mut well = Unit::new(O, 111);
    well.mode = 2;
    let mut us = UnitSound {
        obj_seen: true,
        obj_prev_mode: 1,
        ..Default::default()
    };
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| object_mode(cx, t, &well, &mut us)).unwrap();
    well.mode = 1;
    run(&mut f, &mut g, 1, |cx| object_mode(cx, t, &well, &mut us)).unwrap();
    assert_eq!(f.reqs(), vec![(2644, Some(O), 0)]);
    // Town portal 59: transition on the first call.
    let mut tp = Unit::new(O, 59);
    tp.mode = 1;
    let mut us = UnitSound::default();
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| object_mode(cx, t, &tp, &mut us)).unwrap();
    assert_eq!(f.reqs(), vec![(2633, Some(O), 0)]);
    // Class > 572: fatal.
    let bad = Unit::new(O, 573);
    assert_eq!(
        run(&mut f, &mut g, 0, |cx| object_mode(cx, t, &bad, &mut us)),
        Err(TriggerError::ObjectClass(573))
    );
}

#[test]
fn cain_gibbet_line() {
    let t = ObjectSounds::spec();
    let mut g = Globals::default();
    let mut cain = Unit::new(O, 26);
    cain.local_dist = 19;
    let mut us = UnitSound::default();
    let mut f = Fake::default();
    run(&mut f, &mut g, 33, |cx| object_mode(cx, t, &cain, &mut us)).unwrap();
    assert_eq!(f.reqs(), vec![(3671, Some(O), 0)]);
    assert_eq!(us.last_idle, 33);
    run(&mut f, &mut g, 34, |cx| object_mode(cx, t, &cain, &mut us)).unwrap();
    assert_eq!(f.reqs().len(), 1);
    let mut far = cain;
    far.local_dist = 20;
    let mut us = UnitSound::default();
    let mut f = Fake::default();
    run(&mut f, &mut g, 33, |cx| object_mode(cx, t, &far, &mut us)).unwrap();
    assert!(f.log.is_empty());
}

// ---- skills, missiles, states, items ----

#[test]
fn skill_start_sounds() {
    let mut g = Globals::default();
    let mut u = player(1);
    u.weapon_hit_class = 1;
    u.speed = 256;
    let sk = SkillStart {
        stsound: 600,
        stsoundclass: 610,
        stsounddelay: true,
        weaponsnd: true,
        stsuccessonly: false,
        charclass: 1,
        item_cast_sound: None,
    };
    // swing = (6·256+128)/256 = 6; delay 6 − 3 = 3.
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| skill_start(cx, &u, &sk, true)).unwrap();
    assert_eq!(
        f.reqs(),
        vec![(600, Some(P), 3), (251, Some(P), 6), (610, Some(P), 3)]
    );
    // Item cast sound replaces stsound and suppresses stsoundclass.
    let mut f = Fake::default();
    let item = SkillStart {
        item_cast_sound: Some(620),
        weaponsnd: false,
        ..sk
    };
    run(&mut f, &mut g, 0, |cx| skill_start(cx, &u, &item, true)).unwrap();
    assert_eq!(f.reqs(), vec![(620, Some(P), 3)]);
    // Negative delay passed as is (speed large: swing 0).
    u.speed = 100_000;
    let mut f = Fake::default();
    let only = SkillStart {
        weaponsnd: false,
        stsoundclass: 0,
        ..sk
    };
    run(&mut f, &mut g, 0, |cx| skill_start(cx, &u, &only, true)).unwrap();
    assert_eq!(f.reqs(), vec![(600, Some(P), (-3i32) as u32)]);
    // stsuccessonly and a failed start: nothing.
    let mut f = Fake::default();
    let so = SkillStart {
        stsuccessonly: true,
        ..sk
    };
    run(&mut f, &mut g, 0, |cx| skill_start(cx, &u, &so, false)).unwrap();
    assert!(f.log.is_empty());
}

#[test]
fn skill_missile_state_sounds() {
    let mut g = Globals::default();
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| {
        skills::skill_do(cx, P, Some(M), 10, 11);
        skills::skill_do(cx, P, None, 0, 11);
        skills::missile_hit(cx, O, -1);
        skills::missile_hit(cx, O, 0);
        skills::state_on(cx, M, false, true, true, 20);
        skills::state_on(cx, M, true, false, false, 21);
        skills::state_on(cx, M, false, false, false, 22);
        skills::state_off(cx, M, true, 23);
        skills::state_off(cx, M, false, 24);
    });
    assert_eq!(
        f.reqs(),
        vec![
            (10, Some(P), 0),
            (11, Some(M), 0),
            (0, Some(O), 0),
            (22, Some(M), 0),
            (23, Some(M), 0)
        ]
    );
}

#[test]
fn item_sounds() {
    let base = ItemSounds {
        dropsound: 300,
        dropsfxframe: 0,
        usesound: 301,
    };
    let uniq = ItemSounds {
        dropsound: 310,
        dropsfxframe: 0,
        usesound: 311,
    };
    let plain = ItemRows {
        base,
        quality: 2,
        special: None,
    };
    assert_eq!(plain.drop_sound(), (300, 12));
    let u = ItemRows {
        quality: 7,
        special: Some(uniq),
        ..plain
    };
    assert_eq!(u.drop_sound(), (310, 12));
    assert_eq!(u.use_sound(), 311);
    let s = ItemRows {
        quality: 5,
        special: Some(ItemSounds {
            dropsfxframe: 4,
            ..uniq
        }),
        ..plain
    };
    assert_eq!(s.drop_sound(), (310, 4));
    // Unique row without a drop sound: the base stays.
    let nu = ItemRows {
        quality: 7,
        special: Some(ItemSounds {
            dropsound: 0,
            ..uniq
        }),
        ..plain
    };
    assert_eq!(nu.use_sound(), 301);
    // Use sound only when the base dropsound is set.
    let nodrop = ItemRows {
        base: ItemSounds {
            dropsound: 0,
            ..base
        },
        ..plain
    };
    assert_eq!(nodrop.use_sound(), 0);
    let item = UnitKey::new(4, 3);
    let mut g = Globals::default();
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| {
        item_drop(cx, item, &plain);
        skills::item_to_cursor(cx);
        skills::item_place(cx, &u);
        skills::item_use(cx, &nodrop);
        skills::item_gold(cx);
    });
    assert_eq!(
        f.log,
        vec![
            Call::Req(216, Some(item), 0, 0),
            Call::Req(300, Some(item), 12, 0),
            Call::Vol(2, 180),
            Call::Req(235, None, 0, 0),
            Call::Req(310, None, 0, 0),
            Call::Req(0, None, 0, 0),
            Call::Req(221, None, 0, 0),
        ]
    );
}

// ---- NPC speech ----

#[test]
fn greeting_picks() {
    let mut g = Globals::default();
    // Akara-like record, mode 2: return line, no draw, no variant.
    let mut akara = NpcGreeting {
        greet: 100,
        inactive: 110,
        time: 120,
        ret: 130,
        ..Default::default()
    };
    let mut f = Fake::default();
    let pick = run(&mut f, &mut g, 5, |cx| {
        greet(cx, &mut akara, GreetMode::Return, 1)
    });
    assert_eq!(pick, 130);
    assert!(f.log.is_empty());
    // Mode 0: roll(2) = 0 → inactive; roll(3) = 1 keeps it; variant.
    let mut f = Fake::default().rolls(&[0, 1]);
    f.variants = [112].into();
    let pick = run(&mut f, &mut g, 6, |cx| {
        greet(cx, &mut akara, GreetMode::Idle, 2)
    });
    assert_eq!(pick, 112);
    assert_eq!(
        f.log,
        vec![Call::Roll(2, 0), Call::Roll(3, 1), Call::Variant(110)]
    );
    assert_eq!((akara.last, akara.tick), (112, 6));
    // Same pick as last → retry; time line for day phase 2–3 = +1.
    let mut f = Fake::default().rolls(&[1, 0, 1, 2]);
    f.variants = [112, 121].into();
    let pick = run(&mut f, &mut g, 7, |cx| {
        greet(cx, &mut akara, GreetMode::Idle, 3)
    });
    assert_eq!(pick, 121);
    assert_eq!(
        f.log,
        vec![
            Call::Roll(2, 1),
            Call::Roll(3, 0),
            Call::Variant(121),
            Call::Roll(2, 1),
            Call::Roll(3, 2),
            Call::Variant(100)
        ]
    );
    // Warriv-like (greet 0, time 0), mode 1: variant(0) every attempt until
    // it differs from last; here last = 0, so all 20 attempts, 20th kept.
    let mut w = NpcGreeting::default();
    let mut f = Fake::default();
    let pick = run(&mut f, &mut g, 8, |cx| {
        greet(cx, &mut w, GreetMode::Event, 1)
    });
    assert_eq!(pick, 0);
    assert_eq!(f.log, vec![Call::Variant(0); 20]);
    // time ≠ 0 and s = 0: no roll(3), time line for phase 4 = +2.
    let mut t = NpcGreeting {
        time: 50,
        ..Default::default()
    };
    let mut f = Fake::default();
    run(&mut f, &mut g, 8, |cx| {
        greet(cx, &mut t, GreetMode::Event, 4)
    });
    assert_eq!(f.log, vec![Call::Variant(52)]);
}

#[test]
fn dialog_lines() {
    let table = NpcSpeech::spec();
    let mut g = Globals::default();
    let mut st = DialogState::default();
    let npc = Unit::new(M, 148);
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| {
        dialog_line(cx, &mut st, table, &npc, Some(P), 14)
    });
    assert_eq!(
        f.log,
        vec![Call::StopSpeech, Call::Req(3488, Some(P), 5, 0)]
    );
    assert_eq!((st.handle, st.id), (1, 3488));
    // Next line: fade the previous one over 4, stop speech, request.
    run(&mut f, &mut g, 1, |cx| {
        dialog_line(cx, &mut st, table, &npc, Some(P), 15)
    });
    assert_eq!(
        f.log[2..],
        [
            Call::Fade(1, 0, 0, 4),
            Call::StopSpeech,
            Call::Req(3489, Some(P), 5, 0)
        ]
    );
    // Option 1 disables the lines (text on).
    assert!(set_npc_speech_option(&mut st, 1));
    let mut f = Fake::default();
    run(&mut f, &mut g, 2, |cx| {
        dialog_line(cx, &mut st, table, &npc, Some(P), 16)
    });
    assert_eq!(f.log, vec![Call::StopSpeech]);
    assert!(!set_npc_speech_option(&mut st, 0));
    assert!(st.enabled);
    assert!(set_npc_speech_option(&mut st, 2));
    assert!(st.enabled);
    // Unknown key: no request.
    let mut f = Fake::default();
    run(&mut f, &mut g, 2, |cx| {
        dialog_line(cx, &mut st, table, &npc, Some(P), 60000)
    });
    assert_eq!(f.log, vec![Call::StopSpeech]);
}

// Cancelling the dialog text stops its line: a playing line fades over 4,
// one still in its start delay is stopped, the handle is cleared, and a
// second close (or none remembered) does nothing.
#[test]
fn dialog_close_stops_the_line() {
    let table = NpcSpeech::spec();
    let mut g = Globals::default();
    let npc = Unit::new(M, 148);
    // Playing on the local player: fade to 0 over 4.
    let mut st = DialogState::default();
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| {
        dialog_line(cx, &mut st, table, &npc, Some(P), 14)
    });
    f.speaking.insert(P);
    f.log.clear();
    run(&mut f, &mut g, 1, |cx| dialog_close(cx, &mut st, Some(P)));
    assert_eq!(f.log, vec![Call::Fade(1, 0, 0, 4)]);
    assert_eq!(st.handle, 0);
    run(&mut f, &mut g, 2, |cx| dialog_close(cx, &mut st, Some(P)));
    assert_eq!(f.log.len(), 1, "nothing left to stop");
    // Still waiting for its delay: stopped outright.
    let mut st = DialogState::default();
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| {
        dialog_line(cx, &mut st, table, &npc, Some(P), 14)
    });
    f.log.clear();
    run(&mut f, &mut g, 1, |cx| dialog_close(cx, &mut st, Some(P)));
    assert_eq!(f.log, vec![Call::Other("stop_handle")]);
    assert_eq!(st.handle, 0);
}

// ---- UI sounds and other fixed requests ----

#[test]
fn ui_action_sounds() {
    let mut g = Globals::default();
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| {
        ui_action(cx, 1, 33, 0);
        ui_action(cx, 2, 4, 0);
        ui_action(cx, 2, 18, 0);
        ui_action(cx, 2, 32, 0);
        ui_action(cx, 2, 33, 0);
        ui_action(cx, 2, 99, 0);
        ui_action(cx, 0x10, 10, 0);
        ui_action(cx, 0x10, 33, 1234);
        ui_action(cx, 0x11, 33, 1234);
    });
    let ids: Vec<i32> = f.reqs().iter().map(|r| r.0).collect();
    assert_eq!(ids, vec![237, 241, 7, 217, 243, 2456, 2474, 1234, 237]);
    assert!(f.reqs().iter().all(|r| r.1.is_none() && r.2 == 0));
}

#[test]
fn hireling_and_class_speech_events() {
    let mut g = Globals::default();
    let mut us = UnitSound::default();
    let rogue = Unit::new(M, events::ROGUEHIRE);
    let other = Unit::new(M, 300);
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| {
        for e in [85, 86] {
            server_event(cx, &rogue, &mut us, e, extra()).unwrap();
            server_event(cx, &other, &mut us, e, extra()).unwrap();
        }
    });
    let ids: Vec<i32> = f.made().iter().map(|r| r.0).collect();
    assert_eq!(ids, vec![4612, 4616, 4613, 4619]);
    let u = player(0);
    let mut f = Fake::default();
    // Distinct ids: the global guard does not hold them.
    run(&mut f, &mut g, 0, |cx| {
        for e in [20, 22, 23, 24] {
            player_event(cx, &u, e).unwrap();
        }
    });
    let ids: Vec<i32> = f.made().iter().map(|r| r.0).collect();
    assert_eq!(ids, vec![2936, 2955, 2934, 2959]);
}

#[test]
fn unit_sound_fields_and_globals_start_at_zero() {
    // The per-unit fields (+0x70, +0x74, +0x7C, +0x80, +0x84, +0xB0) and
    // the global timers, the idle gap included, all start 0.
    let us = UnitSound::default();
    assert!(!us.obj_seen);
    assert_eq!(
        (
            us.obj_prev_mode,
            us.last_voice,
            us.last_idle,
            us.last_footstep,
            us.hit_class
        ),
        (0, 0, 0, 0, 0)
    );
    let g = Globals::default();
    assert_eq!(
        (
            g.last_idle_any,
            g.last_voice_any,
            g.idle_gap,
            g.speech_time,
            g.speech_id
        ),
        (0, 0, 0, 0, 0)
    );
}

// ---- settled rules ----

#[test]
fn footstep_integer_types() {
    // Step not a power of two: signed remainders, so f − s < 0 reduces to a
    // negative value (|b − a| uses it as is).
    assert_eq!(circular_distance(-2, 0, 12), 2);
    assert_eq!(circular_distance(-14, 0, 12), 2);
    // Power of two: two's-complement mask.
    assert_eq!(circular_distance(-256, 0, 1024), 256);
    // Step 0 masks with 0xFFFFFFFF: no reduction.
    assert_eq!(circular_distance(7, 3, 0), 4);
    // s is U+0x4C as signed 16-bit: 0x10000 reads 0 → nothing.
    let mut r = zero_sounds();
    r.fscnt = 2;
    r.footstep = 700;
    let mut u = monster(&r);
    u.frame_count = 2048;
    u.frame = 1024;
    u.mode = 2;
    u.speed = 0x10000;
    let mut g = Globals::default();
    let mut us = UnitSound::default();
    let mut f = Fake::default();
    run(&mut f, &mut g, 1000, |cx| footstep(cx, &u, &mut us, 1)).unwrap();
    assert!(f.log.is_empty());
    // F < n (step 0) is not fatal: dist is |o − x| and the step test fails
    // at f = 0 with s = 256.
    u.speed = 256;
    u.frame_count = 1;
    u.frame = 0;
    let mut f = Fake::default();
    assert!(run(&mut f, &mut g, 1000, |cx| footstep(cx, &u, &mut us, 1)).is_ok());
}

#[test]
fn objects_without_a_record_and_cairn_loops() {
    let t = ObjectSounds::spec();
    let no_record = (0..=572).find(|&c| t.get(c).is_none()).unwrap();
    let mut g = Globals::default();
    // No record: all-zero row; loop 0 detaches the looping requests, and
    // the fields are still updated.
    let mut u = Unit::new(O, no_record);
    u.mode = 1;
    let mut us = UnitSound::default();
    let mut f = Fake::default();
    f.loops.insert(77);
    f.unit_reqs.insert(O, vec![(4, 77)]);
    run(&mut f, &mut g, 0, |cx| object_mode(cx, t, &u, &mut us)).unwrap();
    assert_eq!(f.log, vec![Call::Detach(4, O, false)]);
    assert_eq!((us.obj_seen, us.obj_prev_mode), (true, 1));
    // Mode ≥ 8 is fatal.
    u.mode = 8;
    assert_eq!(
        run(&mut f, &mut g, 0, |cx| object_mode(cx, t, &u, &mut us)),
        Err(TriggerError::ObjectMode(8))
    );
    // Cairn stone (61) in mode 1–5 loops cairn_stone_<m> (413–417).
    for m in 1..=5u8 {
        let mut c = Unit::new(O, 61);
        c.mode = m;
        let mut us = UnitSound::default();
        let mut f = Fake::default();
        run(&mut f, &mut g, 0, |cx| object_mode(cx, t, &c, &mut us)).unwrap();
        assert_eq!(f.reqs(), vec![(412 + i32::from(m), Some(O), 0)]);
    }
}

#[test]
fn greeting_records_from_the_tsv() {
    let t = NpcGreetings::parse(NPC_GREETINGS_TSV).unwrap();
    assert_eq!(t.rows().len(), 35);
    let mut recs = GreetingRecords::new(&t);
    assert_eq!(recs.len(), 28);
    // Akara (148).
    let a = *recs.for_class(148).unwrap();
    assert_eq!(
        (a.greet, a.inactive, a.time, a.ret),
        (3480, 3482, 3485, 3484)
    );
    // No record → none (the greeting is id 0).
    assert!(recs.for_class(0).is_none());
    // The Cain classes share one record's last / tick.
    let cains: Vec<i32> = t
        .rows()
        .iter()
        .filter(|r| r.record == 0x0072_AE88)
        .map(|r| r.class)
        .collect();
    assert_eq!(cains.len(), 6);
    recs.for_class(cains[0]).unwrap().last = 3644;
    assert_eq!(recs.for_class(cains[5]).unwrap().last, 3644);
    // Strict parse.
    assert!(NpcGreetings::parse(
        "class\tmonstats\trecord\tgreet\tinactive\ttime\treturn\n1\tx\t72AE88\t0\t0\t0\t0"
    )
    .is_err());
}

#[test]
fn greeting_mode_2_writes_nothing() {
    let mut g = Globals::default();
    let mut rec = NpcGreeting {
        greet: 100,
        ret: 130,
        last: 7,
        tick: 3,
        ..Default::default()
    };
    let mut f = Fake::default();
    let pick = run(&mut f, &mut g, 50, |cx| {
        greet(cx, &mut rec, GreetMode::Return, 1)
    });
    assert_eq!(pick, 130);
    assert_eq!((rec.last, rec.tick), (7, 3));
}

#[test]
fn skill_voices_match_raw_cells() {
    let mut r = zero_sounds();
    r.skill1 = 541; // not a group base: compared raw
    let mut npc = monster(&r);
    let mut g = Globals::default();
    let mut rec = NpcGreeting::default();
    let mut f = Fake::default();
    // Handle 1: id 542 in group 541 (base 541 = cell); handle 2: id 541
    // whose base is 540 (≠ cell); handle 3: freed (id 0, base 0 = the 0
    // cells).
    f.bases.insert(542, 541);
    f.bases.insert(541, 540);
    f.unit_reqs.insert(M, vec![(1, 542), (2, 541), (3, 0)]);
    run(&mut f, &mut g, 0, |cx| {
        interact_greeting(cx, &mut rec, &npc, Some(P), GreetMode::Return, 1)
    });
    let detached: Vec<Handle> = f
        .log
        .iter()
        .filter_map(|c| match c {
            Call::Detach(h, M, true) => Some(*h),
            _ => None,
        })
        .collect();
    assert_eq!(detached, vec![1, 3]);
    // Identity type not 1 (a player NPC unit): nothing detached.
    npc.identity_type = PLAYER;
    let mut f = Fake::default();
    f.unit_reqs.insert(M, vec![(1, 541)]);
    run(&mut f, &mut g, 0, |cx| {
        interact_greeting(cx, &mut rec, &npc, Some(P), GreetMode::Return, 1)
    });
    assert!(!f.log.iter().any(|c| matches!(c, Call::Detach(..))));
}

#[test]
fn player_class_is_checked_before_the_dead_test() {
    let mut g = Globals::default();
    let mut dead = player(9);
    dead.mode = 17;
    let mut f = Fake::default();
    assert_eq!(
        run(&mut f, &mut g, 0, |cx| player_event(cx, &dead, 2)),
        Err(TriggerError::PlayerClass(9))
    );
    // Another (non-local) dead player still plays.
    let mut other = Unit::new(UnitKey::new(PLAYER, 2), 0);
    other.mode = 17;
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| player_event(cx, &other, 2)).unwrap();
    assert_eq!(f.made(), vec![(7, None, 0)]);
}

#[test]
fn server_events_read_the_sound_identity() {
    let mut r = zero_sounds();
    r.taunt = 600;
    // A player transformed into a monster form: identity type 1.
    let mut u = player(0);
    u.identity_type = MONSTER;
    u.monsounds = Some(&r);
    let mut g = Globals::default();
    let mut us = UnitSound::default();
    let mut f = Fake::default();
    let extra = EventExtra {
        local: Some(P),
        event12_stsound: 0,
        greeting: None,
        day_phase: 0,
    };
    run(&mut f, &mut g, 0, |cx| {
        server_event(cx, &u, &mut us, 16, extra)
    })
    .unwrap();
    assert_eq!(f.made(), vec![(600, Some(P), 0)]);
    // Not a player for the player-event fall-through.
    let mut f = Fake::default();
    let extra = EventExtra {
        local: Some(P),
        event12_stsound: 0,
        greeting: None,
        day_phase: 0,
    };
    run(&mut f, &mut g, 0, |cx| {
        server_event(cx, &u, &mut us, 2, extra)
    })
    .unwrap();
    assert!(f.made().is_empty());
}

#[test]
fn ui_action_first_match_wins() {
    let mut g = Globals::default();
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| {
        ui_action(cx, 3, 33, 0); // bit 0: 237 only
        ui_action(cx, 1, 4, 0); // bit 0, c ≠ 33: nothing
        ui_action(cx, 0x12, 33, 5); // bit 1: 243, not the 0x10 rows
    });
    let ids: Vec<i32> = f.reqs().iter().map(|r| r.0).collect();
    assert_eq!(ids, vec![237, 243]);
}

#[test]
fn dosound_a_and_b() {
    let mut g = Globals::default();
    // cltdofunc 16 (Jab): frame event 3 only; player a, monster b.
    let p = player(0);
    let mut f = Fake::default();
    let ret = run(&mut f, &mut g, 0, |cx| {
        jab_do(cx, &p, 2, 286, 292);
        jab_do(cx, &p, 3, 286, 292)
    });
    assert_eq!(ret, 1);
    assert_eq!(f.reqs(), vec![(286, Some(P), 0)]);
    let mut r = zero_sounds();
    r.skill3 = 610;
    let m = monster(&r);
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| {
        jab_do(cx, &m, 3, 286, 292);
        // cltstfunc 25: a monster plays its skill slot's voice (slot 2 →
        // Skill3); slot 4 or none → nothing; a player its dosound a.
        charge_start(cx, &m, 0, Some(2));
        charge_start(cx, &m, 0, Some(4));
        charge_start(cx, &m, 0, None);
        charge_start(cx, &p, 0, None);
        charge_start(cx, &p, 77, None);
    });
    assert_eq!(
        f.reqs(),
        vec![(292, Some(M), 0), (610, Some(M), 0), (77, Some(P), 0)]
    );
}

#[test]
fn prog_sound_functions() {
    let mut g = Globals::default();
    let mis = UnitKey::new(3, 40);
    let new = UnitKey::new(3, 41);
    let owner = UnitKey::new(MONSTER, 42);
    let mut f = Fake::default();
    f.unit_reqs.insert(mis, vec![(5, 2438), (4, 9)]);
    let mut data = 0;
    run(&mut f, &mut g, 0, |cx| {
        // 9: elapsed = max(P1, 1) − 2 only; the head handle is detached
        // with force first.
        missile_prog_fn9(cx, mis, 5, 8, 2438);
        missile_prog_fn9(cx, mis, 6, 8, 2438);
        // 29: elapsed 315 only, on the owner.
        missile_prog_fn29(cx, Some(owner), 314, 310, 451);
        missile_prog_fn29(cx, Some(owner), 315, 310, 451);
        // 47: a = 0 → no request, a := b; then a ≠ b → request.
        missile_prog_fn47(cx, mis, 2416, &mut data, 3);
        missile_prog_fn47(cx, mis, 2416, &mut data, 3);
        missile_prog_fn47(cx, mis, 2416, &mut data, 4);
        // 51: on the created missile.
        missile_prog_fn51(cx, None, 2458);
        missile_prog_fn51(cx, Some(new), 2458);
    });
    assert_eq!(
        f.log,
        vec![
            Call::Detach(5, mis, true),
            Call::Req(2438, Some(mis), 0, 0),
            Call::Req(451, Some(owner), 0, 0),
            Call::Req(2416, Some(mis), 0, 0),
            Call::Req(2458, Some(new), 0, 0),
        ]
    );
    assert_eq!(data, 4);
}

#[test]
fn mode_set_dispatch_and_order() {
    let mut g = Globals::default();
    let mut us = UnitSound::default();
    let mut u = player(0);
    u.weapon_hit_class = 5;
    u.speed = 256;
    let sk = SkillStart {
        stsound: 600,
        stsounddelay: false,
        ..SkillStart::default()
    };
    // A mode set uses the unit's current mode (explicit = 0): A1 → the
    // swing (280, delay 8) comes before the skill's start sound (600).
    u.mode = 7;
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| {
        mode_set(cx, &u, &mut us, u.mode, Some((&sk, true)))
    })
    .unwrap();
    assert_eq!(f.reqs(), vec![(280, Some(P), 8), (600, Some(P), 0)]);
    // The player machine's explicit calls (m = 3, 19) pass their own m:
    // 3 is not in the table, 19 (KB) is a hit/death mode, whatever the
    // unit's current mode.
    u.mode = 0;
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| mode_sound(cx, &u, &mut us, 3)).unwrap();
    assert!(f.log.is_empty());
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| mode_sound(cx, &u, &mut us, 19)).unwrap();
    assert!(!f.reqs().is_empty());
    // No skill: the mode sounds alone; objects and other types nothing.
    let mut f = Fake::default();
    let o = Unit::new(O, 1);
    run(&mut f, &mut g, 0, |cx| mode_set(cx, &o, &mut us, 1, None)).unwrap();
    assert!(f.log.is_empty());
}

#[test]
fn server_item_events_reach_player_events_and_93() {
    let mut g = Globals::default();
    let mut us = UnitSound::default();
    let p = player(0);
    let rec = &CLASS_RECORDS[0];
    let mut f = Fake::default();
    run(&mut f, &mut g, 1000, |cx| {
        for e in [1u16, 9, 11, 19, 20, 21, 22, 23, 24] {
            // Distinct C so the global speech guard never repeats.
            cx.c += 100;
            server_event(cx, &p, &mut us, e, extra()).unwrap();
        }
        server_event(cx, &Unit::new(O, 1), &mut us, 93, extra()).unwrap();
    });
    assert_eq!(
        f.made(),
        vec![
            (235, Some(P), 0),
            (9, Some(P), 0),
            (228, Some(P), 0),
            (rec.impossible, Some(P), 0),
            (rec.cantuseyet, Some(P), 0),
            (rec.needmana, Some(P), 0),
            (rec.needkey, Some(P), 0),
            (rec.cantcarry, Some(P), 0),
            (rec.notintown, Some(P), 0),
            (2553, Some(O), 0),
        ]
    );
}

#[test]
fn fixed_npc_lines() {
    use super::npc::{GUARD_HALT, NIHLATHAK_HURRYUP, WUSSIE_CHEER, WUSSIE_CLASS, WUSSIE_HELP_ME};
    assert_eq!(
        (
            WUSSIE_CHEER,
            WUSSIE_HELP_ME,
            WUSSIE_CLASS,
            GUARD_HALT,
            NIHLATHAK_HURRYUP
        ),
        (4603, 4607, 534, 3983, 4560)
    );
}

#[test]
fn dialog_table_lookup() {
    let table = NpcSpeech::spec();
    assert_eq!(table.rows().len(), 863);
    // Key 506 is listed twice (order 37 → 3,533, 38 → 3,534): the first
    // wins, 3,534 is never played.
    assert_eq!(table.sound(506), 3533);
    assert!(table.rows().iter().all(|r| r.sound != 0));
    // The key is 16 bits wide.
    assert_eq!(table.sound(0x1_0000 + 506), 3533);
    // Disabled → 0: no request, only the speech stop.
    let mut g = Globals::default();
    let mut st = DialogState::default();
    set_npc_speech_option(&mut st, 1);
    let npc = Unit::new(M, 148);
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| {
        dialog_line(cx, &mut st, table, &npc, Some(P), 506)
    });
    assert!(f.made().is_empty());
    set_npc_speech_option(&mut st, 0);
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| {
        dialog_line(cx, &mut st, table, &npc, Some(P), 506)
    });
    assert_eq!(f.made(), vec![(3533, Some(P), 5)]);
}

#[test]
fn steal_overlays_play_on_the_unit() {
    let mut g = Globals::default();
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| {
        impact_overlay(cx, M, 151);
        impact_overlay(cx, M, 152);
        impact_overlay(cx, M, 150);
    });
    assert_eq!(f.reqs(), vec![(396, Some(M), 0), (397, Some(M), 0)]);
}

#[test]
fn quake_loop_follows_the_shake_level() {
    assert_eq!(QuakeLoop::level(20.0), 255);
    assert_eq!(QuakeLoop::level(10.0), 127);
    assert_eq!(QuakeLoop::level(100.0), 255);
    assert_eq!(QuakeLoop::level(-3.0), 0);
    let mut g = Globals::default();
    let mut f = Fake::default();
    let mut q = QuakeLoop::default();
    // l = 0 with no request: nothing.
    run(&mut f, &mut g, 0, |cx| q.update(cx, 0.0));
    assert!(f.log.is_empty());
    // l > 0: request 452 on no unit, volume := l.
    run(&mut f, &mut g, 0, |cx| q.update(cx, 10.0));
    assert_eq!(f.log, vec![Call::Req(452, None, 0, 0), Call::Vol(1, 127)]);
    assert_eq!(q.handle, 1);
    // Toward a lower level by at most 6 per call.
    run(&mut f, &mut g, 0, |cx| q.update(cx, 0.0));
    assert_eq!(f.log[2..], [Call::Vol(1, 121)]);
    for _ in 0..20 {
        run(&mut f, &mut g, 0, |cx| q.update(cx, 0.0));
    }
    // 127 → 121 → … → 1 (21 steps of 6 from 127 reaches 1), then 0 stops.
    assert!(f.log.contains(&Call::Vol(1, 1)));
    run(&mut f, &mut g, 0, |cx| q.update(cx, 0.0));
    assert_eq!(f.log.last(), Some(&Call::Other("stop_handle")));
    assert_eq!(q.handle, 0);
    // Upward by at most 6 too.
    let mut f = Fake::default();
    let mut q = QuakeLoop::default();
    run(&mut f, &mut g, 0, |cx| q.update(cx, 1.0)); // l = 12
    run(&mut f, &mut g, 0, |cx| q.update(cx, 4.0)); // l = 51
    assert_eq!(f.log[1..], [Call::Vol(1, 12), Call::Vol(1, 18)]);
}

#[test]
fn panel_and_effect_sounds() {
    let mut g = Globals::default();
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, waypoint_row_chosen);
    assert_eq!(f.reqs(), vec![(2231, None, 0)]);
    // A step needs more than 50 ms; symbol i plays on the step where
    // counter − start = 1.
    let mut f = Fake::default();
    let mut p = InifussPanel {
        starts: [0, 1, 2, 3, 4],
        ..InifussPanel::default()
    };
    run(&mut f, &mut g, 0, |cx| {
        p.update(cx, 50); // 50 ms: not more than 50
        assert_eq!(p.counter, 0);
        p.update(cx, 51); // counter 1: symbol 0 (1 − 0 = 1)
        p.update(cx, 60); // too soon
        p.update(cx, 102); // counter 2: symbol 1
        p.update(cx, 153); // counter 3: symbol 2
    });
    assert_eq!(p.counter, 3);
    assert_eq!(f.reqs(), vec![(2671, None, 0); 3]);
    // Shake at frames left 150, request at 50.
    let mut f = Fake::default();
    let shake: Vec<bool> = run(&mut f, &mut g, 0, |cx| {
        [151u32, 150, 100, 50, 49]
            .iter()
            .map(|&l| diablo_appears(cx, l))
            .collect()
    });
    assert_eq!(shake, [false, true, false, false, false]);
    assert_eq!(f.reqs(), vec![(4638, None, 0)]);
}

// ---- hooks, frame events, sound identity, request lists, object functions ----

use super::hooks::{self, Effect};
use super::identity::{monsounds_row, sound_identity, DrawSubstitution, RecordInputs};

fn missiles(fx: &[Effect]) -> Vec<(i32, i32, i32)> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Missile { id, x, y } => Some((*id, *x, *y)),
            _ => None,
        })
        .collect()
}

#[test]
fn umod_fire_and_goboom_explode_at_frame_4() {
    assert_eq!(hooks::umod_hook_slot(9, 2), 2 + 5 * 9);
    assert_eq!(hooks::SOUND_PHASE, 2);
    let mut g = Globals::default();
    let mut u = Unit::new(M, 100);
    u.mode = 0;
    u.frame = 4 << 8;
    let pts = [(10, 20), (11, 21), (11, 19), (9, 21), (9, 19)];
    // Fire: five 117 then five 82 at the same points, then the request.
    let mut f = Fake::default();
    let mut fx = Vec::new();
    run(&mut f, &mut g, 0, |cx| {
        hooks::umod_phase2(cx, &u, 9, true, (10, 20), &mut |e| fx.push(e))
    });
    let mut want: Vec<(i32, i32, i32)> = pts.iter().map(|&(x, y)| (117, x, y)).collect();
    want.extend(pts.iter().map(|&(x, y)| (82, x, y)));
    assert_eq!(missiles(&fx), want);
    assert_eq!(f.reqs(), vec![(2458, Some(M), 0)]);
    // Goboom: only the five 82.
    let mut f = Fake::default();
    let mut fx = Vec::new();
    run(&mut f, &mut g, 0, |cx| {
        hooks::umod_phase2(cx, &u, 31, true, (10, 20), &mut |e| fx.push(e))
    });
    assert_eq!(
        missiles(&fx),
        pts.iter().map(|&(x, y)| (82, x, y)).collect::<Vec<_>>()
    );
    assert_eq!(f.reqs(), vec![(2458, Some(M), 0)]);
    // Not unique, mode ≠ 0 or frame ≠ 4: nothing.
    let mut f = Fake::default();
    let mut fx = Vec::new();
    let mut other = u;
    run(&mut f, &mut g, 0, |cx| {
        hooks::umod_phase2(cx, &other, 9, false, (0, 0), &mut |e| fx.push(e));
        other.mode = 1;
        hooks::umod_phase2(cx, &other, 9, true, (0, 0), &mut |e| fx.push(e));
        other.mode = 0;
        other.frame = 5 << 8;
        hooks::umod_phase2(cx, &other, 31, true, (0, 0), &mut |e| fx.push(e));
    });
    assert!(fx.is_empty() && f.log.is_empty());
    // A frame value of 4 that lasts several updates repeats it.
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| {
        for _ in 0..3 {
            hooks::umod_phase2(cx, &u, 9, true, (0, 0), &mut |_| {});
        }
    });
    assert_eq!(f.reqs().len(), 3);
}

#[test]
fn worms_have_no_test() {
    let mut g = Globals::default();
    // Umod 40: every update, whatever the mode and frame; the pregnant
    // remove hook does the same.
    let mut u = Unit::new(M, 100);
    u.mode = 5;
    u.frame = 99;
    let mut f = Fake::default();
    let mut fx = Vec::new();
    run(&mut f, &mut g, 0, |cx| {
        hooks::umod_phase2(cx, &u, 40, false, (7, 8), &mut |e| fx.push(e));
        hooks::worms(cx, &u, (7, 8), &mut |e| fx.push(e));
    });
    assert_eq!(
        missiles(&fx),
        vec![(117, 7, 8), (545, 7, 8), (117, 7, 8), (545, 7, 8)]
    );
    assert_eq!(f.reqs(), vec![(2458, Some(M), 0); 2]);
    // Other umods and other phases make no sound.
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| {
        hooks::umod_phase2(cx, &u, 3, true, (0, 0), &mut |_| {})
    });
    assert!(f.log.is_empty());
}

#[test]
fn monster_death_branches() {
    let mut g = Globals::default();
    let u = Unit::new(M, 453);
    // d8 = ((d + 4) >> 3) & 7 for the 64 directions.
    assert_eq!(
        (0..64).map(hooks::fold_direction).collect::<Vec<_>>(),
        (0..64u8).map(|d| ((d + 4) >> 3) & 7).collect::<Vec<_>>()
    );
    assert_eq!(hooks::fold_direction(60), 0);
    // minion1: voice by d8 & 3 before the mode set.
    for (dir, want) in [(0u8, 1308), (8, 1311), (16, 1314), (24, 1317), (32, 1308)] {
        let mut f = Fake::default();
        run(&mut f, &mut g, 0, |cx| {
            hooks::monster_death_before(cx, &u, 453, dir, (0, 0), &mut |_| {})
        });
        assert_eq!(f.reqs(), vec![(want, Some(M), 0)], "dir {dir}");
    }
    // suicideminion1: overlay 204, then 2,419.
    let mut f = Fake::default();
    let mut fx = Vec::new();
    run(&mut f, &mut g, 0, |cx| {
        hooks::monster_death_before(cx, &u, 461, 0, (0, 0), &mut |e| fx.push(e))
    });
    assert_eq!(fx, vec![Effect::Overlay(204)]);
    assert_eq!(f.reqs(), vec![(2419, Some(M), 0)]);
    // Vine creatures: missile 470 before, 790 after, tail skipped.
    for base in [425, 426, 427] {
        let mut f = Fake::default();
        let mut fx = Vec::new();
        let skipped = run(&mut f, &mut g, 0, |cx| {
            hooks::monster_death_before(cx, &u, base, 0, (3, 4), &mut |e| fx.push(e));
            assert!(cx.s.sound_on());
            hooks::monster_death_after(cx, &u, base)
        });
        assert_eq!(missiles(&fx), vec![(470, 3, 4)]);
        assert_eq!(f.reqs(), vec![(790, Some(M), 0)]);
        assert!(skipped);
    }
    // Other BaseIds: no request.
    let mut f = Fake::default();
    let skipped = run(&mut f, &mut g, 0, |cx| {
        hooks::monster_death_before(cx, &u, 100, 0, (0, 0), &mut |_| {});
        hooks::monster_death_after(cx, &u, 100)
    });
    assert!(f.log.is_empty() && !skipped);
}

#[test]
fn leap_landing_sound() {
    let mut g = Globals::default();
    assert!(hooks::leap_check_runs(0x100) && !hooks::leap_check_runs(0xFF));
    let u = Unit::new(P, 4);
    // In the air (h ≠ 0, not at the target): not landed, no sound.
    let mut f = Fake::default();
    let landed = run(&mut f, &mut g, 0, |cx| {
        hooks::leap_landing(cx, &u, false, 5, false, 2820, &mut |_| {})
    });
    assert!(!landed && f.log.is_empty());
    // Landed (h = 0, or at the target): dust overlay, the running
    // footstep, then 2,517 for a non-monster.
    for (h, at) in [(0, false), (5, true)] {
        let mut f = Fake::default();
        let mut fx = Vec::new();
        let landed = run(&mut f, &mut g, 0, |cx| {
            hooks::leap_landing(cx, &u, false, h, at, 2820, &mut |e| fx.push(e))
        });
        assert!(landed);
        assert_eq!(fx, vec![Effect::Overlay(80)]);
        assert_eq!(f.reqs(), vec![(2820, Some(P), 0), (2517, Some(P), 0)]);
    }
    // A monster (sandleaper1 or any): no 2,517; footstep id 0: nothing.
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| {
        hooks::leap_landing(cx, &u, true, 0, false, 0, &mut |_| {})
    });
    assert!(f.made().is_empty());
}

#[test]
fn spider_lay_sound() {
    let mut g = Globals::default();
    let u = Unit::new(M, 100);
    // d16 = [10, 8, 22, 20, 18, 16, 14, 12][d8].
    assert_eq!(
        (0..8)
            .map(|d8| hooks::spider_offset_index(d8 * 8))
            .collect::<Vec<_>>(),
        vec![10, 8, 22, 20, 18, 16, 14, 12]
    );
    // Needs state 22, a path with flag 0x08, and not a town room.
    for (st, flag, town) in [
        (false, true, false),
        (true, false, false),
        (true, true, true),
    ] {
        let mut f = Fake::default();
        let mut made = false;
        run(&mut f, &mut g, 0, |cx| {
            hooks::spider_lay(cx, &u, st, flag, town, &mut || {
                made = true;
                true
            })
        });
        assert!(!made && f.log.is_empty());
    }
    // 1,830 whether or not the missile was created.
    for created in [true, false] {
        let mut f = Fake::default();
        let mut made = 0;
        run(&mut f, &mut g, 0, |cx| {
            hooks::spider_lay(cx, &u, true, true, false, &mut || {
                made += 1;
                created
            })
        });
        assert_eq!(made, 1);
        assert_eq!(f.reqs(), vec![(1830, Some(M), 0)]);
    }
}

#[test]
fn animation_event_3() {
    use super::skills::{
        event3_attack, frame_event_after_advance, monster_generic_do, player_generic_do,
    };
    // Cleared, then the last crossed event in 1–4 is stored.
    assert_eq!(frame_event_after_advance(&[]), 0);
    assert_eq!(frame_event_after_advance(&[0, 0, 5]), 0);
    assert_eq!(frame_event_after_advance(&[1, 0, 3, 7]), 3);
    assert_eq!(frame_event_after_advance(&[4, 2]), 2);
    // Player.
    assert!(player_generic_do(2, true, false, false, 3));
    assert!(player_generic_do(2, true, false, false, 1));
    assert!(!player_generic_do(2, true, false, false, 4));
    assert!(!player_generic_do(1, true, false, false, 3));
    assert!(!player_generic_do(2, false, false, false, 3));
    assert!(!player_generic_do(2, true, true, false, 3));
    assert!(!player_generic_do(2, true, false, true, 3));
    // Monster.
    assert!(monster_generic_do(4, true, true));
    assert!(monster_generic_do(3, false, false));
    assert!(!monster_generic_do(3, true, false));
    assert!(!monster_generic_do(2, false, true));
    assert!(!monster_generic_do(0, false, false));
    // Event 3 only; delay argument 0.
    let mut g = Globals::default();
    let mut us = UnitSound::default();
    let mut p = player(0);
    p.weapon_hit_class = 5;
    p.speed = 256;
    let mut f = Fake::default();
    run(&mut f, &mut g, 40, |cx| {
        event3_attack(cx, &p, &mut us, 2).unwrap();
        event3_attack(cx, &p, &mut us, 3).unwrap();
    });
    assert_eq!(f.reqs(), vec![(280, Some(P), 0)]);
    assert_eq!(us.last_voice, 40);
    // A monster plays its attack slot (A1 → slot 1).
    let mut r = zero_sounds();
    r.attack1 = 700;
    r.att1prb = 100;
    let mut m = monster(&r);
    m.mode = 4;
    let mut f = Fake::default().rolls(&[0]);
    let mut us = UnitSound::default();
    run(&mut f, &mut g, 0, |cx| event3_attack(cx, &m, &mut us, 3)).unwrap();
    assert!(f.made().contains(&(700, Some(M), 0)));
}

#[test]
fn options_menu_sounds_are_ids_1_and_2() {
    use crate::audio::sound_table::sliders::{activate, after_move, EntryKind};
    assert_eq!(after_move(0, 1).sound, Some(1));
    assert_eq!(activate(EntryKind::Choice, 0, 2).sound, Some(1));
    assert_eq!(activate(EntryKind::Action, 0, 1).sound, Some(2));
    assert_eq!(UI_SOUNDS[0], (1, "cursor_pass"));
    assert_eq!(UI_SOUNDS[1], (2, "cursor_select"));
}

#[test]
fn sound_identity_substitution() {
    let all = |_: i32, _: u8| true;
    let none = |_: i32, _: u8| false;
    // Plain units keep their triple; a running sequence replaces the mode
    // for types 0 and 1 only.
    assert_eq!(sound_identity(0, 2, 1, None, None, &all), (0, 2, 1));
    assert_eq!(sound_identity(0, 2, 1, Some(7), None, &all), (0, 2, 7));
    assert_eq!(sound_identity(1, 100, 4, Some(5), None, &all), (1, 100, 5));
    assert_eq!(sound_identity(2, 27, 1, Some(5), None, &all), (2, 27, 1));
    // Only players are substituted: a monster with a substitution state
    // keeps its own triple.
    let wolf = DrawSubstitution {
        gfxtype: 1,
        gfxclass: 430,
    };
    assert_eq!(sound_identity(1, 7, 3, None, Some(wolf), &all), (1, 7, 3));
    // A wolf: monster class 430 with the player→monster mode map T1.
    let expect = [
        0, 1, 2, 15, 3, 1, 2, 4, 5, 6, 7, 4, 11, 8, 9, 10, 11, 12, 14, 13,
    ];
    for (pm, want) in expect.iter().enumerate() {
        assert_eq!(
            sound_identity(0, 2, pm as u8, None, Some(wolf), &all),
            (1, 430, *want),
            "player mode {pm}"
        );
    }
    // The mode bit of the class decides: clear bits fall back (A2 → A1 →
    // NU; RN → WL → NU; S3 → S1 → NU; DD → NU; BL → GH → NU).
    let bits_without = |clear: &'static [u8]| move |_c: i32, m: u8| !clear.contains(&m);
    assert_eq!(
        sound_identity(0, 2, 8, None, Some(wolf), &bits_without(&[5])),
        (1, 430, 4)
    );
    assert_eq!(
        sound_identity(0, 2, 8, None, Some(wolf), &bits_without(&[5, 4])),
        (1, 430, 1)
    );
    assert_eq!(
        sound_identity(0, 2, 3, None, Some(wolf), &bits_without(&[15])),
        (1, 430, 2)
    );
    assert_eq!(
        sound_identity(0, 2, 13, None, Some(wolf), &bits_without(&[8])),
        (1, 430, 1)
    );
    assert_eq!(
        sound_identity(0, 2, 17, None, Some(wolf), &bits_without(&[12])),
        (1, 430, 1)
    );
    assert_eq!(
        sound_identity(0, 2, 9, None, Some(wolf), &bits_without(&[6])),
        (1, 430, 3)
    );
    assert_eq!(
        sound_identity(0, 2, 0, None, Some(wolf), &none),
        (1, 430, 1)
    );
    // gfxtype 2 keeps a player, class from gfxclass, mode unchanged.
    let doppel = DrawSubstitution {
        gfxtype: 2,
        gfxclass: 0,
    };
    assert_eq!(sound_identity(0, 2, 9, None, Some(doppel), &all), (0, 0, 9));
}

#[test]
fn monsounds_record_choice() {
    let base = RecordInputs {
        raw_type: 1,
        class: 10,
        monstats_rows: 700,
        monsounds_rows: 400,
        monsound: 55,
        ..RecordInputs::default()
    };
    // MonSound in 0 … rows − 1.
    assert_eq!(monsounds_row(&base), Some(55));
    for bad in [-1, 400, 9999] {
        assert_eq!(
            monsounds_row(&RecordInputs {
                monsound: bad,
                ..base
            }),
            None
        );
    }
    assert_eq!(
        monsounds_row(&RecordInputs {
            monsound: 0,
            ..base
        }),
        Some(0)
    );
    // Class outside the row count → row 0.
    assert_eq!(monsounds_row(&RecordInputs { class: 700, ..base }), Some(0));
    assert_eq!(monsounds_row(&RecordInputs { class: -1, ..base }), Some(0));
    // Raw type 1 only. Superunique with a MonSound row wins; unique
    // or minion use UMonSound > 0; UMonSound ≤ 0 falls through.
    let su = RecordInputs {
        type_flags: 2,
        superunique_monsound: Some(80),
        umonsound: 90,
        ..base
    };
    assert_eq!(monsounds_row(&su), Some(80));
    let su0 = RecordInputs {
        superunique_monsound: Some(0),
        ..su
    };
    assert_eq!(monsounds_row(&su0), Some(55));
    let su_none = RecordInputs {
        superunique_monsound: None,
        ..su
    };
    assert_eq!(monsounds_row(&su_none), Some(55));
    for flags in [8, 0x10] {
        let u = RecordInputs {
            type_flags: flags,
            umonsound: 90,
            ..base
        };
        assert_eq!(monsounds_row(&u), Some(90));
        assert_eq!(monsounds_row(&RecordInputs { umonsound: 0, ..u }), Some(55));
        assert_eq!(
            monsounds_row(&RecordInputs { umonsound: -2, ..u }),
            Some(55)
        );
    }
    // The raw type decides, not the identity: a player (raw 0) never uses
    // the unique rows.
    let p = RecordInputs {
        raw_type: 0,
        type_flags: 8,
        umonsound: 90,
        ..base
    };
    assert_eq!(monsounds_row(&p), Some(55));

    // A transformed player (identity type 1) with no record gets
    // nothing, not the player rules.
    let mut g = Globals::default();
    let mut us = UnitSound::default();
    let mut u = player(0);
    u.identity_type = MONSTER;
    u.monsounds = None;
    u.weapon_hit_class = 5;
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| {
        for m in [3u8, 4, 12, 17, 19] {
            mode_sound(cx, &u, &mut us, m).unwrap();
        }
    });
    assert!(f.log.is_empty());
    // With a record, the monster rules read it (GH → hit sound).
    let mut r = zero_sounds();
    r.hitsound = 640;
    u.monsounds = Some(&r);
    let mut f = Fake::default();
    run(&mut f, &mut g, 0, |cx| mode_sound(cx, &u, &mut us, 3)).unwrap();
    assert!(f.made().contains(&(640, Some(P), 0)));
}

#[test]
fn request_list_walks_and_unit_free() {
    use super::{detach_all, request_in_group};
    // B = the group base, n = B's Group Size: ids B … B + n − 1.
    let mut f = Fake::default();
    f.bases.insert(137, 134);
    f.bases.insert(134, 134);
    f.bases.insert(133, 133);
    f.sizes.insert(133, 5);
    f.sizes.insert(134, 5);
    // A request of id 137 is in group 133's range 133–137 (the nested
    // group of 134 does not hide it) but 138 is not.
    f.unit_reqs.insert(M, vec![(1, 137)]);
    assert!(request_in_group(&f, M, 133));
    f.unit_reqs.insert(M, vec![(1, 138)]);
    assert!(!request_in_group(&f, M, 133));
    // A freed handle reads id 0: never in a group above 0.
    f.unit_reqs.insert(M, vec![(1, 0)]);
    assert!(!request_in_group(&f, M, 133));
    // The sound system off: false.
    f.unit_reqs.insert(M, vec![(1, 133)]);
    assert!(request_in_group(&f, M, 133));
    f.off = true;
    assert!(!request_in_group(&f, M, 133));
    // Unit free detaches every handle, newest first, without force.
    let mut f = Fake::default();
    f.unit_reqs.insert(M, vec![(3, 5), (2, 6), (1, 0)]);
    detach_all(&mut f, M, false);
    assert_eq!(
        f.log,
        vec![
            Call::Detach(3, M, false),
            Call::Detach(2, M, false),
            Call::Detach(1, M, false)
        ]
    );
}

#[test]
fn object_client_function_calls() {
    use super::objects::{
        client_function_sound, client_only_second_call, keeper_grunt, object_update_sounds,
    };
    let t = ObjectSounds::spec();
    let mut g = Globals::default();
    let mut door = Unit::new(O, 27);
    door.mode = 1;
    let fresh = || UnitSound {
        obj_seen: true,
        obj_prev_mode: 0,
        ..UnitSound::default()
    };
    // ClientFn ≤ 3 → the mode sound call; ≥ 4 → the client function
    // first, then the call only if it returned non-zero.
    for cf in 0..=3u8 {
        let (mut f, mut us) = (Fake::default(), fresh());
        run(&mut f, &mut g, 0, |cx| {
            object_update_sounds(cx, t, &door, &mut us, cf, false)
        })
        .unwrap();
        assert_eq!(f.reqs(), vec![(2568, Some(O), 0)], "ClientFn {cf}");
    }
    for (cf, res, n) in [(7u8, false, 0), (7, true, 1), (18, true, 1), (18, false, 0)] {
        let (mut f, mut us) = (Fake::default(), fresh());
        run(&mut f, &mut g, 0, |cx| {
            object_update_sounds(cx, t, &door, &mut us, cf, res)
        })
        .unwrap();
        assert_eq!(f.reqs().len(), n, "ClientFn {cf} result {res}");
    }
    // ≥ 19 is fatal.
    let (mut f, mut us) = (Fake::default(), fresh());
    assert_eq!(
        run(&mut f, &mut g, 0, |cx| object_update_sounds(
            cx, t, &door, &mut us, 19, true
        )),
        Err(TriggerError::ClientFn(19))
    );
    // ClientFn 4, 5, 6 call it themselves and return 0 (once per
    // update); 3 calls it and returns 1; case 0 returns 1 at once.
    for cf in 4..=6u8 {
        let (mut f, mut us) = (Fake::default(), fresh());
        run(&mut f, &mut g, 0, |cx| {
            object_update_sounds(cx, t, &door, &mut us, cf, true)
        })
        .unwrap();
        assert_eq!(f.reqs().len(), 1, "ClientFn {cf}");
    }
    let (mut f, mut us) = (Fake::default(), fresh());
    let ret = run(&mut f, &mut g, 0, |cx| {
        client_function_sound(cx, t, &door, &mut us, 3)
    })
    .unwrap();
    assert!(ret && f.reqs().len() == 1);
    let (mut f, mut us) = (Fake::default(), fresh());
    let ret = run(&mut f, &mut g, 0, |cx| {
        client_function_sound(cx, t, &door, &mut us, 0)
    })
    .unwrap();
    assert!(ret && f.log.is_empty());
    // A second call in the same mode requests nothing (U+0x74 =
    // the mode); the client-only walk's second call of a ClientFn 3–6
    // object is that same call.
    let (mut f, mut us) = (Fake::default(), fresh());
    run(&mut f, &mut g, 0, |cx| {
        object_update_sounds(cx, t, &door, &mut us, 0, false).unwrap();
        client_only_second_call(cx, t, &door, &mut us, 4).unwrap();
        // ClientFn 9 (clientsmoke) has no sound call of its own.
        assert!(!client_only_second_call(cx, t, &door, &mut us, 9).unwrap());
    });
    assert_eq!(f.reqs().len(), 1);
    // ClientFn 18 (keeper): wall-clock gated, two seed steps.
    let keeper = Unit::new(O, 568);
    let mut next = 1000u32;
    let mut f = Fake::default();
    let mut steps = vec![5u32, 119].into_iter();
    run(&mut f, &mut g, 0, |cx| {
        keeper_grunt(cx, &keeper, 1000, &mut next, &mut || unreachable!());
        keeper_grunt(cx, &keeper, 1001, &mut next, &mut || steps.next().unwrap());
    });
    assert_eq!(f.reqs(), vec![(2505, Some(O), 0)]);
    // s2 mod 60 = 59 → U+0xD4 = now + 59,000.
    assert_eq!(next, 1001 + 59_000);
    let mut f = Fake::default();
    let mut steps = vec![10u32, 0].into_iter();
    let mut next = 0;
    run(&mut f, &mut g, 0, |cx| {
        keeper_grunt(cx, &keeper, 5, &mut next, &mut || steps.next().unwrap())
    });
    assert!(f.log.is_empty() && next == 5);
}

#[test]
fn sound_init_sets_the_idle_gap_to_90() {
    let g = Globals::sound_init();
    assert_eq!(g.idle_gap, 90);
    assert_eq!(g.last_idle_any, 0);
    assert_eq!(g.speech_id, 0);
}

#[test]
fn the_group_walk_finds_the_first_handle_newest_first() {
    use super::first_in_group;
    let mut f = Fake::default();
    f.bases.insert(314, 312);
    f.bases.insert(313, 312);
    f.bases.insert(312, 312);
    f.sizes.insert(312, 3);
    // Newest first: handle 4 (id 313) before handle 2 (id 314); handle 9
    // (id 315) is past the group.
    f.unit_reqs.insert(M, vec![(9, 315), (4, 313), (2, 314)]);
    assert_eq!(first_in_group(&f, M, 314), Some(4));
    f.unit_reqs.insert(M, vec![(9, 315)]);
    assert_eq!(first_in_group(&f, M, 314), None);
    f.unit_reqs.insert(M, vec![(4, 313)]);
    f.off = true;
    assert_eq!(first_in_group(&f, M, 314), None);
}

#[test]
fn chicken_voice_keeps_its_one_second_gap_at_both_rates() {
    for hz in [25, 60] {
        let mut sounds = zero_sounds();
        sounds.cvtmo1 = 4;
        sounds.cvtsk1 = u32::MAX;
        sounds.cvttgt1 = 8;
        sounds.skill1 = 2692;
        let unit = monster(&sounds);
        let mut us = UnitSound::default();
        let mut globals = Globals::default();
        let mut fake = Fake {
            rate: d2_sim::tick::TickRate::new(hz).unwrap(),
            ..Fake::default()
        };
        run(&mut fake, &mut globals, hz - 1, |cx| {
            mode_sound(cx, &unit, &mut us, 4)
        })
        .unwrap();
        assert!(fake.log.is_empty());
        run(&mut fake, &mut globals, hz, |cx| {
            mode_sound(cx, &unit, &mut us, 4)
        })
        .unwrap();
        assert_eq!(fake.reqs(), vec![(2692, Some(M), 0)]);
    }
}

#[test]
fn repeated_player_speech_keeps_its_three_second_guard_at_both_rates() {
    for hz in [25, 60] {
        let unit = player(0);
        let mut globals = Globals::default();
        let mut fake = Fake {
            rate: d2_sim::tick::TickRate::new(hz).unwrap(),
            ..Fake::default()
        };
        run(&mut fake, &mut globals, 0, |cx| player_event(cx, &unit, 19)).unwrap();
        assert_eq!(fake.reqs().len(), 1);
        fake.log.clear();
        run(&mut fake, &mut globals, 3 * hz - 1, |cx| {
            player_event(cx, &unit, 19)
        })
        .unwrap();
        assert!(fake.log.is_empty());
        run(&mut fake, &mut globals, 3 * hz, |cx| {
            player_event(cx, &unit, 19)
        })
        .unwrap();
        assert_eq!(fake.reqs().len(), 1);
    }
}
