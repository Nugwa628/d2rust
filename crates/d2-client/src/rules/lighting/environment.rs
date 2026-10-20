//! The act environment, day and night, and the ambient of a room.
//!
//! The day-night cycle and the eclipse use doubles and `sin`; this is
//! client-only state, never `d2-sim`.

pub use d2_server::game::rows::Ambient;

use thiserror::Error;

/// `π` as a float, read as a double.
pub const PI_F: f64 = 3.1415927410125732;

/// Speed in ticks per degree: the act's speed slot is never written, so
/// it is always 128.
pub const SPEED: i32 = 128;

/// Level id whose environment color is forced.
pub const LEVEL_COLOR_OVERRIDE: u32 = 120;
/// The forced color of level 120.
pub const LEVEL_120_RGB: (u8, u8, u8) = (245, 240, 255);

/// The period tables as text (`render/env-periods.tsv`).
pub const ENV_PERIODS_TSV: &str = include_str!("../../../../../tables/render/env-periods.tsv");

const TSV_HEADER: &str = "table\tperiod\tstart_deg\ttype\tr\tg\tb";
const TABLE_NAMES: [&str; 3] = ["normal", "act4", "eclipse"];

/// One period entry (start degree, type, color).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Period {
    /// Start degree.
    pub start: i32,
    /// Period type.
    pub kind: i32,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// The three period tables: normal, act 4 and eclipse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PeriodTables {
    pub normal: [Period; 6],
    pub act4: [Period; 6],
    pub eclipse: [Period; 6],
}

/// A malformed `env-periods.tsv`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TsvError {
    #[error("env-periods.tsv: bad header {0:?}")]
    Header(String),
    #[error("env-periods.tsv: line {line}: {reason}")]
    Row { line: usize, reason: String },
    #[error("env-periods.tsv: {0} data rows, expected 18")]
    RowCount(usize),
}

impl PeriodTables {
    /// Parses `env-periods.tsv` strictly: the exact header, then 18 rows in
    /// order (normal, act4, eclipse; periods 0–5 each), every field an
    /// integer in range. A trailing newline is allowed; nothing else.
    pub fn parse(text: &str) -> Result<Self, TsvError> {
        let mut lines = text.lines();
        let header = lines.next().unwrap_or("");
        if header != TSV_HEADER {
            return Err(TsvError::Header(header.to_string()));
        }
        let mut entries = Vec::with_capacity(18);
        for (n, line) in lines.enumerate() {
            let line_no = n + 2;
            let row = |reason: &str| TsvError::Row {
                line: line_no,
                reason: reason.to_string(),
            };
            let idx = entries.len();
            if idx >= 18 {
                return Err(TsvError::RowCount(idx + 1));
            }
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() != 7 {
                return Err(row("expected 7 fields"));
            }
            if f[0] != TABLE_NAMES[idx / 6] {
                return Err(row("table name out of order"));
            }
            let int = |s: &str| s.parse::<i32>().map_err(|_| row("not an integer"));
            if int(f[1])? != (idx % 6) as i32 {
                return Err(row("period out of order"));
            }
            let start = int(f[2])?;
            if !(0..360).contains(&start) {
                return Err(row("start degree out of 0..360"));
            }
            let kind = int(f[3])?;
            let byte = |s: &str| s.parse::<u8>().map_err(|_| row("not a byte"));
            entries.push(Period {
                start,
                kind,
                r: byte(f[4])?,
                g: byte(f[5])?,
                b: byte(f[6])?,
            });
        }
        if entries.len() != 18 {
            return Err(TsvError::RowCount(entries.len()));
        }
        let table = |t: usize| -> [Period; 6] { std::array::from_fn(|i| entries[t * 6 + i]) };
        Ok(PeriodTables {
            normal: table(0),
            act4: table(1),
            eclipse: table(2),
        })
    }

    /// The tables of the repo's `env-periods.tsv`.
    pub fn builtin() -> Result<Self, TsvError> {
        Self::parse(ENV_PERIODS_TSV)
    }
}

/// The act index `A` of a level id: the number of
/// act start levels 40, 75, 103, 109 that are ≤ `level`.
pub fn act_index(level: u32) -> u32 {
    [40, 75, 103, 109].iter().filter(|&&s| s <= level).count() as u32
}

/// A fatal S→C 0x53, or a case whose outcome is not known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum EnvError {
    #[error("S→C 0x53: period index {0} outside 0..=5 (fatal)")]
    BadIndex(i32),
    #[error("S→C 0x53: negative ticks {0} (fatal)")]
    NegativeTicks(i32),
}

