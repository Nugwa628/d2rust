//! Tests of the host frame loop (`crate::host`): the 1 ms base clock, the tick
//! driver's timing vectors (25 and 60 ticks per second, catch-up, an hour at
//! 60 Hz across the clock wrap), the order drain, tick, flush within
//! a frame, system and game message routing, the duplicate filter, flush ordering
//! and throttling, and direct sends. Part of the `d2-server` test suite (see
//! `mod.rs`); the game, session and clock are the fakes from `fakes.rs`.

use super::fakes::*;
use crate::dispatch::Outcome;
use crate::host::*;
use crate::seams::*;
use crate::transport::{Classified, Queue};
use d2_sim::tick::TickRate;

type TestHost = Host<FakeGame, TsvSizes, FakeSession, ManualClock>;

fn host() -> TestHost {
    let mut h = Host::new(
        FakeGame::with_player(0, ALIVE),
        TsvSizes::new(),
        FakeSession::default(),
        ManualClock(1000),
    );
    h.connect(0);
    h
}

fn rate(hz: u32) -> TickRate {
    TickRate::new(hz).unwrap()
}

fn at_base(last: u64) -> TickDriver {
    TickDriver::started(TickRate::BASE, last)
}

// One tick is 40 ms at the base rate (25 per second), 16 ms at the
// shortest at 60 Hz; the host takes the game's rate.
#[test]
fn tick_length() {
    assert_eq!(TickDriver::default().rate(), TickRate::BASE);
    assert_eq!(TickDriver::default().tick_ms(), 40);
    assert_eq!(TickDriver::new(rate(60)).tick_ms(), 16);
    assert_eq!(host().driver.rate(), TickRate::BASE);
    let mut game = FakeGame::with_player(0, ALIVE);
    game.rate = rate(60);
    let h = Host::new(
        game,
        TsvSizes::new(),
        FakeSession::default(),
        ManualClock(1000),
    );
    assert_eq!(h.driver.rate(), rate(60));
}

// Driver timing: no tick before 40 ms, catch-up runs several ticks, long lag is dropped.
#[test]
fn driver_vectors() {
    let at = |last, now, catch_up| {
        let mut d = at_base(last);
        let ran = d.poll(now, catch_up);
        (ran, d.last)
    };
    assert_eq!(at(1000, 1039, true), (false, 1000));
    assert_eq!(at(1000, 1040, true), (true, 1040));
    assert_eq!(at(1000, 1150, true), (true, 1110));
    assert_eq!(at(1000, 1150, false), (true, 1040));
    // After the catch-up the next call at the same time ticks again, then
    // the remaining lag is dropped.
    let mut d = at_base(1000);
    assert!(d.poll(1150, true));
    assert!(d.poll(1150, true));
    assert_eq!(d.last, 1150);
    assert!(!d.poll(1150, true));
}
// The first poll starts the clock without ticking; times past 2^31 ms tick
// as any other (the base clock has no 31-bit limit).
#[test]
fn driver_first_use() {
    let mut d = TickDriver::default();
    assert_eq!(d.next_tick_ms(), None);
    assert!(!d.poll(5000, true));
    assert_eq!(d.last, 5000);
    assert_eq!(d.next_tick_ms(), Some(5040));
    let mut d = at_base(0x7FFF_FFF0);
    assert!(d.poll(0x8000_0018, true));
    assert_eq!(d.last, 0x8000_0018);
    assert!(d.poll(0x1_0000_0040 + 0x18, false));
    assert_eq!(d.last, 0x8000_0040);
}

// A time before the last tick runs none and leaves the schedule as it was.
#[test]
fn driver_time_before_last() {
    let mut d = at_base(2000);
    assert!(!d.poll(1000, true));
    assert_eq!(d.last, 2000);
    assert!(d.poll(2040, true));
}

// A clock reading of 0 on first use makes the driver start on the next call.
#[test]
fn driver_zero_clock_reinitialises() {
    // The clock reads 0 on first use: `last` stays 0 and the next call
    // starts the schedule instead of ticking.
    let mut d = TickDriver::default();
    assert!(!d.poll(0, true));
    assert_eq!(d.last, 0);
    assert!(!d.poll(100, true));
    assert_eq!(d.last, 100);
    assert!(d.poll(140, true));
    assert_eq!(d.last, 140);
}

