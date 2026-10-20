//! The soak input log `soak-log 1`: a header line naming the start, then
//! one concrete action per line, anchored to the soak step it runs
//! before. Generation resolves every random choice into one of these
//! lines, so a log replays the run without the generator (and a reduced
//! log replays what is left of it).

use std::fmt;

use crate::controls::Action as Key;

/// The format line (M20).
pub const FORMAT: &str = "soak-log 1";

/// One input the soak gives the play client. Every one goes through the
/// client (UI events, the bridge's interact or C→S intents the client's
/// panels send); none touches the server game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    /// `click L|R X Y`: press and release at frame pixel (X, Y) (the UI
    /// first, then the world, as a window click).
    Click { right: bool, x: i32, y: i32 },
    /// `move X Y`: the cursor to (X, Y).
    Move { x: i32, y: i32 },
    /// `key NAME`: the bound action of a key press (`controls` name).
    Key(Key),
    /// `interact UT GUID`: the bridge's unit interaction (a click that
    /// hovered the unit).
    Interact { ut: u8, guid: u32 },
    /// `pick GUID CURSOR`: C→S 0x16.
    Pick { guid: u32, cursor: bool },
    /// `drop GUID`: C→S 0x17.
    Drop { guid: u32 },
    /// `insert GUID X Y PAGE`: C→S 0x18.
    Insert {
        guid: u32,
        x: u32,
        y: u32,
        page: u32,
    },
    /// `remove GUID`: C→S 0x19.
    Remove { guid: u32 },
    /// `equip GUID BODY`: C→S 0x1A.
    Equip { guid: u32, body: u8 },
    /// `belt GUID SLOT`: C→S 0x23.
    Belt { guid: u32, slot: u32 },
    /// `usegrid GUID`: C→S 0x20 at the player's position.
    UseGrid { guid: u32 },
    /// `usebelt GUID`: C→S 0x26.
    UseBelt { guid: u32 },
    /// `wp GUID LEVEL`: C→S 0x49 (waypoint travel).
    Waypoint { guid: u32, level: u32 },
    /// `wait N`: N steps with no input.
    Wait(u32),
    /// `roundtrip`: a save/load round trip before this step's frame
    /// (`--roundtrip-at`, spec); the run goes on in the reloaded game.
    RoundTrip,
}

impl fmt::Display for Act {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Act::Click { right, x, y } => {
                write!(f, "click {} {x} {y}", if right { "R" } else { "L" })
            }
            Act::Move { x, y } => write!(f, "move {x} {y}"),
            Act::Key(k) => write!(f, "key {}", k.name()),
            Act::Interact { ut, guid } => write!(f, "interact {ut} {guid}"),
            Act::Pick { guid, cursor } => write!(f, "pick {guid} {}", u8::from(cursor)),
            Act::Drop { guid } => write!(f, "drop {guid}"),
            Act::Insert { guid, x, y, page } => write!(f, "insert {guid} {x} {y} {page}"),
            Act::Remove { guid } => write!(f, "remove {guid}"),
            Act::Equip { guid, body } => write!(f, "equip {guid} {body}"),
            Act::Belt { guid, slot } => write!(f, "belt {guid} {slot}"),
            Act::UseGrid { guid } => write!(f, "usegrid {guid}"),
            Act::UseBelt { guid } => write!(f, "usebelt {guid}"),
            Act::Waypoint { guid, level } => write!(f, "wp {guid} {level}"),
            Act::Wait(n) => write!(f, "wait {n}"),
            Act::RoundTrip => write!(f, "roundtrip"),
        }
    }
}