/// The act's environment record. The −cos θ and zero floats and the
/// server's last hour have no client reader and are not held.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Environment {
    /// `+0x00` period index 0–5.
    pub index: i32,
    /// `+0x04` period type.
    pub kind: i32,
    /// `+0x08` ticks.
    pub ticks: i32,
    /// `+0x0C` intensity `I`.
    pub intensity: i32,
    /// `+0x10` `GetTickCount` at creation (an input).
    pub created_ms: u32,
    /// `+0x18..+0x1A`.
    pub r: u8,
    pub g: u8,
    pub b: u8,
    /// `+0x24` the sine term `s` (a float).
    pub s: f32,
    /// `+0x28` speed (ticks per degree).
    pub speed: i32,
    /// `+0x30` eclipse flag.
    pub eclipse: bool,
}

// The one float, `s`, is a sine of a finite angle or 0: never NaN, so
// equality is reflexive and the model holding the record stays `Eq`.
impl Eq for Environment {}

impl Environment {
    /// Creation: index 2, type and ticks from the
    /// normal entry 2, then intensity and color with `A` = 0, `L` = 0,
    /// eclipse 0: ticks 0, `I` = 128, R, G, B = entry 2's color.
    pub fn new(tables: &PeriodTables, now_ms: u32) -> Self {
        let e = tables.normal[2];
        let mut env = Environment {
            index: 2,
            kind: e.kind,
            ticks: e.start * SPEED,
            intensity: 0,
            created_ms: now_ms,
            r: 0,
            g: 0,
            b: 0,
            s: 0.0,
            speed: SPEED,
            eclipse: false,
        };
        env.intensity(0, 0);
        env.color(tables, 0);
        env
    }

    fn day_ticks(&self) -> i32 {
        self.speed * 360
    }

    /// Advance in act index `a`.
    pub fn advance(&mut self, tables: &PeriodTables, a: u32) {
        self.advance_at(tables, a, d2_sim::tick::TickRate::BASE, 0);
    }

    pub fn advance_at(
        &mut self,
        tables: &PeriodTables,
        a: u32,
        rate: d2_sim::tick::TickRate,
        frame: i32,
    ) {
        let mut amount = 1;
        if !self.eclipse {
            if a == 3 {
                amount += 15;
            } else if tables.normal[self.index as usize].kind == 2 {
                amount += 1;
                if a == 2 {
                    amount += 8;
                }
            }
        }
        self.ticks += rate.share(amount, frame);
        if self.ticks >= self.day_ticks() {
            self.ticks = 0;
        }
        let next = (self.index as usize + 1) % 6;
        let t = if !self.eclipse {
            &tables.normal
        } else if a == 3 {
            &tables.act4
        } else {
            &tables.eclipse
        };
        if t[next].start * self.speed < self.ticks {
            self.index = next as i32;
            let set = if self.eclipse {
                &tables.eclipse
            } else {
                &tables.normal
            };
            self.kind = set[next].kind;
            self.ticks = set[next].start * self.speed;
        }
    }

    /// Intensity in act index `a`, level `l`.
    pub fn intensity(&mut self, a: u32, l: u32) {
        if a == 3 {
            let target = match l {
                103 => 128,
                104 => 64,
                105 => 56,
                106 => 48,
                _ => 16,
            };
            if self.intensity < target {
                self.intensity += 1;
            } else if self.intensity > target {
                self.intensity -= 1;
            }
            if self.intensity == 0 {
                self.intensity = target;
            }
        } else if self.eclipse {
            if self.intensity > 32 {
                self.intensity -= 8;
            }
            if self.intensity < 32 {
                self.intensity = 32;
            }
        } else if l == LEVEL_COLOR_OVERRIDE {
            self.intensity = 200;
        } else {
            let speed = f64::from(self.speed);
            let theta = ((f64::from(self.ticks) / speed) / 180.0) * PI_F;
            let s = if self.ticks < self.speed * 180 {
                theta.sin()
            } else {
                0.5 * theta.sin()
            };
            // Stored as a float, then reloaded.
            self.s = s as f32;
            let v = (f64::from(self.s) * 128.0 + 128.0 + 0.5) as i32;
            let cap = if a == 4 { 170 } else { 255 };
            self.intensity = v.clamp(0, cap);
        }
    }

    /// Color in act index `a`: interpolation between
    /// the current and the next entry, each channel stored mod 256.
    pub fn color(&mut self, tables: &PeriodTables, a: u32) {
        let t = if a == 3 {
            &tables.act4
        } else if self.eclipse {
            &tables.eclipse
        } else {
            &tables.normal
        };
        let i = self.index as usize;
        let c = t[i];
        let n = t[(i + 1) % 6];
        let speed = f64::from(self.speed);
        let f = (f64::from(self.ticks) - f64::from(c.start) * speed)
            / (f64::from(n.start - c.start) * speed);
        let ch = |cv: u8, nv: u8| -> u8 {
            let d = (f64::from(i32::from(nv) - i32::from(cv)) * f + 0.5) as i32;
            (i32::from(cv).wrapping_add(d) & 0xFF) as u8
        };
        self.r = ch(c.r, n.r);
        self.g = ch(c.g, n.g);
        self.b = ch(c.b, n.b);
    }

    fn level_override(&mut self, l: u32) {
        if l == LEVEL_COLOR_OVERRIDE {
            (self.r, self.g, self.b) = LEVEL_120_RGB;
        }
    }