// The base clock extends the 32-bit reading: it starts at the first one,
// goes on across the 32-bit wrap, and holds while the clock steps back.
#[test]
fn base_clock_extends_the_reading() {
    let mut c = BaseClock::default();
    assert_eq!(c.ms(), 0);
    assert_eq!(c.advance(0xFFFF_FFF0), 0xFFFF_FFF0);
    assert_eq!(c.advance(0xFFFF_FFFF), 0xFFFF_FFFF);
    assert_eq!(c.advance(0x10), 0x1_0000_0010);
    // A step back holds the clock until the reading passes 0x10 again.
    assert_eq!(c.advance(0x8), 0x1_0000_0010);
    assert_eq!(c.advance(0x10), 0x1_0000_0010);
    assert_eq!(c.advance(0x14), 0x1_0000_0014);
    // A reading 2^31 ms or more ahead counts as a step back.
    assert_eq!(c.advance(0x8000_0014), 0x1_0000_0014);
    assert_eq!(c.advance(0x7FFF_FFFF), 0x1_7FFF_FFFF);
    assert_eq!(c.ms(), 0x1_7FFF_FFFF);
}

// The schedule before the driver took a rate and the base clock: a fixed
// 40 ms tick on the clock's reading with bit 31 masked, the lag carried
// forward clamped to one tick under catch-up.
fn base_rate_reference(last: &mut u32, now: u32, catch_up: bool) -> bool {
    let now = now & 0x7FFF_FFFF;
    if *last == 0 {
        *last = now;
    }
    let elapsed = now.wrapping_sub(*last) as i32;
    if elapsed < 40 {
        return false;
    }
    let mut excess = elapsed - 40;
    if catch_up && excess >= 40 {
        excess = 40;
    }
    *last = now.wrapping_sub(excess as u32);
    true
}

// At the base rate the host ticks exactly as the fixed 40 ms schedule does
// for any clock readings below 2^31 ms (24.8 days of uptime; past it the
// old schedule stalled), with and without catch-up, a clock that starts at
// 0 and one that steps back included.
#[test]
fn base_rate_matches_the_fixed_schedule() {
    for catch_up in [true, false] {
        let mut d = TickDriver::default();
        let mut c = BaseClock::default();
        let mut last = 0u32;
        // A deterministic walk of uneven frame times with long stalls and
        // steps back.
        let mut now = 0u32;
        let mut x = 0x1234_5678u32;
        for i in 0..20_000 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            now = match i % 997 {
                0 => now + 300 + x % 400,
                500 => now.saturating_sub(x % 200),
                _ => now + x % 23,
            };
            let want = base_rate_reference(&mut last, now, catch_up);
            assert_eq!(
                d.poll(c.advance(now), catch_up),
                want,
                "frame {i} now {now}"
            );
            assert_eq!(d.last, u64::from(last), "frame {i} now {now}");
        }
    }
}

// At 60 Hz tick n is due at ⌊n · 1000 / 60⌋ ms after the start: 60 ticks a
// second, lengths of 16 and 17 ms, and no drift over many seconds.
#[test]
fn sixty_hz_schedule_does_not_drift() {
    let start = 1000;
    let mut d = TickDriver::started(rate(60), start);
    let mut ticks = Vec::new();
    for now in start + 1..=start + 10_000 {
        if d.poll(now, true) {
            ticks.push(now - start);
        }
    }
    assert_eq!(ticks.len(), 600);
    for (i, &t) in ticks.iter().enumerate() {
        let n = i as u64 + 1;
        assert_eq!(t, n * 1000 / 60, "tick {n}");
    }
    let lengths: Vec<u64> = ticks.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(lengths.iter().all(|&l| l == 16 || l == 17));
    assert_eq!(&ticks[..3], &[16, 33, 50]);
}