impl Act {
    /// Parses one action (the text after the step number).
    pub fn parse(text: &str) -> Result<Act, String> {
        let w: Vec<&str> = text.split_whitespace().collect();
        let n = |i: usize| -> Result<u32, String> {
            w.get(i)
                .ok_or_else(|| format!("`{text}`: missing argument {i}"))?
                .parse::<u32>()
                .map_err(|_| format!("`{text}`: argument {i} is not a number"))
        };
        // Screen points: a unit click's jitter can leave the frame
        // (`gen.rs`, x − 4), so they are signed.
        let i = |k: usize| -> Result<i32, String> {
            w.get(k)
                .ok_or_else(|| format!("`{text}`: missing argument {k}"))?
                .parse::<i32>()
                .map_err(|_| format!("`{text}`: argument {k} is not a number"))
        };
        let want = |k: usize| -> Result<(), String> {
            if w.len() == k {
                Ok(())
            } else {
                Err(format!("`{text}`: expected {} argument(s)", k - 1))
            }
        };
        let act = match w.first().copied() {
            Some("click") => {
                want(4)?;
                let right = match w[1] {
                    "L" => false,
                    "R" => true,
                    b => return Err(format!("`{text}`: button {b} is not L or R")),
                };
                Act::Click {
                    right,
                    x: i(2)?,
                    y: i(3)?,
                }
            }
            Some("move") => {
                want(3)?;
                Act::Move { x: i(1)?, y: i(2)? }
            }
            Some("key") => {
                want(2)?;
                Act::Key(
                    Key::from_name(w[1]).ok_or_else(|| format!("`{text}`: unknown key action"))?,
                )
            }
            Some("interact") => {
                want(3)?;
                Act::Interact {
                    ut: u8::try_from(n(1)?).map_err(|_| format!("`{text}`: unit type"))?,
                    guid: n(2)?,
                }
            }
            Some("pick") => {
                want(3)?;
                Act::Pick {
                    guid: n(1)?,
                    cursor: n(2)? != 0,
                }
            }
            Some("drop") => {
                want(2)?;
                Act::Drop { guid: n(1)? }
            }
            Some("insert") => {
                want(5)?;
                Act::Insert {
                    guid: n(1)?,
                    x: n(2)?,
                    y: n(3)?,
                    page: n(4)?,
                }
            }
            Some("remove") => {
                want(2)?;
                Act::Remove { guid: n(1)? }
            }
            Some("equip") => {
                want(3)?;
                Act::Equip {
                    guid: n(1)?,
                    body: u8::try_from(n(2)?).map_err(|_| format!("`{text}`: body"))?,
                }
            }
            Some("belt") => {
                want(3)?;
                Act::Belt {
                    guid: n(1)?,
                    slot: n(2)?,
                }
            }
            Some("usegrid") => {
                want(2)?;
                Act::UseGrid { guid: n(1)? }
            }
            Some("usebelt") => {
                want(2)?;
                Act::UseBelt { guid: n(1)? }
            }
            Some("wp") => {
                want(3)?;
                Act::Waypoint {
                    guid: n(1)?,
                    level: n(2)?,
                }
            }
            Some("roundtrip") => {
                want(1)?;
                Act::RoundTrip
            }
            Some("wait") => {
                want(2)?;
                Act::Wait(n(1)?)
            }
            _ => return Err(format!("`{text}`: unknown action")),
        };
        Ok(act)
    }
}

/// Where a run starts: the character and the level, all a replay needs
/// besides the actions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Start {
    /// `save=FILE`: a `.d2s`; else `new=CLASS`.
    pub save: Option<String>,
    /// `new=CLASS` (when no save).
    pub class: Option<String>,
    /// `game-seed=N`: the map seed (`play --seed`); none: the save's own.
    pub game_seed: Option<u32>,
    /// `difficulty=0..2`.
    pub difficulty: u8,
    /// `warp=LEVEL`: the start poke that moves the player to a level.
    pub warp: Option<u32>,
    /// `kit=0|1`: the start pokes that put the soak's item kit on the
    /// ground by the player (default 1).
    pub kit: bool,
}

impl fmt::Display for Start {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(s) = &self.save {
            write!(f, "save={s}")?;
        } else {
            write!(f, "new={}", self.class.as_deref().unwrap_or("sorceress"))?;
        }
        if let Some(s) = self.game_seed {
            write!(f, " game-seed={s}")?;
        }
        write!(f, " difficulty={}", self.difficulty)?;
        if let Some(l) = self.warp {
            write!(f, " warp={l}")?;
        }
        write!(f, " kit={}", u8::from(self.kit))
    }
}

impl Start {
    fn parse(words: &[&str]) -> Result<Start, String> {
        let mut s = Start {
            kit: true,
            ..Start::default()
        };
        for w in words {
            let (k, v) = w
                .split_once('=')
                .ok_or_else(|| format!("start field `{w}` is not key=value"))?;
            let num = || v.parse::<u32>().map_err(|_| format!("`{w}`: not a number"));
            match k {
                "save" => s.save = Some(v.to_owned()),
                "new" => s.class = Some(v.to_owned()),
                "game-seed" => s.game_seed = Some(num()?),
                "difficulty" => s.difficulty = num()?.min(2) as u8,
                "warp" => s.warp = Some(num()?),
                "kit" => s.kit = num()? != 0,
                _ => return Err(format!("unknown start field `{k}`")),
            }
        }
        Ok(s)
    }
}

/// A whole log: the start, the generator seed it came from (0: written
/// by hand or reduced) and the actions with their steps.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SoakLog {
    pub start: Start,
    pub seed: u64,
    pub acts: Vec<(u32, Act)>,
}

