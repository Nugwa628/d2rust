//! The NPC intro table (0x91), the gossip index and the gossip
//! click, and the interact-NPC state.

use d2_sim::rng::Seed;

use super::msg_u32s;
use crate::ui::panel::ClientIntent;

/// Entries of the intro table.
pub const INTRO_COUNT: usize = 46;
/// Draws of the gossip index.
pub const GOSSIP_DRAWS: u32 = 10;

/// NPC classes whose no-introduction byte +0x14 is 1 (not 210; corrected
/// 2026-10-09 from the image).
pub const NO_INTRO: [u32; 5] = [146, 175, 176, 244, 265];
/// The class of the entry whose records are the array (intro entry 15).
pub const CLASS_725CB0: u32 = 201;
/// NPC classes whose byte +0x13 is 1.
pub const FLAG_13: [u32; 4] = [155, 210, 367, 521];

/// A 15-byte text record at +5 of an intro entry (other fields: Questions 4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GossipRecord {
    pub raw: [u8; 15],
}

impl GossipRecord {
    /// Text id, u16 +0.
    pub fn text(&self) -> u16 {
        u16::from_le_bytes([self.raw[0], self.raw[1]])
    }
    /// Flag byte +2.
    pub fn flag(&self) -> u8 {
        self.raw[2]
    }
    /// u32 +3 (the expected quest bit).
    pub fn quest_value(&self) -> u32 {
        u32::from_le_bytes([self.raw[3], self.raw[4], self.raw[5], self.raw[6]])
    }
    /// u32 +7 (the quest record).
    pub fn quest_record(&self) -> u32 {
        u32::from_le_bytes([self.raw[7], self.raw[8], self.raw[9], self.raw[10]])
    }
    /// The class byte +0x0B; 7 = any.
    pub fn class(&self) -> u8 {
        self.raw[0x0B]
    }
}

/// One entry of the intro table (0x16 bytes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntroEntry {
    /// NPC class u32 +0x00.
    pub class: u32,
    /// Act u8 +0x04.
    pub act: u8,
    /// Text records (pointer +0x05, count u32 +0x09).
    pub records: Vec<GossipRecord>,
    /// Gossip index u32 +0x0D.
    pub gossip_index: u32,
    /// Gossip heard u8 +0x11.
    pub gossip_heard: bool,
    /// Return greeting due u8 +0x12.
    pub return_due: bool,
    /// u8 +0x13.
    pub flag13: bool,
    /// No-introduction u8 +0x14.
    pub no_intro: bool,
    /// Greeting due u8 +0x15.
    pub greeting_due: bool,
    /// The entry's records are the class-201 record set.
    pub records_are_725cb0: bool,
}

impl IntroEntry {
    /// An entry with its static values: +0x15 = 1 in all, +0x14
    /// for 146, 175, 176, 244, 265, +0x13 for 155, 210, 367, 521.
    pub fn new(class: u32, act: u8, records: Vec<GossipRecord>) -> Self {
        Self {
            class,
            act,
            records,
            gossip_index: 0,
            gossip_heard: false,
            return_due: false,
            flag13: FLAG_13.contains(&class),
            no_intro: NO_INTRO.contains(&class),
            greeting_due: true,
            records_are_725cb0: class == CLASS_725CB0,
        }
    }
}

/// The greeting kind of the menu open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GreetingMode {
    /// Mode 0, the plain greeting (confirmed): its handle is not
    /// kept.
    Plain,
    /// Mode 2, "return".
    Return,
}

/// What a played greeting does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GreetingPlayed {
    /// C→S 0x4D [class u16].
    pub send_4d: Option<ClientIntent>,
    /// The greeting-sent flag:= 1.
    pub flag_7c0db4: bool,
}

/// The player facts of the gossip draws.
pub struct GossipCtx<'a> {
    /// The local player's class.
    pub class: u8,
    /// The quest bit test (flags, record, bit) on the client quest record.
    pub quest_bit: &'a dyn Fn(u32, u32) -> u32,
    /// = 1.
    pub game_quest12_bit13: bool,
}