// At 60 Hz catch-up carries at most one tick of lag; a late frame does not
// move the later ticks off the schedule when no lag is dropped.
#[test]
fn sixty_hz_catch_up() {
    // 20 ms late: the tick runs, the next is due 16 ms after the first
    // was scheduled plus 17 (at 33), and runs at 33.
    let mut d = TickDriver::started(rate(60), 1000);
    assert!(d.poll(1020, true));
    assert_eq!(d.last, 1016);
    assert!(!d.poll(1032, true));
    assert!(d.poll(1033, true));
    assert_eq!(d.last, 1033);
    // 150 ms late: one tick, then the next at once, then the lag is gone.
    let mut d = TickDriver::started(rate(60), 1000);
    assert!(d.poll(1150, true));
    assert_eq!(d.last, 1150 - 17);
    assert!(d.poll(1150, true));
    assert_eq!(d.last, 1150);
    assert!(!d.poll(1150, true));
    // Without catch-up every missed tick runs, one per call.
    let mut d = TickDriver::started(rate(60), 1000);
    let mut n = 0;
    while d.poll(1150, false) {
        n += 1;
    }
    assert_eq!(n, 150 * 60 / 1000);
}

// The 1 ms base clock keeps a 60 Hz game at real-time speed for an hour:
// uneven frames of 1 to 16 ms, the 32-bit clock wrapping half-way, and
// every tick on its schedule ⌊n · 1000 / 60⌋ ms after the start (lengths
// 16, 17, 17 ms, 50 ms per three ticks), 216,000 ticks in all.
#[test]
fn sixty_hz_hour_on_the_base_clock() {
    const HOUR_MS: u64 = 3_600_000;
    let start = u32::MAX - 1_800_000;
    let game = FakeGame {
        rate: rate(60),
        ..FakeGame::default()
    };
    let mut h = Host::new(
        game,
        TsvSizes::new(),
        FakeSession::default(),
        ManualClock(start),
    );
    let start = u64::from(start);
    assert!(!h.frame().unwrap().ticked);
    assert_eq!(h.base_ms(), start);
    let mut ticks = 0u64;
    let mut prev = start;
    let mut x = 0x9E37_79B9u32;
    while h.base_ms() < start + HOUR_MS {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        let gap = (1 + u64::from(x % 16)).min(start + HOUR_MS - h.base_ms());
        h.clock.0 = h.clock.0.wrapping_add(gap as u32);
        if h.frame().unwrap().ticked {
            ticks += 1;
            let at = start + ticks * 1000 / 60;
            assert_eq!(h.driver.last, at, "tick {ticks}");
            assert!(h.base_ms() - at < 16, "tick {ticks} late");
            let len = at - prev;
            assert_eq!(len, [17, 16, 17][(ticks % 3) as usize], "tick {ticks}");
            prev = at;
        }
    }
    assert_eq!(h.base_ms(), start + HOUR_MS);
    assert_eq!(ticks, 216_000);
    assert_eq!(h.game.frame, 216_000);
    assert_eq!(h.driver.next_tick_ms(), Some(start + HOUR_MS + 16));
}

// The flush throttle is the shortest tick of the game's rate: at 60 Hz an
// unforced flush 16 ms after the last one sends, one at 15 ms waits.
#[test]
fn flush_throttle_follows_the_rate() {
    let mut game = FakeGame::with_player(0, ALIVE);
    game.rate = rate(60);
    let mut h = Host::new(
        game,
        TsvSizes::new(),
        FakeSession::default(),
        ManualClock(1000),
    );
    h.connect(0);
    assert_eq!(h.flush(false, 1000), Ok((0, 0)));
    h.buffers.queue_for_test(0, &[0x01; 8]);
    assert_eq!(h.flush(false, 1015), Ok((0, 0)));
    assert_eq!(h.flush(false, 1016), Ok((1, 0)));
}

