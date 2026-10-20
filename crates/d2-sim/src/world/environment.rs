//! The act environment: the day/night period of one act, as the server
//! tracks it. [`Environment`] holds the period index, type, ticks, speed and
//! eclipse flag; [`Environment::server_advance`] steps it and says when a
//! client needs a new S→C 0x53 (also sent at a join). The Tainted Sun
//! quest code swaps the tables and speed ([`ECLIPSE`], [`SPEED_ECLIPSE`]).
//!
//! Kept: the three fields 0x53 carries (period index, ticks, eclipse flag)
//! and what the server advance reads (period type, speed, last reported
//! hour). The server never runs the intensity or color steps, so the light
//! intensity and R, G, B are the client's.

/// One period table entry: start degree, type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Period {
    pub start: u32,
    pub kind: u32,
}

const fn p(start: u32, kind: u32) -> Period {
    Period { start, kind }
}

/// The normal period table.
pub const NORMAL: [Period; 6] = [
    p(320, 3),
    p(340, 3),
    p(0, 0),
    p(160, 1),
    p(180, 1),
    p(200, 2),
];
/// The act 4 period table.
pub const ACT4: [Period; 6] = [
    p(340, 3),
    p(350, 3),
    p(0, 0),
    p(180, 1),
    p(190, 1),
    p(200, 2),
];
/// The eclipse (Tainted Sun) period table.
pub const ECLIPSE: [Period; 6] = [
    p(300, 3),
    p(0, 0),
    p(60, 1),
    p(120, 2),
    p(180, 2),
    p(240, 2),
];

/// Speed (ticks per degree) in the normal cycle and in the Tainted Sun
/// cycle.
pub const SPEED_NORMAL: u32 = 128;
pub const SPEED_ECLIPSE: u32 = 4;

/// The fields of one act's environment record the server keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Environment {
    /// Period index 0–5.
    pub period: u32,
    /// Period type (from the table).
    pub kind: u32,
    /// Ticks.
    pub ticks: u32,
    /// Speed (ticks per degree).
    pub speed: u32,
    /// Eclipse flag.
    pub eclipse: bool,
    /// Last reported hour (set by [`Environment::server_advance`]).
    pub last_hour: i32,
}

impl Environment {
    /// The record at its creation: index 2, type and ticks (start × speed
    /// = 0) from the normal table's entry 2, speed 128, eclipse 0, last
    /// reported hour 0.
    pub const CREATED: Self = Self {
        period: 2,
        kind: NORMAL[2].kind,
        ticks: 0,
        speed: SPEED_NORMAL,
        eclipse: false,
        last_hour: 0,
    };

    /// S→C 0x53 (10 bytes): u32@1 period index, u32@5 ticks,
    /// u8@9 eclipse.
    pub fn message(&self) -> [u8; 10] {
        let mut b = [0u8; 10];
        b[0] = 0x53;
        b[1..5].copy_from_slice(&self.period.to_le_bytes());
        b[5..9].copy_from_slice(&self.ticks.to_le_bytes());
        b[9] = u8::from(self.eclipse);
        b
    }

    /// The base time `t` for time-based stat values: ticks divided by the
    /// speed, 0 when the speed is 0.
    pub fn time(&self) -> i32 {
        self.ticks.checked_div(self.speed).unwrap_or(0) as i32
    }

    /// Advance one tick with act index `a`: step the ticks, wrap, and
    /// change the period when the next entry starts.
    pub fn advance(&mut self, a: u8) {
        // Step the ticks: +1, more in act 4, and in act 3 or at a type-2
        // period.
        let mut t = self.ticks.wrapping_add(1);
        if !self.eclipse {
            if a == 3 {
                t = t.wrapping_add(15);
            } else if NORMAL[self.period as usize % 6].kind == 2 {
                t = t.wrapping_add(1);
                if a == 2 {
                    t = t.wrapping_add(8);
                }
            }
        }
        // Wrap at speed × 360.
        if u64::from(t) >= u64::from(self.speed) * 360 {
            t = 0;
        }
        self.ticks = t;
        // Move to the next period when its start has passed.
        let next = (self.period as usize + 1) % 6;
        let table = if !self.eclipse {
            &NORMAL
        } else if a == 3 {
            &ACT4
        } else {
            &ECLIPSE
        };
        if u64::from(table[next].start) * u64::from(self.speed) < u64::from(self.ticks) {
            let entry = if self.eclipse {
                ECLIPSE[next]
            } else {
                NORMAL[next]
            };
            self.period = next as u32;
            self.kind = entry.kind;
            self.ticks = entry.start.wrapping_mul(self.speed);
        }
    }

    /// The server advance for one act: the advance
    /// with act index `a` (no intensity, no color), then the report. With
    /// speed ≠ 0 and |ticks / speed − last hour| > 16 (signed integer
    /// division), the last hour becomes ticks / speed and the result is
    /// true; otherwise true when the index or the type changed.
    pub fn server_advance(&mut self, a: u8) -> bool {
        let (index, kind) = (self.period, self.kind);
        self.advance(a);
        if self.speed != 0 {
            let hour = (self.ticks as i32).wrapping_div(self.speed as i32);
            if (i64::from(hour) - i64::from(self.last_hour)).abs() > 16 {
                self.last_hour = hour;
                return true;
            }
        }
        self.period != index || self.kind != kind
    }
}

