//! Tests of the remote link with a scripted frame source and clock:
//! pacing, the jitter buffer growing and shrinking, catch-up, unticked
//! frames, sends, and a lost connection with and without a reconnect.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::*;

type Polled = Result<ServerFrame, OnlineError>;
type SentLog = Vec<(Lane, Vec<u8>)>;

/// Frames the test pushes, the sends the link made, and the game token
/// the source answers with.
#[derive(Clone, Default)]
struct Script {
    frames: Arc<Mutex<VecDeque<Polled>>>,
    sent: Arc<Mutex<SentLog>>,
    token: Option<ResumeToken>,
}

impl Script {
    fn with_token(mut self, token: ResumeToken) -> Self {
        self.token = Some(token);
        self
    }
    fn push(&self, f: ServerFrame) {
        self.frames.lock().unwrap().push_back(Ok(f));
    }
    fn tick(&self, n: u8) {
        self.push(ServerFrame {
            ticked: true,
            delivers: vec![vec![0x95, n]],
            ext: vec![],
        });
    }
    fn fail(&self, e: OnlineError) {
        self.frames.lock().unwrap().push_back(Err(e));
    }
}

impl FrameSource for Script {
    fn send(&mut self, lane: Lane, msg: &[u8]) -> Result<(), OnlineError> {
        self.sent.lock().unwrap().push((lane, msg.to_vec()));
        Ok(())
    }
    fn send_ext(&mut self, _channel: u16, _msg: &[u8]) -> Result<(), OnlineError> {
        Ok(())
    }
    fn poll_frame(&mut self) -> Result<Option<ServerFrame>, OnlineError> {
        self.frames.lock().unwrap().pop_front().transpose()
    }
    fn resume_token(&self) -> Option<ResumeToken> {
        self.token
    }
}

/// A clock the test moves, in whole milliseconds.
#[derive(Clone, Default)]
struct Clock(Arc<AtomicU64>);

impl Clock {
    fn add(&self, ms: u64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
    fn reader(&self) -> impl FnMut() -> f64 + Send + 'static {
        let c = self.0.clone();
        move || c.load(Ordering::SeqCst) as f64
    }
}

/// A 50 Hz link (20 ms interval) on a script and a test clock.
fn link() -> (RemoteLink<Script>, Script, Clock) {
    let script = Script::default();
    let clock = Clock::default();
    let rate = TickRate::new(50).unwrap();
    let l = RemoteLink::new(script.clone(), rate).with_clock(clock.reader());
    (l, script, clock)
}

fn pump(l: &mut RemoteLink<Script>) -> (bool, Vec<Vec<u8>>) {
    // The bridge clears its world on the frame after a resume, then pumps.
    l.take_resume();
    let p = l.pump().unwrap();
    (p.ticked, l.receive())
}

fn pump_raw(l: &mut RemoteLink<Script>) -> (bool, Vec<Vec<u8>>) {
    let p = l.pump().unwrap();
    (p.ticked, l.receive())
}

#[test]
fn it_waits_for_the_target_then_plays_one_tick_per_interval() {
    let (mut l, s, c) = link();
    s.tick(1);
    assert_eq!(
        pump(&mut l),
        (false, vec![]),
        "one frame is below the target"
    );
    s.tick(2);
    assert_eq!(pump(&mut l), (true, vec![vec![0x95, 1]]));
    // The next one waits for its beat.
    c.add(10);
    assert_eq!(pump(&mut l), (false, vec![]));
    c.add(10);
    s.tick(3);
    assert_eq!(pump(&mut l), (true, vec![vec![0x95, 2]]));
    c.add(20);
    assert_eq!(pump(&mut l), (true, vec![vec![0x95, 3]]));
}

#[test]
fn unticked_frames_pass_at_once_and_keep_their_order() {
    let (mut l, s, _c) = link();
    s.push(ServerFrame {
        ticked: false,
        delivers: vec![vec![0x01]],
        ext: vec![(4, vec![9])],
    });
    assert_eq!(pump(&mut l), (false, vec![vec![0x01]]));
    assert_eq!(l.take_ext(), [(4, vec![9])]);
}

#[test]
fn a_starved_buffer_grows_and_a_full_one_shrinks() {
    let (mut l, s, c) = link();
    s.tick(1);
    s.tick(2);
    assert!(pump(&mut l).0);
    c.add(20);
    assert!(pump(&mut l).0);
    // The network goes quiet for two intervals: the buffer starves.
    c.add(40);
    assert!(!pump(&mut l).0);
    assert_eq!(l.status().buffer().1, Pacer::START_TARGET + 1);
    // It refills to the new target before playing on.
    s.tick(3);
    s.tick(4);
    assert!(!pump(&mut l).0);
    s.tick(5);
    assert!(pump(&mut l).0);
    // Ten seconds with more than the target buffered: one frame less.
    for i in 0..600u32 {
        s.tick(i as u8);
        c.add(20);
        pump(&mut l);
        // Keep one extra frame in the buffer.
        if i == 0 {
            s.tick(0);
        }
    }
    assert!(l.status().buffer().1 <= Pacer::START_TARGET);
}

#[test]
fn far_behind_it_catches_up() {
    let (mut l, s, _c) = link();
    for i in 0..8 {
        s.tick(i);
    }
    // Without the clock moving, the frames past target + slack go at once.
    let mut ticks = 0;
    for _ in 0..8 {
        ticks += usize::from(pump(&mut l).0);
    }
    assert!(ticks >= 4, "{ticks}");
    assert!(l.status().buffer().0 <= Pacer::START_TARGET + 2);
}