// Within a frame: drain messages, then tick if due, then flush.
#[test]
fn loop_order() {
    let mut h = host();
    // First frame: the driver starts its clock, no tick, no flush.
    h.send_game(0, &[0x01, 100, 0, 100, 0]).unwrap();
    let r = h.frame().unwrap();
    assert!(!r.ticked);
    assert_eq!(
        r.messages[0].handled,
        Outcome::Dispatched(ResultCode::Done).into()
    );
    // The handler ran before any tick, with frame 0.
    assert_eq!(h.game.handled.len(), 1);
    assert_eq!(h.game.point_state(0).unwrap().last_accept, 0);

    // A handler message queued without a tick stays buffered.
    h.game.handler_out = vec![(0, vec![0x0C; 9])];
    h.game.tick_out = vec![(0, vec![0x01; 8])];
    h.clock.0 = 1020;
    h.send_game(0, &[0x03, 100, 0, 101, 0]).unwrap();
    let r = h.frame().unwrap();
    assert!(!r.ticked);
    assert_eq!(h.buffers.buffers(0).unwrap().len(), 1);
    assert!(h.receive(0).is_empty());

    // Next frame: drain (handler message) → tick (tick message) → flush.
    h.clock.0 = 1040;
    h.send_game(0, &[0x01, 100, 0, 102, 0]).unwrap();
    let r = h.frame().unwrap();
    assert!(r.ticked);
    assert_eq!(h.game.frame, 1);
    assert_eq!(r.flushed_buffers, 1);
    assert_eq!(h.game.handled.len(), 3);
    let got: Vec<u8> = h.receive(0).iter().map(|m| m[0]).collect();
    assert_eq!(got, [0x0C, 0x0C, 0x01]);
    assert_eq!(h.record(0).unwrap().last_message_ms, 1040);
}

impl From<Outcome> for Handled {
    fn from(o: Outcome) -> Self {
        Handled::Game(o)
    }
}

// Messages that arrive between ticks run against the previous frame number.
#[test]
fn messages_between_ticks_run_with_the_previous_frame() {
    let mut h = host();
    h.frame().unwrap();
    h.clock.0 = 1040;
    assert!(h.frame().unwrap().ticked);
    // Three messages arrive before the next tick; all run at frame 1, in
    // arrival order, before frame 2.
    for y in [100u8, 101, 102] {
        h.send_game(0, &[0x01, 100, 0, y, 0]).unwrap();
    }
    h.clock.0 = 1080;
    let r = h.frame().unwrap();
    assert!(r.ticked);
    let ys: Vec<u8> = h.game.handled.iter().map(|(_, m, _)| m[3]).collect();
    assert_eq!(ys, [100, 101, 102]);
    assert_eq!(h.game.point_state(0).unwrap().last_accept, 1);
    assert_eq!(h.game.frame, 2);
}

// The drain routes every queue: game, system and admin messages.
#[test]
fn drain_routes_every_queue() {
    let mut h = host();
    let mut ff = vec![0xFF];
    ff.resize(16, 0);
    h.send_game(0, &[0x01, 100, 0, 100, 0]).unwrap();
    h.send_system(0, &[0x6B]).unwrap();
    h.send_system(0, &ff).unwrap();
    // Not in a game.
    h.connect(4);
    h.send_game(4, &[0x01, 100, 0, 100, 0]).unwrap();
    let r = h.frame().unwrap();
    let handled: Vec<(u32, u8, Handled)> = r
        .messages
        .iter()
        .map(|m| (m.client, m.id, m.handled))
        .collect();
    assert_eq!(
        handled,
        [
            (0, 0x6B, Handled::System),
            (0, 0x01, Outcome::Dispatched(ResultCode::Done).into()),
            (4, 0x01, Outcome::NotInGame.into()),
            (0, 0xFF, Handled::AdminIgnored),
        ]
    );
    assert_eq!(h.session.seen, [(0, vec![0x6B], 1)]);
}

// Session-owned system messages go to the game first, the session handler gets the rest.
#[test]
fn system_messages_reach_the_game_first() {
    let mut h = host();
    h.game.session_ids = vec![0x67, 0x6B];
    let mut create = vec![0x67];
    create.resize(46, 0);
    h.send_system(0, &create).unwrap();
    h.send_system(0, &[0x69]).unwrap();
    h.send_system(0, &[0x6B]).unwrap();
    let r = h.frame().unwrap();
    let ids: Vec<(u8, Handled)> = r.messages.iter().map(|m| (m.id, m.handled)).collect();
    assert_eq!(
        ids,
        [
            (0x67, Handled::System),
            (0x69, Handled::System),
            (0x6B, Handled::System)
        ]
    );
    // The game took 0x67 and 0x6B in drain order; 0x69 went to the
    // session handler.
    assert_eq!(
        h.game.session_seen,
        [(0, create.clone(), 46), (0, vec![0x6B], 1)]
    );
    assert_eq!(h.session.seen, [(0, vec![0x69], 1)]);
    // What the game queued in the drain leaves with the tick's flush.
    h.clock.0 += 40;
    assert!(h.frame().unwrap().ticked);
    assert_eq!(h.receive(0), [vec![0x02], vec![0x02]]);
}