impl SoakLog {
    /// The header line.
    pub fn header(&self) -> String {
        format!(
            "{FORMAT} tick_rate={} seed={} {}",
            d2_sim::tick::TickRate::GAME.hz(),
            self.seed,
            self.start
        )
    }

    /// One action line.
    pub fn line(step: u32, act: &Act) -> String {
        format!("{step} {act}")
    }

    pub fn to_text(&self) -> String {
        let mut s = self.header();
        s.push('\n');
        for (step, a) in &self.acts {
            s.push_str(&Self::line(*step, a));
            s.push('\n');
        }
        s
    }

    /// Parses a log. Steps never go back; `#` starts a comment.
    pub fn parse(text: &str) -> Result<SoakLog, String> {
        let mut lines = text
            .lines()
            .enumerate()
            .map(|(i, l)| (i + 1, l.split('#').next().unwrap_or("").trim()))
            .filter(|(_, l)| !l.is_empty());
        let (_, head) = lines.next().ok_or("empty log")?;
        let rest = head
            .strip_prefix(FORMAT)
            .ok_or_else(|| format!("line 1 must start with `{FORMAT}`"))?;
        let mut words: Vec<&str> = rest.split_whitespace().collect();
        if words.iter().filter(|w| w.starts_with("tick_rate=")).count() > 1 {
            return Err("recording: duplicate tick_rate".into());
        }
        let mut rate = None;
        if let Some(p) = words.iter().position(|w| w.starts_with("tick_rate=")) {
            rate = Some(
                words.remove(p)[10..]
                    .parse()
                    .map_err(|_| "invalid tick_rate")?,
            );
        }
        crate::recording::validate(rate, d2_sim::tick::TickRate::GAME)?;
        let mut seed = 0;
        if let Some(p) = words.iter().position(|w| w.starts_with("seed=")) {
            seed = words[p][5..]
                .parse()
                .map_err(|_| "header seed= is not a number".to_owned())?;
            words.remove(p);
        }
        let start = Start::parse(&words)?;
        let mut acts = Vec::new();
        let mut last = 0;
        for (no, l) in lines {
            let (step, a) = l
                .split_once(' ')
                .ok_or_else(|| format!("line {no}: `STEP ACTION`"))?;
            let step: u32 = step
                .parse()
                .map_err(|_| format!("line {no}: step `{step}` is not a number"))?;
            if step < last {
                return Err(format!("line {no}: step {step} goes back (after {last})"));
            }
            last = step;
            acts.push((step, Act::parse(a).map_err(|e| format!("line {no}: {e}"))?));
        }
        Ok(SoakLog { start, seed, acts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_round_trips_through_text() {
        let acts = [
            Act::Click {
                right: true,
                x: 10,
                y: 599,
            },
            Act::Click {
                right: false,
                x: -1,
                y: 101,
            },
            Act::Move { x: 1, y: 2 },
            Act::Key(Key::BeltSlot3),
            Act::Interact { ut: 2, guid: 77 },
            Act::Pick {
                guid: 5,
                cursor: true,
            },
            Act::Drop { guid: 5 },
            Act::Insert {
                guid: 5,
                x: 1,
                y: 2,
                page: 4,
            },
            Act::Remove { guid: 5 },
            Act::Equip { guid: 5, body: 4 },
            Act::Belt { guid: 5, slot: 3 },
            Act::UseGrid { guid: 5 },
            Act::UseBelt { guid: 5 },
            Act::Waypoint { guid: 9, level: 40 },
            Act::Wait(12),
            Act::RoundTrip,
        ];
        let log = SoakLog {
            start: Start {
                save: None,
                class: Some("amazon".into()),
                game_seed: Some(7),
                difficulty: 1,
                warp: Some(3),
                kit: true,
            },
            seed: 42,
            acts: acts
                .iter()
                .cloned()
                .enumerate()
                .map(|(i, a)| (i as u32, a))
                .collect(),
        };
        let back = SoakLog::parse(&log.to_text()).unwrap();
        assert_eq!(back, log);
    }

    #[test]
    fn bad_lines_are_refused_with_their_line() {
        let e = SoakLog::parse("soak-log 1 tick_rate=60 new=amazon\n3 click L 1\n").unwrap_err();
        assert!(e.starts_with("line 2"), "{e}");
        let e =
            SoakLog::parse("soak-log 1 tick_rate=60 new=amazon\n3 wait 1\n2 wait 1\n").unwrap_err();
        assert!(e.contains("goes back"), "{e}");
        assert!(SoakLog::parse("soak-log 2\n").is_err());
        assert!(SoakLog::parse("soak-log 1 tick_rate=60 new=amazon\n1 key no_such\n").is_err());
    }
}