    /// One client update with the player room's
    /// level id `level` (0 when none): advance, intensity, color, then the
    /// level-120 color.
    pub fn update(&mut self, tables: &PeriodTables, level: u32) {
        self.update_at(tables, level, d2_sim::tick::TickRate::BASE, 0);
    }

    pub fn update_at(
        &mut self,
        tables: &PeriodTables,
        level: u32,
        rate: d2_sim::tick::TickRate,
        frame: i32,
    ) {
        let a = act_index(level);
        self.advance_at(tables, a, rate, frame);
        self.intensity(a, level);
        self.color(tables, a);
        self.level_override(level);
    }

    /// The S→C 0x53 setter: index > 5 or < 0,
    /// ticks < 0 → fatal; ticks > speed × 360 → 0; index, ticks and type
    /// (normal or eclipse table by the received flag) set; intensity with
    /// the **previous** eclipse flag; the flag set; with it, the period
    /// reset ([`Self::period_reset`]), intensity again and color; then
    /// the level-120 color. Without the flag no color is recomputed.
    ///
    /// `a`, `level`: the act index and level the intensity step uses. The
    /// setter's own `A` / `L` arguments are not known; the caller passes
    /// those of the player's room.
    pub fn set_from_server(
        &mut self,
        tables: &PeriodTables,
        index: i32,
        ticks: i32,
        eclipse: u8,
        a: u32,
        level: u32,
    ) -> Result<(), EnvError> {
        if !(0..=5).contains(&index) {
            return Err(EnvError::BadIndex(index));
        }
        if ticks < 0 {
            return Err(EnvError::NegativeTicks(ticks));
        }
        let flag = eclipse != 0;
        self.index = index;
        self.ticks = if ticks > self.day_ticks() { 0 } else { ticks };
        let t = if flag {
            &tables.eclipse
        } else {
            &tables.normal
        };
        self.kind = t[index as usize].kind;
        self.intensity(a, level);
        self.eclipse = flag;
        if flag {
            self.period_reset(tables);
            self.intensity(a, level);
            self.color(tables, a);
        }
        self.level_override(level);
        Ok(())
    }

    /// The period reset: by the record's eclipse
    /// flag, the normal or eclipse table entry of the current index gives
    /// the type and the ticks (its start × speed).
    pub fn period_reset(&mut self, tables: &PeriodTables) {
        let t = if self.eclipse {
            &tables.eclipse
        } else {
            &tables.normal
        };
        let p = t[self.index as usize];
        self.kind = p.kind;
        self.ticks = p.start.wrapping_mul(self.speed);
    }

    /// The ambient this environment gives a room: `I` = `+0x0C`,
    /// R, G, B = `+0x18..+0x1A`.
    pub fn ambient(&self) -> Ambient {
        Ambient {
            i: self.intensity as u8,
            r: self.r,
            g: self.g,
            b: self.b,
        }
    }
}

/// The S→C 0x53 arguments of the eclipse triggers: index 5,
/// ticks 0, eclipse 1.
pub const ECLIPSE_SET: (i32, i32, u8) = (5, 0, 1);

/// The eclipse pending flag: set by the Tainted Sun quest message while
/// no act is loaded, taken up by the next act 2 load.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct EclipseTrigger {
    pub pending: bool,
}

impl EclipseTrigger {
    /// S→C 0x5D (the quest message) with quest byte `quest` (@1) and flag
    /// byte `flags` (@2): quest 10 (Tainted Sun) and flag bit 0 → with a
    /// client act, `true` (call the setter with [`ECLIPSE_SET`] now);
    /// without one, the pending flag is set and `false` is returned.
    pub fn on_quest_message(&mut self, quest: u8, flags: u8, has_act: bool) -> bool {
        if quest != 10 || flags & 1 == 0 {
            return false;
        }
        if has_act {
            true
        } else {
            self.pending = true;
            false
        }
    }

    /// The act load (S→C 0x03): `true` (call the setter with
    /// [`ECLIPSE_SET`]) when the flag is pending and the loaded act is act 2
    /// (byte 1). The flag is not cleared here; it is left as is.
    pub fn on_act_load(&self, act: u8) -> bool {
        self.pending && act == 1
    }
}

/// The ambient of a room: the scripted override,
/// then the leveldefs `Intensity`, `Red`, `Green`, `Blue`, then the
/// environment; the first with any of R, G, B ≠ 0 wins.
pub fn room_ambient(scripted: Ambient, leveldefs: Ambient, env: &Environment) -> Ambient {
    if scripted.has_color() {
        scripted
    } else {
        room_ambient_without_override(leveldefs, env)
    }
}

/// The ambient without the override, used by the darkness
/// event.
pub fn room_ambient_without_override(leveldefs: Ambient, env: &Environment) -> Ambient {
    if leveldefs.has_color() {
        leveldefs
    } else {
        env.ambient()
    }
}