// A mod game's wire carries a mod message each way: the C→S envelope goes
// to the game's mod rules (not the dispatcher), and their S→C envelope
// leaves with the tick's flush.
#[test]
fn a_mod_wire_carries_a_mod_message_each_way() {
    use d2_proto::envelope::{Wire, CLIENT_ID, SERVER_ID};
    let mut h = host();
    h.set_wire(Wire::Mod);
    h.game.mod_rules = true;
    assert_eq!(
        h.send_game(0, &[CLIENT_ID, 3, 0, 1, 2, 3]),
        Ok(Some(Classified::Queued(Queue::Game)))
    );
    let r = h.frame().unwrap();
    let ids: Vec<(u8, Handled)> = r.messages.iter().map(|m| (m.id, m.handled)).collect();
    assert_eq!(ids, [(CLIENT_ID, Handled::Mod(true))]);
    assert_eq!(h.game.mod_seen, [(0, vec![1, 2, 3])]);
    assert!(h.game.handled.is_empty(), "the dispatcher never sees it");
    h.clock.0 += 40;
    assert!(h.frame().unwrap().ticked);
    assert_eq!(h.receive(0), [vec![SERVER_ID, 3, 0, 3, 2, 1]]);
    // A game without mod rules declines it; nothing is sent back.
    h.game.mod_rules = false;
    h.send_game(0, &[CLIENT_ID, 0, 0]).unwrap();
    let r = h.frame().unwrap();
    assert_eq!(r.messages[0].handled, Handled::Mod(false));
    assert_eq!(h.game.mod_seen.len(), 1);
}

// On D2RUST's own wire the envelope id is refused like any unknown id,
// even when the game has mod rules.
#[test]
fn the_original_wire_refuses_a_mod_message() {
    use d2_proto::envelope::{Wire, CLIENT_ID};
    let mut h = host();
    assert_eq!(h.wire(), Wire::Original);
    h.game.mod_rules = true;
    assert_eq!(
        h.send_game(0, &[CLIENT_ID, 3, 0, 1, 2, 3]),
        Ok(Some(Classified::Invalid))
    );
    let r = h.frame().unwrap();
    assert!(r.messages.is_empty());
    assert!(h.game.mod_seen.is_empty());
}

// The duplicate filter drops a repeated game message but never a system message.
#[test]
fn duplicate_filter_on_the_host() {
    let mut h = host();
    let walk = [0x03, 0x10, 0x00, 0x20, 0x00];
    assert!(h.send_game(0, &walk).unwrap().is_some());
    h.clock.0 += 120;
    assert_eq!(h.send_game(0, &walk), Ok(None));
    // System messages are not filtered.
    assert!(h.send_system(0, &[0x6B]).is_ok());
    assert!(h.send_system(0, &[0x6B]).is_ok());
    assert_eq!(h.queues.len(crate::transport::Queue::System), 2);
}

// A recorded per-frame send clock replaces the host clock in the duplicate filter.
#[test]
fn recorded_send_clock_per_frame() {
    let mut h = host();
    let walk = [0x03, 0x10, 0x00, 0x20, 0x00];
    // frame 10 reads 5000, frame 15 on reads 5200: the host clock is ignored
    h.set_send_clock([(10, 5000), (15, 5200)].into_iter().collect());
    h.game.frame = 10;
    assert!(h.send_game(0, &walk).unwrap().is_some());
    h.game.frame = 14; // still 5000: inside the window
    h.clock.0 += 10_000;
    assert_eq!(h.send_game(0, &walk), Ok(None));
    h.game.frame = 15; // 5200: the window is over
    assert!(h.send_game(0, &walk).unwrap().is_some());
    // before the first recorded frame: the host clock
    h.game.frame = 3;
    assert!(h.send_game(0, &walk).unwrap().is_some());
    // a replayed send time comes first
    h.game.frame = 15;
    h.set_send_now(Some(h.clock.0 + 100));
    assert_eq!(h.send_game(0, &walk), Ok(None));
}

