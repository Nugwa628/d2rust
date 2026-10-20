//! Rate limits on registering and logging in (contracts section 5).
//!
//! | What | Limit |
//! |---|---|
//! | New accounts per address | 3 per hour |
//! | Login attempts per address | 10 per minute (a token bucket) |
//! | Failed logins per account name | 10 in 15 minutes, then locked 15 minutes |
//!
//! An address is an IPv4 address, or the /64 network of an IPv6 address
//! (one home connection usually holds a whole /64). Failed logins count
//! per account name whether or not the account exists, so a lock says
//! nothing about which names are taken. The limits live in memory and
//! start empty on a restart. The caller passes the time, so the rules
//! are tested without waiting.
//!
//! With `limits = test` ([`Limits::test`]) every window and lock is
//! divided by 60 (an hour becomes a minute, a minute a second, 15 minutes
//! 15 seconds); the counts stay. The server warns about it in its log.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::time::{Duration, Instant};

/// New accounts one address may make per [`REGISTER_WINDOW`].
pub const REGISTERS: usize = 3;
pub const REGISTER_WINDOW: Duration = Duration::from_secs(3600);
/// Login attempts per address: the bucket holds this many, and refills
/// all of them over [`LOGIN_REFILL`].
pub const LOGINS: u32 = 10;
pub const LOGIN_REFILL: Duration = Duration::from_secs(60);
/// Failed logins of one account name per [`FAILURE_WINDOW`] before it locks.
pub const FAILURES: usize = 10;
pub const FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);
/// How long a locked account name stays locked.
pub const LOCK: Duration = Duration::from_secs(15 * 60);

/// The windows and the lock length; the test mode divides each by 60.
#[derive(Clone, Copy, Debug)]
struct Windows {
    register: Duration,
    login_refill: Duration,
    failure: Duration,
    lock: Duration,
}

const NORMAL: Windows = Windows {
    register: REGISTER_WINDOW,
    login_refill: LOGIN_REFILL,
    failure: FAILURE_WINDOW,
    lock: LOCK,
};

const TEST: Windows = Windows {
    register: Duration::from_secs(60),
    login_refill: Duration::from_secs(1),
    failure: Duration::from_secs(15),
    lock: Duration::from_secs(15),
};

/// Entries are pruned of expired state every this many calls.
const PRUNE_EVERY: u32 = 1024;

/// The key an address is limited under.
pub fn address_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(_) => ip,
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => {
                let s = v6.segments();
                IpAddr::V6(std::net::Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
            }
        },
    }
}

/// Whole seconds until `until`, at least 1: what an answer's `wait_s` says.
fn wait_s(now: Instant, until: Instant) -> u32 {
    let d = until.saturating_duration_since(now);
    let s = d.as_secs() + u64::from(d.subsec_nanos() > 0);
    u32::try_from(s.max(1)).unwrap_or(u32::MAX)
}

#[derive(Debug)]
struct Bucket {
    /// When the bucket is full again. Each attempt moves it one token
    /// (1/[`LOGINS`] of [`LOGIN_REFILL`]) later; the bucket is empty while
    /// it is a whole refill period away.
    full_at: Instant,
}

#[derive(Debug, Default)]
struct Failures {
    times: VecDeque<Instant>,
    locked_until: Option<Instant>,
}

/// The limits' state.
#[derive(Debug)]
pub struct Limits {
    windows: Windows,
    registers: HashMap<IpAddr, VecDeque<Instant>>,
    logins: HashMap<IpAddr, Bucket>,
    failures: HashMap<String, Failures>,
    calls: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self::new()
    }
}

impl Limits {
    /// The normal limits.
    pub fn new() -> Self {
        Self::with(NORMAL)
    }

    /// The test limits: every window and lock of [`NORMAL`] divided by 60.
    pub fn test() -> Self {
        Self::with(TEST)
    }

    fn with(windows: Windows) -> Self {
        Self {
            windows,
            registers: HashMap::new(),
            logins: HashMap::new(),
            failures: HashMap::new(),
            calls: 0,
        }
    }

    fn tick(&mut self, now: Instant) {
        self.calls = self.calls.wrapping_add(1);
        if self.calls.is_multiple_of(PRUNE_EVERY) {
            self.prune(now);
        }
    }

    /// Drops state that no longer limits anything.
    pub fn prune(&mut self, now: Instant) {
        self.registers.retain(|_, times| {
            times.retain(|&t| now.saturating_duration_since(t) < self.windows.register);
            !times.is_empty()
        });
        self.logins.retain(|_, b| b.full_at > now);
        self.failures.retain(|_, f| {
            f.times
                .retain(|&t| now.saturating_duration_since(t) < self.windows.failure);
            !f.times.is_empty() || f.locked_until.is_some_and(|u| u > now)
        });
    }

