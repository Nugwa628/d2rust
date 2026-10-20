//! Tests of the packet recorder on the host (`crate::packets`): recording does not
//! change the game, and the record kinds and their order are pinned. Part of the
//! `d2-server` test suite (see `mod.rs`).
//! The D2RUST packet recorder on the host: the game runs the same with it
//! on, and the records come out in a fixed set of kinds and order.

use super::fakes::*;
use crate::host::*;
use crate::packets::{PacketLines, PacketLog};

type TestHost = Host<FakeGame, TsvSizes, FakeSession, ManualClock>;

fn host() -> TestHost {
    let mut h = Host::new(
        FakeGame::with_player(0, ALIVE),
        TsvSizes::new(),
        FakeSession::default(),
        ManualClock(1000),
    );
    h.connect(0);
    h.game.session_ids = vec![0x67];
    h.game.handler_out = vec![(0, vec![0x0C; 9])];
    h.game.tick_out = vec![(0, vec![0x01; 8]), (0, vec![0x15; 11])];
    h
}

/// A fixed session: create request, a point message, ticks, a direct
/// send. Returns everything observable of the game and the transport.
fn run(h: &mut TestHost) -> Vec<String> {
    let mut seen = Vec::new();
    let mut create = vec![0x67];
    create.resize(46, 0);
    h.send_system(0, &create).unwrap();
    for step in 0..6u32 {
        if step == 1 {
            h.send_game(0, &[0x01, 100, 0, 102, 0]).unwrap();
            // Filtered duplicate: a client_send with no client_out.
            h.send_game(0, &[0x01, 100, 0, 102, 0]).unwrap();
        }
        if step == 3 {
            h.send_direct(0, &[0xB0]).unwrap();
        }
        let r = h.frame().unwrap();
        seen.push(format!("{r:?}"));
        seen.push(format!("{:?}", h.receive(0)));
        h.clock.0 += 40;
    }
    seen.push(format!(
        "{:?} {:?} {:?} {}",
        h.game.handled, h.game.session_seen, h.session.seen, h.game.frame
    ));
    seen
}

fn recorded() -> (Vec<String>, PacketLines) {
    let mut h = host();
    let (log, lines) = PacketLog::new();
    h.set_packet_observer(Some(Box::new(log)));
    (run(&mut h), lines)
}

// A recorded session plays out identically to an unrecorded one.
#[test]
fn recording_does_not_change_the_game() {
    let plain = run(&mut host());
    let (with, lines) = recorded();
    assert_eq!(plain, with);
    assert!(lines.events() > 0);
    // Removing the observer turns the tap off again.
    let mut h = host();
    let (log, _) = PacketLog::new();
    h.set_packet_observer(Some(Box::new(log)));
    h.set_packet_observer(None);
    assert_eq!(run(&mut h), plain);
    assert!(h.buffers.take_tap().is_empty());
}

// The recorder emits one line per event, with fixed kinds, order and sequence numbers.
#[test]
fn records_in_the_1_14d_kinds_and_order() {
    let (_, lines) = recorded();
    let all = lines.take();
    let kinds: Vec<&str> = all
        .iter()
        .map(|l| {
            let k = &l[9..];
            &k[..k.find('"').unwrap()]
        })
        .collect();
    // Frame 0 (first use of the driver): drain with the create request.
    assert_eq!(kinds[..4], ["client_out", "drain", "c2s_sys", "s2c"][..]);
    assert_eq!(
        all[2],
        format!(
            "{{\"type\":\"c2s_sys\",\"client\":0,\"size\":46,\"bytes\":\"67{}\",\"frame\":null,\"phase\":\"input\",\"seq\":2}}",
            "00".repeat(45)
        )
    );
    // The session answer is queued in the drain.
    assert_eq!(
        all[3],
        r#"{"type":"s2c","client":0,"size":1,"bytes":"02","frame":null,"phase":"input","seq":3}"#
    );
    // Step 1: the game message, its filtered duplicate, dispatch, the
    // handler's message, the result; then the first tick and its flush.
    let rest: Vec<&str> = kinds[4..].to_vec();
    assert_eq!(
        rest[..14],
        [
            "client_send",
            "client_out",
            "client_send",
            "drain",
            "c2s",
            "dispatch",
            "s2c",
            "result",
            "tick",
            "s2c",
            "s2c",
            "tick_end",
            "flush",
            "net"
        ][..]
    );
    let line = |k: &str, n: usize| all.iter().filter(|l| l.contains(k)).nth(n).unwrap().clone();
    assert_eq!(
        line("\"dispatch\"", 0),
        r#"{"type":"dispatch","client":0,"game_frame":0,"size":5,"id":1,"frame":null,"phase":"input","seq":9}"#
    );
    assert_eq!(
        line("\"result\"", 0),
        r#"{"type":"result","client":0,"dispatched":true,"code":0,"frame":null,"phase":"input","seq":11}"#
    );
    assert!(line("\"tick\"", 0).contains(r#""frame":1,"phase":"tick""#));
    assert!(line("\"tick_end\"", 0).contains(r#""frame":1,"phase":"post""#));
    // One buffer: 0x02, 0x0C×9, 0x01×8, 0x15×11, from the flush.
    let net = line("\"net\"", 0);
    assert!(net.starts_with(r#"{"type":"net","kind":1,"client":0,"size":29,"bytes":"020c"#));
    assert!(net.contains(r#""caller":"0x52e3b5","frame":1,"phase":"flush""#));
    // The direct send: a net record marked direct, in its position.
    let direct = line("\"direct\"", 0);
    assert!(direct.starts_with(
        r#"{"type":"net","kind":1,"client":0,"size":1,"bytes":"b0","via":"direct","frame":2"#
    ));
    // Seq numbers are the line order.
    for (i, l) in all.iter().enumerate() {
        assert!(l.ends_with(&format!(",\"seq\":{i}}}")), "{l}");
    }
    let counts = lines.counts();
    assert_eq!(counts["tick"], counts["tick_end"]);
    assert_eq!(counts["tick"], counts["flush"]);
}