/// Failure of the intro table update.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IntroError {
    /// The re-roll with no local player is fatal 0xFAB.
    #[error("gossip re-roll without a local player (fatal 0xFAB)")]
    NoLocalPlayer,
}

/// Gossip index: up to 10 draws on the local
/// player's unit seed; mask when the count is a power of two, else
/// modulo; count 0 → index 0. Each draw is stored in +0x0D.
/// is a record array (the records of the class-201 entry), not a list.
pub fn roll_gossip(entry: &mut IntroEntry, seed: &mut Seed, ctx: &GossipCtx<'_>) {
    let count = entry.records.len() as u32;
    for _ in 0..GOSSIP_DRAWS {
        let i = if count == 0 {
            0
        } else if count.is_power_of_two() {
            seed.mask(count)
        } else {
            seed.roll(count as i32)
        };
        entry.gossip_index = i;
        let Some(rec) = entry.records.get(i as usize) else {
            continue;
        };
        let class_ok = rec.class() == ctx.class || rec.class() == 7;
        let quest_ok =
            rec.flag() == 0 || (ctx.quest_bit)(rec.quest_record(), 0) == rec.quest_value();
        if i >= 2 && class_ok && quest_ok {
            // Only on the quest path (byte +2 != 0 and the test passed).
            if rec.flag() != 0
                && entry.records_are_725cb0
                && rec.text() == 0xFF
                && ctx.game_quest12_bit13
            {
                entry.gossip_index = 2;
            }
            return;
        }
    }
    entry.gossip_index = 2;
}

/// The outcome of the "gossip" option.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GossipOutcome {
    /// The NPC is gone: menu state 0, the cursor reset, the interaction ends and
    /// state 8 closes.
    Ended,
    /// The talk flag:= 1, end callback, the topic box is freed; the text
    /// played is `text`.
    Play { text: Option<u16> },
}

/// The intro table and its globals.
#[derive(Clone, Debug, Default)]
pub struct IntroTable {
    pub entries: Vec<IntroEntry>,
    /// The first-time re-roll was done.
    pub rerolled: bool,
    /// A talk is running.
    pub talk_flag: bool,
    /// A greeting was sent (C→S 0x4D).
    pub flag_7c0db4: bool,
    /// A greeting handle is held (a plain greeting's is
    /// dropped again).
    pub handle_7c0db8: bool,
}

impl IntroTable {
    pub fn new(entries: Vec<IntroEntry>) -> Self {
        Self {
            entries,
            ..Default::default()
        }
    }

    /// The first entry whose class is `class`.
    pub fn index_of(&self, class: u32) -> Option<usize> {
        self.entries.iter().position(|e| e.class == class)
    }

    /// 0x91: +0x12:= 1.
    pub fn on_0x91(&mut self, class: u32) {
        if let Some(i) = self.index_of(class) {
            self.entries[i].return_due = true;
        }
    }

    /// Game start / exit reset: +0x11 and +0x12 of
    /// all cleared.
    pub fn reset(&mut self) {
        for e in &mut self.entries {
            e.gossip_heard = false;
            e.return_due = false;
        }
    }

    /// The NPC menu open (first open of a talk, after the
    /// menu is built): the greeting to request, `None` when the
    /// entry's +0x15 is clear. A class with no entry greets in mode 0.
    pub fn menu_open(&mut self, class: u32) -> Option<GreetingMode> {
        let Some(i) = self.index_of(class) else {
            return Some(GreetingMode::Plain);
        };
        let e = &mut self.entries[i];
        let mut mode = GreetingMode::Plain;
        if e.return_due {
            e.return_due = false;
            mode = GreetingMode::Return;
        }
        if !e.greeting_due {
            return None;
        }
        e.greeting_due = false;
        Some(mode)
    }