#[test]
fn a_slow_display_catches_up_within_one_display_frame() {
    let (mut l, s, c) = link();
    l.set_display_catch_up(true);
    for i in 0..6 {
        s.tick(i);
    }
    assert!(pump(&mut l).0);
    // The display stalls for three intervals.
    c.add(60);
    let mut ticks = 0;
    while l.tick_pending() || ticks == 0 {
        assert!(pump(&mut l).0);
        ticks += 1;
        assert!(ticks < 10);
    }
    assert!(ticks >= 2, "{ticks}");
}

#[test]
fn sends_go_to_the_server_lane_and_are_queued() {
    let (mut l, s, _c) = link();
    assert_eq!(l.send(SendQueue::Game, &[0x03, 1]).unwrap(), Sent::Queued);
    assert_eq!(l.send(SendQueue::System, &[0x6B]).unwrap(), Sent::Queued);
    assert_eq!(
        *s.sent.lock().unwrap(),
        [(Lane::Game, vec![0x03, 1]), (Lane::System, vec![0x6B])]
    );
}

#[test]
fn a_lost_connection_without_reconnect_is_reported_not_raised() {
    let (mut l, s, _c) = link();
    s.fail(OnlineError::Lost(d2_net::NetError::TimedOut));
    let p = l.pump().unwrap();
    assert!(!p.ticked);
    let Connection::Lost(why) = l.status().connection() else {
        panic!("expected lost");
    };
    assert_eq!(why, "The connection to the server was lost.");
    // Later sends do nothing and say nothing.
    assert_eq!(l.send(SendQueue::Game, &[1]).unwrap(), Sent::Queued);
}

#[test]
fn a_reconnect_brings_the_game_back() {
    let (l, s, c) = link();
    let next = Script::default();
    next.tick(7);
    next.tick(8);
    let handed = next.clone();
    let mut l = l.with_reconnect(Box::new(move |_token| Ok(handed.clone())));
    s.fail(OnlineError::Lost(d2_net::NetError::TimedOut));
    l.pump().unwrap();
    assert_eq!(
        l.status().connection(),
        Connection::Reconnecting { attempts: 0 }
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut got = Vec::new();
    while l.status().connection() != Connection::Playing || got.is_empty() {
        assert!(Instant::now() < deadline, "no reconnect");
        c.add(20);
        got.extend(pump(&mut l).1);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(l.status().resumed(), 1);
    assert_eq!(got, [vec![0x95, 7]]);
}

#[test]
fn a_resume_drops_the_buffered_frames_and_renews_the_game_token() {
    let a = ResumeToken([1; 16]);
    let b = ResumeToken([2; 16]);
    let script = Script::default().with_token(a);
    let clock = Clock::default();
    let rate = TickRate::new(50).unwrap();
    let mut l = RemoteLink::new(script.clone(), rate).with_clock(clock.reader());
    // Two frames play; a third waits in the buffer.
    script.tick(1);
    script.tick(2);
    script.tick(3);
    assert_eq!(pump_raw(&mut l).1, [vec![0x95, 1]]);
    assert_eq!(l.status().buffer().0, 2, "two ticked frames wait");
    script.fail(OnlineError::Lost(d2_net::NetError::TimedOut));

    let calls = Arc::new(Mutex::new(Vec::new()));
    let next = Script::default().with_token(b);
    next.tick(7);
    next.tick(8);
    let handed = next.clone();
    let seen = calls.clone();
    let mut l = l.with_reconnect(Box::new(move |token| {
        seen.lock().unwrap().push(token);
        Ok(handed.clone())
    }));

    // The link notices the loss, reconnects, and says so before handing
    // out anything from the new stream.
    let deadline = Instant::now() + Duration::from_secs(5);
    while !l.take_resume() {
        assert!(Instant::now() < deadline, "no resume");
        clock.add(20);
        pump_raw(&mut l);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(*calls.lock().unwrap(), [Some(a)]);
    // The frames still buffered when the reconnect came back are gone: the
    // first frame the bridge sees is the new stream's first, not a stale one.
    clock.add(20);
    let (ticked, got) = pump_raw(&mut l);
    assert!(ticked);
    assert_eq!(got, [vec![0x95, 7]]);

    // A later loss comes back with the token the resumed stream carried, not
    // the game's first one.
    next.fail(OnlineError::Lost(d2_net::NetError::TimedOut));
    let deadline = Instant::now() + Duration::from_secs(5);
    while calls.lock().unwrap().len() < 2 {
        assert!(Instant::now() < deadline, "no second attempt");
        clock.add(20);
        l.take_resume();
        pump_raw(&mut l);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(calls.lock().unwrap()[1], Some(b));
}

#[test]
fn a_refused_reconnect_ends_the_game_with_the_reason() {
    let (l, s, _c) = link();
    let mut l = l.with_reconnect(Box::new(|_token| {
        Err(OnlineError::Refused {
            code: d2_online::ErrorCode::UNKNOWN_RESUME,
            text: "gone".into(),
        })
    }));
    s.fail(OnlineError::Lost(d2_net::NetError::Closed));
    l.pump().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Connection::Lost(why) = l.status().connection() {
            assert_eq!(why, d2_online::ErrorCode::UNKNOWN_RESUME.text());
            break;
        }
        assert!(Instant::now() < deadline, "never gave up");
        l.pump().unwrap();
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn a_remote_link_says_its_answers_take_network_time() {
    let (l, _s, _c) = link();
    assert!(l.is_remote());
    // Through a boxed link too (the bridge holds it boxed).
    let boxed: Box<dyn ServerLink + Send> = Box::new(l);
    assert!(boxed.is_remote());
}