    /// Whether `ip` may make another account now; `Err(wait_s)` if not.
    pub fn check_register(&mut self, ip: IpAddr, now: Instant) -> Result<(), u32> {
        self.tick(now);
        let window = self.windows.register;
        let Some(times) = self.registers.get_mut(&address_key(ip)) else {
            return Ok(());
        };
        times.retain(|&t| now.saturating_duration_since(t) < window);
        if times.len() < REGISTERS {
            return Ok(());
        }
        Err(wait_s(now, times[0] + window))
    }

    /// Counts a new account made from `ip`.
    pub fn note_register(&mut self, ip: IpAddr, now: Instant) {
        self.registers
            .entry(address_key(ip))
            .or_default()
            .push_back(now);
    }

    /// Checks and counts one new account from `ip` in one step, so
    /// registrations that run at the same time can't all pass a check made
    /// before any of them is counted. A failed registration releases its
    /// slot with [`Limits::release_register`].
    pub fn reserve_register(&mut self, ip: IpAddr, now: Instant) -> Result<(), u32> {
        self.check_register(ip, now)?;
        self.registers
            .entry(address_key(ip))
            .or_default()
            .push_back(now);
        Ok(())
    }

    /// Gives back the slot [`Limits::reserve_register`] took at `at`.
    pub fn release_register(&mut self, ip: IpAddr, at: Instant) {
        let Some(times) = self.registers.get_mut(&address_key(ip)) else {
            return;
        };
        if let Some(i) = times.iter().position(|&t| t == at) {
            times.remove(i);
        }
    }

    /// Takes one login attempt from `ip`'s bucket; `Err(wait_s)` when it is
    /// empty.
    pub fn take_login(&mut self, ip: IpAddr, now: Instant) -> Result<(), u32> {
        self.tick(now);
        let refill = self.windows.login_refill;
        let per_token = refill / LOGINS;
        let bucket = self
            .logins
            .entry(address_key(ip))
            .or_insert(Bucket { full_at: now });
        // The bucket is empty when it is a whole refill period from full.
        let full_at = bucket.full_at.max(now);
        let after = full_at + per_token;
        if after.saturating_duration_since(now) > refill {
            return Err(wait_s(now, after - refill));
        }
        bucket.full_at = after;
        Ok(())
    }

    /// `Some(wait_s)` while the account name `key` is locked.
    pub fn locked(&mut self, key: &str, now: Instant) -> Option<u32> {
        let f = self.failures.get_mut(key)?;
        match f.locked_until {
            Some(until) if until > now => Some(wait_s(now, until)),
            Some(_) => {
                f.locked_until = None;
                None
            }
            None => None,
        }
    }

    /// Counts a failed login of the account name `key`; locks it on the
    /// [`FAILURES`]th failure inside the window.
    pub fn note_failure(&mut self, key: &str, now: Instant) {
        let window = self.windows.failure;
        let lock = self.windows.lock;
        let f = self.failures.entry(key.to_owned()).or_default();
        f.times
            .retain(|&t| now.saturating_duration_since(t) < window);
        f.times.push_back(now);
        if f.times.len() >= FAILURES {
            f.times.clear();
            f.locked_until = Some(now + lock);
        }
    }

    /// A good login clears the name's failures.
    pub fn clear_failures(&mut self, key: &str) {
        self.failures.remove(key);
    }