    /// The greeting gave a sound and played: mode 2
    /// sends C→S 0x4D [class u16] and sets the greeting-sent flag; mode 0 drops its
    /// handle. With no sound the caller does not call
    /// this: nothing is sent.
    pub fn greeting_played(&mut self, mode: GreetingMode, class: u32) -> GreetingPlayed {
        match mode {
            GreetingMode::Return => {
                self.flag_7c0db4 = true;
                GreetingPlayed {
                    send_4d: Some(ClientIntent(vec![0x4D, class as u8, (class >> 8) as u8])),
                    flag_7c0db4: true,
                }
            }
            GreetingMode::Plain => {
                self.handle_7c0db8 = false;
                GreetingPlayed {
                    send_4d: None,
                    flag_7c0db4: false,
                }
            }
        }
    }

    /// The player leaves town (from the room-change handler when the town flag
    /// goes from 1 to 0): +0x15 re-armed for all; entries with +0x11 = 1 get
    /// +0x11:= 0 and a new gossip index. Returns whether an active interaction
    /// with a present NPC ends.
    pub fn leave_town(
        &mut self,
        town_before: bool,
        town_now: bool,
        interaction_with_present_npc: bool,
        player: Option<(&mut Seed, &GossipCtx<'_>)>,
    ) -> Result<bool, IntroError> {
        if !(town_before && !town_now) {
            return Ok(false);
        }
        for e in &mut self.entries {
            e.greeting_due = true;
        }
        let mut player = player;
        for e in &mut self.entries {
            if !e.gossip_heard {
                continue;
            }
            e.gossip_heard = false;
            let (seed, ctx) = player.as_mut().ok_or(IntroError::NoLocalPlayer)?;
            roll_gossip(e, seed, ctx);
        }
        Ok(interaction_with_present_npc)
    }

    /// "gossip": the entry's gossip index, as re-rolled the first time.
    pub fn gossip_click(
        &mut self,
        entry: usize,
        npc_present: bool,
        seed: &mut Seed,
        ctx: &GossipCtx<'_>,
    ) -> GossipOutcome {
        self.click_with_index(entry, None, npc_present, seed, ctx)
    }

    /// The gossip body: the presence test (absent: the interaction ends), talk
    /// flag, +0x11:= 1 and the first-time re-roll of every entry. `index`
    /// `None`: the entry's +0x0D after the re-roll (gossip); `Some(i)`: `i` is
    /// kept and +0x0D is not read for the text (introduction). The text is u16
    /// +0 of record `index` modulo the count.
    pub fn click_with_index(
        &mut self,
        entry: usize,
        index: Option<u32>,
        npc_present: bool,
        seed: &mut Seed,
        ctx: &GossipCtx<'_>,
    ) -> GossipOutcome {
        if !npc_present || entry >= self.entries.len() {
            return GossipOutcome::Ended;
        }
        self.talk_flag = true;
        self.entries[entry].gossip_heard = true;
        if !self.rerolled {
            for e in &mut self.entries {
                roll_gossip(e, seed, ctx);
            }
            self.rerolled = true;
        }
        let e = &self.entries[entry];
        let i = index.unwrap_or(e.gossip_index) as usize;
        let text = (!e.records.is_empty()).then(|| e.records[i % e.records.len()].text());
        GossipOutcome::Play { text }
    }

    /// The "introduction" index: `player` = the
    /// local player's class (−1 without one). Malah (513) with a class-4
    /// player → 15, Nihlathak (514) with class 2 → 11, Qual-Kehk (515)
    /// with class 3 → 10; otherwise 1 when `player` equals the class
    /// field (u32 +0x0B) of the entry's text record 1, else 0.
    pub fn introduction_index(&self, entry: usize, npc_class: Option<u32>, player: i32) -> u32 {
        match (npc_class, player) {
            (Some(513), 4) => return 15,
            (Some(514), 2) => return 11,
            (Some(515), 3) => return 10,
            _ => {}
        }
        let class_field = self
            .entries
            .get(entry)
            .and_then(|e| e.records.get(1))
            .map(|r| u32::from_le_bytes([r.raw[11], r.raw[12], r.raw[13], r.raw[14]]));
        u32::from(class_field.is_some_and(|c| i64::from(c) == i64::from(player)))
    }
}

/// The interact NPC: active flag, GUID,
/// class. UI state; the model half must look it up at
/// delivery (cross-file request).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InteractNpc {
    pub active: bool,
    pub guid: u32,
    pub class: u32,
}