#[test]
fn flush_follows_the_client_list() {
    let mut h = host();
    h.connect(1);
    h.game.players.insert(1, h.game.players[&0].clone());
    // Client list [1, 0] (joined 0 then 1; joining clients are prepended).
    h.game.client_list = vec![1, 0];
    h.game.tick_out = vec![(0, vec![0x0C; 9]), (1, vec![0x01; 8]), (0, vec![0x01; 8])];
    h.frame().unwrap();
    h.clock.0 = 1040;
    let r = h.frame().unwrap();
    assert_eq!(r.flushed_buffers, 2);
    assert_eq!(h.receive(1), [vec![0x01; 8]]);
    assert_eq!(h.receive(0), [vec![0x0C; 9], vec![0x01; 8]]);
    // A client outside the client list is not flushed.
    h.connect(2);
    h.game.tick_out = vec![(2, vec![0x01; 8])];
    h.clock.0 = 1080;
    h.frame().unwrap();
    assert!(h.receive(2).is_empty());
    assert_eq!(h.buffers.buffers(2).unwrap().len(), 1);
}

// Direct sends leave immediately, ahead of messages still in the buffers.
#[test]
fn direct_sends_overtake_buffered_messages() {
    let mut h = host();
    h.game.handler_out = vec![(0, vec![0x0C; 9])];
    h.send_game(0, &[0x01, 100, 0, 100, 0]).unwrap();
    h.frame().unwrap();
    h.send_direct(0, &[0x06]).unwrap();
    h.send_direct(0, &[0xB0]).unwrap();
    h.clock.0 = 1040;
    h.frame().unwrap();
    let got: Vec<u8> = h.receive(0).iter().map(|m| m[0]).collect();
    // System list first, then the game list: direct 0x06 before 0x0C.
    assert_eq!(got, [0xB0, 0x06, 0x0C]);
}

// A message queued at the end of a tick leaves in the same frame's flush.
#[test]
fn a_tick_end_queue_leaves_in_the_same_frames_flush() {
    let mut h = host();
    h.frame().unwrap();
    h.receive(0);
    h.game.tick_out = vec![(0, vec![0x01; 8])];
    h.clock.0 = 1040;
    // The tick's message first, then the one queued at the tick end.
    let r = h
        .frame_with(|h| h.queue_now(0, &[0x07, 0, 4, 0xF0, 3, 0x28]).unwrap())
        .unwrap();
    assert!(r.ticked);
    let got: Vec<u8> = h.receive(0).iter().map(|m| m[0]).collect();
    assert_eq!(got, [0x01, 0x07]);
}

// Flushing is throttled to one per tick interval unless forced.
#[test]
fn flush_throttle_unless_forced() {
    let mut h = host();
    assert_eq!(h.flush(false, 1000), Ok((0, 0)));
    h.buffers.queue_for_test(0, &[0x01; 8]);
    assert_eq!(h.flush(false, 1039), Ok((0, 0)));
    assert_eq!(h.flush(false, 1040), Ok((1, 0)));
    h.buffers.queue_for_test(0, &[0x01; 8]);
    assert_eq!(h.flush(true, 1041), Ok((1, 0)));
}

impl crate::buffers::ClientBuffers {
    fn queue_for_test(&mut self, client: ClientId, msg: &[u8]) {
        MessageSink::queue(self, client, msg).unwrap();
    }
}

// The same messages before the same frame give the same result for any timing.
#[test]
fn same_messages_same_result_regardless_of_timing() {
    // Whatever the wall-clock gaps, the same messages before the same frame
    // give the same outcome.
    let play = |times: &[u32]| {
        let mut h = host();
        h.game.tick_out = vec![(0, vec![0x01; 8])];
        let mut log = Vec::new();
        for (i, &t) in times.iter().enumerate() {
            h.clock.0 = t;
            if i % 2 == 0 {
                h.send_game(0, &[0x01, 100, 0, 100 + i as u8, 0]).unwrap();
            }
            let r = h.frame().unwrap();
            log.push((r.ticked, h.game.frame, h.receive(0).len()));
        }
        (log, h.game.handled.clone())
    };
    let a = play(&[1000, 1040, 1080, 1120, 1160]);
    let b = play(&[1000, 1041, 1095, 1121, 1170]);
    assert_eq!(a, b);
}