    /// How many addresses and names hold state (for tests and the status
    /// file).
    pub fn tracked(&self) -> usize {
        self.registers.len() + self.logins.len() + self.failures.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn three_accounts_per_address_per_hour() {
        let mut l = Limits::new();
        let t0 = Instant::now();
        let a = ip("10.0.0.1");
        for i in 0..REGISTERS {
            assert_eq!(l.check_register(a, t0), Ok(()), "account {i}");
            l.note_register(a, t0 + Duration::from_secs(i as u64));
        }
        assert_eq!(l.check_register(a, t0 + Duration::from_secs(10)), Err(3590));
        // Another address is not limited.
        assert_eq!(l.check_register(ip("10.0.0.2"), t0), Ok(()));
        // An hour after the first, one more is allowed.
        assert_eq!(l.check_register(a, t0 + REGISTER_WINDOW), Ok(()));
    }

    #[test]
    fn ten_logins_per_minute_then_one_per_six_seconds() {
        let mut l = Limits::new();
        let t0 = Instant::now();
        let a = ip("10.0.0.1");
        for i in 0..LOGINS {
            assert_eq!(l.take_login(a, t0), Ok(()), "attempt {i}");
        }
        assert_eq!(l.take_login(a, t0), Err(6));
        assert_eq!(l.take_login(a, t0 + Duration::from_secs(5)), Err(1));
        assert_eq!(l.take_login(a, t0 + Duration::from_secs(6)), Ok(()));
        assert!(l.take_login(a, t0 + Duration::from_secs(6)).is_err());
        // A full minute refills the bucket.
        let t1 = t0 + Duration::from_secs(6) + LOGIN_REFILL;
        for _ in 0..LOGINS {
            assert_eq!(l.take_login(a, t1), Ok(()));
        }
        assert!(l.take_login(a, t1).is_err());
    }

    #[test]
    fn ipv6_is_limited_per_64() {
        let mut l = Limits::new();
        let t0 = Instant::now();
        for i in 0..LOGINS {
            let a = ip(&format!("2001:db8:1:2::{:x}", i + 1));
            assert_eq!(l.take_login(a, t0), Ok(()));
        }
        assert!(l.take_login(ip("2001:db8:1:2:ffff::1"), t0).is_err());
        assert_eq!(l.take_login(ip("2001:db8:1:3::1"), t0), Ok(()));
        assert_eq!(
            address_key(ip("::ffff:10.1.2.3")),
            ip("10.1.2.3"),
            "a mapped IPv4 address is its IPv4 address"
        );
    }

    #[test]
    fn ten_failures_lock_the_name_for_fifteen_minutes() {
        let mut l = Limits::new();
        let t0 = Instant::now();
        for i in 0..FAILURES - 1 {
            l.note_failure("mogger", t0 + Duration::from_secs(i as u64));
            assert_eq!(l.locked("mogger", t0), None);
        }
        let t1 = t0 + Duration::from_secs(60);
        l.note_failure("mogger", t1);
        assert_eq!(l.locked("mogger", t1), Some(900));
        assert_eq!(l.locked("other", t1), None);
        assert_eq!(
            l.locked("mogger", t1 + LOCK - Duration::from_millis(1)),
            Some(1)
        );
        assert_eq!(l.locked("mogger", t1 + LOCK), None);
    }

    #[test]
    fn failures_outside_the_window_do_not_count() {
        let mut l = Limits::new();
        let t0 = Instant::now();
        for i in 0..FAILURES - 1 {
            l.note_failure("mogger", t0 + Duration::from_secs(i as u64));
        }
        l.note_failure("mogger", t0 + FAILURE_WINDOW + Duration::from_secs(5));
        assert_eq!(
            l.locked("mogger", t0 + FAILURE_WINDOW + Duration::from_secs(5)),
            None
        );
    }

    #[test]
    fn a_good_login_clears_failures() {
        let mut l = Limits::new();
        let t0 = Instant::now();
        for _ in 0..FAILURES - 1 {
            l.note_failure("mogger", t0);
        }
        l.clear_failures("mogger");
        l.note_failure("mogger", t0);
        assert_eq!(l.locked("mogger", t0), None);
    }

    #[test]
    fn prune_forgets_expired_state() {
        let mut l = Limits::new();
        let t0 = Instant::now();
        l.note_register(ip("10.0.0.1"), t0);
        l.take_login(ip("10.0.0.1"), t0).unwrap();
        l.note_failure("mogger", t0);
        assert_eq!(l.tracked(), 3);
        l.prune(t0 + REGISTER_WINDOW);
        assert_eq!(l.tracked(), 0);
    }

    #[test]
    fn a_reserved_slot_can_be_given_back() {
        let mut l = Limits::new();
        let t0 = Instant::now();
        let a = ip("10.0.0.1");
        for i in 0..REGISTERS {
            l.reserve_register(a, t0 + Duration::from_secs(i as u64))
                .unwrap();
        }
        assert!(l.reserve_register(a, t0 + Duration::from_secs(10)).is_err());
        l.release_register(a, t0 + Duration::from_secs(1));
        l.reserve_register(a, t0 + Duration::from_secs(11))
            .expect("the released slot is free again");
    }

    #[test]
    fn the_test_mode_divides_every_window_by_sixty() {
        let mut l = Limits::test();
        let t0 = Instant::now();
        let a = ip("10.0.0.1");
        for i in 0..REGISTERS {
            l.reserve_register(a, t0 + Duration::from_secs(i as u64))
                .unwrap();
        }
        // The first slot frees a minute after it was taken.
        assert_eq!(l.reserve_register(a, t0 + Duration::from_secs(10)), Err(50));
        assert_eq!(l.reserve_register(a, t0 + Duration::from_secs(60)), Ok(()));
        // A login token refills in a tenth of the one-second refill.
        for _ in 0..LOGINS {
            assert_eq!(l.take_login(a, t0), Ok(()));
        }
        assert_eq!(l.take_login(a, t0), Err(1));
        assert_eq!(l.take_login(a, t0 + Duration::from_millis(100)), Ok(()));
        // Ten failures lock the name for 15 seconds.
        for _ in 0..FAILURES {
            l.note_failure("mogger", t0);
        }
        assert_eq!(l.locked("mogger", t0), Some(15));
        assert_eq!(l.locked("mogger", t0 + Duration::from_secs(15)), None);
    }
}