/// The model writes the interaction's end makes besides clearing the interaction
/// (cross-file request): the NPC's unit flag +0xC4 |= 2 and the local player's
/// data fields +0x150…+0x15C:= 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InteractionEnd {
    pub npc_guid: u32,
    pub npc_flag_c4_or: u32,
    pub clear_player_fields: bool,
}

impl InteractNpc {
    /// Active:= 1, GUID:= unit +0x0C, class:= unit +4; no unit → active:= 0.
    /// Called from the menu build and the 0x28 handler.
    pub fn from_unit(&mut self, unit: Option<(u32, u32)>) {
        match unit {
            Some((guid, class)) => {
                *self = Self {
                    active: true,
                    guid,
                    class,
                }
            }
            None => self.active = false,
        }
    }

    /// The NPC menu open: active:= 1, GUID, class; active:= 0 when state 8
    /// cannot open (after C→S 0x30).
    pub fn menu_open(&mut self, unit: (u32, u32), state8_opens: bool) -> Option<ClientIntent> {
        self.from_unit(Some(unit));
        if state8_opens {
            None
        } else {
            self.active = false;
            Some(msg_u32s(0x30, &[1, unit.0]))
        }
    }

    /// The 0x28 NPC path: active:= 1, GUID, class.
    pub fn from_0x28(&mut self, unit: (u32, u32)) {
        self.from_unit(Some(unit));
    }

    /// The interaction end: active:= 0 on every end path; returns the model
    /// writes to request.
    pub fn end(&mut self) -> Option<InteractionEnd> {
        let was = self.active;
        self.active = false;
        was.then_some(InteractionEnd {
            npc_guid: self.guid,
            npc_flag_c4_or: 2,
            clear_player_fields: true,
        })
    }