impl Default for Environment {
    fn default() -> Self {
        Self::CREATED
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn created_record_sends_the_recorded_join_message() {
        // `53 02000000 00000000 00`: the message a player gets at a join.
        assert_eq!(
            Environment::CREATED.message(),
            [0x53, 2, 0, 0, 0, 0, 0, 0, 0, 0]
        );
        // A later message: index 2, ticks 0x880.
        let e = Environment {
            ticks: 0x880,
            ..Environment::default()
        };
        assert_eq!(e.message(), [0x53, 2, 0, 0, 0, 0x80, 0x08, 0, 0, 0]);
        let e = Environment {
            period: 5,
            ticks: 0,
            eclipse: true,
            ..Environment::default()
        };
        assert_eq!(e.message()[9], 1);
        assert_eq!(e.message()[1], 5);
    }

    /// Recorded vector: act 1 (a = 0, speed 128, +1 per frame
    /// at index 2) first reports with ticks 0x880 (2176 / 128 = 17 > 16);
    /// the 0x53 is `53 02000000 80080000 00`.
    #[test]
    fn server_advance_reports_every_17_degrees() {
        let mut e = Environment::CREATED;
        for _ in 0..2175 {
            assert!(!e.server_advance(0));
        }
        assert!(e.server_advance(0));
        assert_eq!(e.ticks, 0x880);
        assert_eq!(e.last_hour, 17);
        assert_eq!(e.message(), [0x53, 2, 0, 0, 0, 0x80, 0x08, 0, 0, 0]);
        // The next report 17 degrees later.
        for _ in 0..17 * 128 - 1 {
            assert!(!e.server_advance(0));
        }
        assert!(e.server_advance(0));
        assert_eq!(e.last_hour, 34);
    }

    /// The tick step: +16 in act 4 (a = 3); at a type-2 index +2, and +10
    /// in act 3 (a = 2); with the eclipse flag +1 everywhere.
    #[test]
    fn advance_steps_by_act_and_type() {
        // Ticks inside the period (no period change).
        let at = |period: u32, eclipse: bool, a: u8| {
            let base = if period == 5 { 30_000 } else { 23_100 };
            let mut e = Environment {
                period,
                ticks: base,
                eclipse,
                ..Environment::CREATED
            };
            e.advance(a);
            e.ticks - base
        };
        assert_eq!(at(5, false, 3), 16);
        assert_eq!(at(5, false, 0), 2);
        assert_eq!(at(5, false, 2), 10);
        assert_eq!(at(4, false, 2), 1);
        assert_eq!(at(5, true, 3), 1);
        assert_eq!(at(5, true, 2), 1);
    }

    /// The wrap at speed × 360, and the period change
    /// when the next entry's start × speed < ticks (normal 2 → 3 at
    /// 160 × 128, ticks := 20480, type 1), which reports.
    #[test]
    fn advance_wraps_and_changes_period() {
        let mut e = Environment {
            ticks: 128 * 360 - 1,
            ..Environment::CREATED
        };
        e.advance(0);
        assert_eq!(e.ticks, 0);
        let mut e = Environment {
            ticks: 20_479,
            last_hour: 159,
            ..Environment::CREATED
        };
        assert!(!e.server_advance(0)); // 20480: not past the start
        assert_eq!((e.period, e.ticks), (2, 20_480));
        assert!(e.server_advance(0));
        assert_eq!((e.period, e.kind, e.ticks), (3, 1, 20_480));
        assert_eq!(e.last_hour, 159);
        // Period 1 (start 340) lasts one update: its successor starts at 0.
        let mut e = Environment {
            period: 1,
            kind: 3,
            ticks: 340 * 128,
            ..Environment::CREATED
        };
        e.advance(0);
        assert_eq!((e.period, e.kind, e.ticks), (2, 0, 0));
    }

    /// The eclipse cycle: speed 4; the
    /// eclipse table in acts other than 4, the act-4 table in act 4 for
    /// the "next" test, the type and ticks from the eclipse table.
    #[test]
    fn eclipse_advance_uses_the_eclipse_tables() {
        let sun = Environment {
            period: 0,
            kind: 3,
            ticks: 1_200,
            speed: SPEED_ECLIPSE,
            eclipse: true,
            last_hour: 0,
        };
        // Act II: eclipse entry 1 starts at 0 < 1201 → index 1, type 0,
        // ticks 0.
        let mut e = sun;
        assert!(e.server_advance(1));
        assert_eq!((e.period, e.kind, e.ticks), (1, 0, 0));
        // Act IV: act-4 entry 1 starts at 350 × 4 = 1400 ≥ 1201: stays.
        let mut e = sun;
        e.advance(3);
        assert_eq!((e.period, e.kind, e.ticks), (0, 3, 1_201));
        // The wrap at 4 × 360.
        let mut e = Environment {
            period: 5,
            kind: 2,
            ticks: 1_439,
            ..sun
        };
        e.advance(1);
        assert_eq!((e.period, e.ticks), (5, 0));
    }
}