    /// Leaving town: active:= 0.
    pub fn leave_town(&mut self) {
        self.active = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(text: u16, flag: u8, qv: u32, qr: u32, class: u8) -> GossipRecord {
        let mut raw = [0u8; 15];
        raw[0..2].copy_from_slice(&text.to_le_bytes());
        raw[2] = flag;
        raw[3..7].copy_from_slice(&qv.to_le_bytes());
        raw[7..11].copy_from_slice(&qr.to_le_bytes());
        raw[0x0B] = class;
        GossipRecord { raw }
    }

    fn table() -> IntroTable {
        IntroTable::new(vec![
            IntroEntry::new(148, 1, vec![rec(1, 0, 0, 0, 7); 4]),
            IntroEntry::new(155, 1, vec![]),
            IntroEntry::new(244, 2, vec![]),
        ])
    }

    fn ctx<'a>(q: &'a dyn Fn(u32, u32) -> u32) -> GossipCtx<'a> {
        GossipCtx {
            class: 1,
            quest_bit: q,
            game_quest12_bit13: false,
        }
    }

    // The static values (the class list and text records are data
    // the install gives; the table is not embedded).
    #[test]
    fn static_values() {
        let t = table();
        assert_eq!(INTRO_COUNT, 46);
        assert!(t.entries.iter().all(|e| e.greeting_due));
        assert!(t.entries[2].no_intro && !t.entries[0].no_intro);
        assert!(t.entries[1].flag13 && !t.entries[2].flag13);
        assert!(IntroEntry::new(210, 0, vec![]).flag13);
        // Class 210 has +0x13 only: it gets the introduction topic.
        assert!(!IntroEntry::new(210, 0, vec![]).no_intro);
    }

    // Test vectors "0x91 slot 148, +0x15 = 1" and "same, +0x15 = 0".
    #[test]
    fn menu_open_greeting_and_4d() {
        let mut t = table();
        t.on_0x91(148);
        assert!(t.entries[0].return_due);
        let m = t.menu_open(148);
        assert_eq!(m, Some(GreetingMode::Return));
        let o = t.greeting_played(GreetingMode::Return, 148);
        assert_eq!(o.send_4d, Some(ClientIntent(vec![0x4D, 0x94, 0x00])));
        assert!(o.flag_7c0db4 && t.flag_7c0db4);
        assert!(!t.entries[0].return_due && !t.entries[0].greeting_due);
        // +0x12 set but +0x15 clear: the flag is cleared, nothing plays.
        t.on_0x91(148);
        assert_eq!(t.menu_open(148), None);
        assert!(!t.entries[0].return_due);
        // No entry of the class: the plain greeting (mode 0).
        assert_eq!(t.menu_open(999), Some(GreetingMode::Plain));
        // +0x12 clear, +0x15 set: the plain greeting, no 0x4D.
        assert_eq!(t.menu_open(155), Some(GreetingMode::Plain));
        let o = t.greeting_played(GreetingMode::Plain, 155);
        assert_eq!((o.send_4d, o.flag_7c0db4), (None, false));
        // Return with no sound: greeting_played is never called; the flag
        // stays as it was.
        let mut t = table();
        t.on_0x91(148);
        assert_eq!(t.menu_open(148), Some(GreetingMode::Return));
        assert!(!t.flag_7c0db4);
        // Writers: the game start / exit reset clears +0x11 and +0x12.
        t.entries[2].gossip_heard = true;
        t.on_0x91(244);
        t.reset();
        assert!(t.entries.iter().all(|e| !e.gossip_heard && !e.return_due));
    }

    #[test]
    fn leaving_town_rearms_and_rerolls() {
        let mut t = table();
        for e in &mut t.entries {
            e.greeting_due = false;
        }
        t.entries[0].gossip_heard = true;
        let mut seed = Seed::init_low(5);
        let q = |_: u32, _: u32| 0;
        let c = ctx(&q);
        // Town flag not 1 → 0: nothing.
        assert!(!t
            .leave_town(true, true, true, Some((&mut seed, &c)))
            .unwrap());
        assert!(!t.entries[0].greeting_due);
        let end = t
            .leave_town(true, false, true, Some((&mut seed, &c)))
            .unwrap();
        assert!(end);
        assert!(t.entries.iter().all(|e| e.greeting_due));
        assert!(!t.entries[0].gossip_heard);
        assert!(t.entries[0].gossip_index >= 2);
        // No active interaction → nothing ends.
        assert!(!t.leave_town(true, false, false, None).unwrap());
        // The re-roll with no local player is fatal 0xFAB.
        t.entries[0].gossip_heard = true;
        assert_eq!(
            t.leave_town(true, false, false, None),
            Err(IntroError::NoLocalPlayer)
        );
    }

    #[test]
    fn gossip_index_draws() {
        let q = |_: u32, _: u32| 0;
        // count 4 (power of two): mask(4) of the seed's steps; the first
        // draw ≥ 2 with a matching class is kept.
        let mut e = IntroEntry::new(148, 1, vec![rec(11, 0, 0, 0, 7); 4]);
        let mut s = Seed::init_low(1);
        let mut probe = s;
        let mut kept = None;
        for _ in 0..10 {
            let i = probe.mask(4);
            if i >= 2 {
                kept = Some(i);
                break;
            }
        }
        roll_gossip(&mut e, &mut s, &ctx(&q));
        assert_eq!(Some(e.gossip_index), kept.or(Some(2)));
        assert_eq!(s, probe);
        // Non power of two count: modulo (roll).
        let mut e = IntroEntry::new(148, 1, vec![rec(11, 0, 0, 0, 7); 5]);
        let mut s = Seed::init_low(9);
        let mut probe = s;
        let want = (0..10)
            .map(|_| probe.roll(5))
            .find(|&i| i >= 2)
            .unwrap_or(2);
        roll_gossip(&mut e, &mut s, &ctx(&q));
        assert_eq!(e.gossip_index, want);
        // A record of another class is never kept: after 10 rejected
        // draws the index is 2, and 10 steps were taken.
        let mut e = IntroEntry::new(148, 1, vec![rec(11, 0, 0, 0, 3); 8]);
        let mut s = Seed::init_low(3);
        let mut probe = s;
        for _ in 0..10 {
            probe.mask(8);
        }
        roll_gossip(&mut e, &mut s, &ctx(&q));
        assert_eq!((e.gossip_index, s), (2, probe));
        // Count 0 → index 0 without a draw; rejected → 2.
        let mut e = IntroEntry::new(148, 1, vec![]);
        let mut s = Seed::init_low(3);
        roll_gossip(&mut e, &mut s, &ctx(&q));
        assert_eq!((e.gossip_index, s), (2, Seed::init_low(3)));
        // The quest test: flag ≠ 0 needs quest_bit(record) == value.
        let quest = |r: u32, _b: u32| u32::from(r == 9);
        let mut e = IntroEntry::new(148, 1, vec![rec(11, 1, 0, 9, 7); 4]);
        let mut s = Seed::init_low(1);
        roll_gossip(&mut e, &mut s, &ctx(&quest));
        assert_eq!(e.gossip_index, 2);
        let mut e = IntroEntry::new(148, 1, vec![rec(11, 1, 1, 9, 7); 4]);
        let mut s = Seed::init_low(1);
        let mut probe = s;
        let want = (0..10)
            .map(|_| probe.mask(4))
            .find(|&i| i >= 2)
            .unwrap_or(2);
        roll_gossip(&mut e, &mut s, &ctx(&quest));
        assert_eq!(e.gossip_index, want);
        // Class 201 (records are): a kept record 2 (text 255, flag 1) gives
        // index 2 with and without game quest 12 bit 13.
        for bit in [false, true] {
            let mut recs = vec![rec(1, 0, 0, 0, 7); 10];
            recs[2] = rec(255, 1, 0, 9, 7);
            let mut e = IntroEntry::new(201, 1, recs);
            assert!(e.records_are_725cb0);
            let mut s = Seed::init_low(1);
            let mut c = ctx(&q);
            c.game_quest12_bit13 = bit;
            roll_gossip(&mut e, &mut s, &c);
            assert!(e.gossip_index >= 2);
            // Force the draw onto record 2: a one-record-eligible array.
            let mut recs = vec![rec(1, 0, 0, 0, 3); 4];
            recs[2] = rec(255, 1, 0, 9, 7);
            let mut e = IntroEntry::new(201, 1, recs);
            let quest = |r: u32, _b: u32| u32::from(r != 9);
            let c2 = GossipCtx {
                quest_bit: &quest,
                game_quest12_bit13: bit,
                ..ctx(&q)
            };
            let mut s = Seed::init_low(1);
            roll_gossip(&mut e, &mut s, &c2);
            assert_eq!(e.gossip_index, 2);
        }
    }

    #[test]
    fn gossip_click_plays_and_rerolls_once() {
        let q = |_: u32, _: u32| 0;
        let c = ctx(&q);
        let mut t = table();
        t.entries[0].records = (0..4).map(|i| rec(100 + i, 0, 0, 0, 7)).collect();
        let mut s = Seed::init_low(1);
        // NPC gone: the interaction ends.
        assert_eq!(t.gossip_click(0, false, &mut s, &c), GossipOutcome::Ended);
        assert!(!t.talk_flag && !t.rerolled);
        let GossipOutcome::Play { text } = t.gossip_click(0, true, &mut s, &c) else {
            panic!()
        };
        assert!(t.talk_flag && t.rerolled && t.entries[0].gossip_heard);
        let idx = t.entries[0].gossip_index as usize;
        assert_eq!(text, Some(100 + (idx % 4) as u16));
        // The second click re-rolls nothing.
        let seed_after = s;
        t.gossip_click(0, true, &mut s, &c);
        assert_eq!(s, seed_after);
        // An entry without records plays nothing.
        assert_eq!(
            t.gossip_click(1, true, &mut s, &c),
            GossipOutcome::Play { text: None }
        );
    }

    #[test]
    fn introduction_picks_the_index_and_runs_the_gossip_body() {
        let q = |_: u32, _: u32| 0;
        let c = ctx(&q);
        let mut t = table();
        t.entries[0].records = (0..16).map(|i| rec(200 + i, 0, 0, 0, 7)).collect();
        // Malah (513) with a barbarian (4) → record 15; Nihlathak (514) with
        // class 2 → 11; Qual-Kehk (515) with class 3 → 10.
        assert_eq!(t.introduction_index(0, Some(513), 4), 15);
        assert_eq!(t.introduction_index(0, Some(514), 2), 11);
        assert_eq!(t.introduction_index(0, Some(515), 3), 10);
        // Record 1's class field: 7 (any) never equals a player class, so 0;
        // a record whose class is the player's gives 1.
        assert_eq!(t.introduction_index(0, Some(148), 3), 0);
        t.entries[0].records[1] = rec(201, 0, 0, 0, 3);
        assert_eq!(t.introduction_index(0, Some(148), 3), 1);
        assert_eq!(t.introduction_index(0, Some(148), 2), 0);
        // No player (−1): index 0.
        assert_eq!(t.introduction_index(0, None, -1), 0);
        // The click plays that record, sets +0x11 and re-rolls once, and
        // leaves +0x0D (the rolled index) alone.
        let mut s = Seed::init_low(1);
        let GossipOutcome::Play { text } = t.click_with_index(0, Some(15), true, &mut s, &c) else {
            panic!()
        };
        assert_eq!(text, Some(215));
        assert!(t.entries[0].gossip_heard && t.rerolled && t.talk_flag);
        assert_ne!(t.entries[0].gossip_index, 99);
        assert_eq!(
            t.click_with_index(0, Some(1), false, &mut s, &c),
            GossipOutcome::Ended
        );
    }

    #[test]
    fn interact_npc_writers() {
        let mut n = InteractNpc::default();
        // With a unit: active, GUID, class; no unit: active 0.
        n.from_unit(Some((7, 148)));
        assert_eq!(
            n,
            InteractNpc {
                active: true,
                guid: 7,
                class: 148
            }
        );
        n.from_unit(None);
        assert!(!n.active);
        // The menu open: state 8 cannot open → active 0 after C→S 0x30.
        let m = n.menu_open((9, 150), true);
        assert!(m.is_none() && n.active && n.guid == 9);
        let m = n.menu_open((9, 150), false).unwrap();
        assert_eq!(m, ClientIntent(vec![0x30, 1, 0, 0, 0, 9, 0, 0, 0]));
        assert!(!n.active);
        // 0x28 path.
        n.from_0x28((3, 5));
        assert_eq!((n.active, n.guid, n.class), (true, 3, 5));
        // The interaction end clears it and asks for the model writes.
        let e = n.end().unwrap();
        assert_eq!(
            e,
            InteractionEnd {
                npc_guid: 3,
                npc_flag_c4_or: 2,
                clear_player_fields: true
            }
        );
        assert!(!n.active);
        assert!(n.end().is_none());
        // Leaving town.
        n.from_0x28((3, 5));
        n.leave_town();
        assert!(!n.active);
    }
}
