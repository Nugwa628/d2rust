//! `dev = test`: dev commands on the server's standard input
//! (contracts 10.1).
//!
//! One line names the character and the command: `dev CHARACTER COMMAND`.
//! A test server also takes `set-progression CHARACTER N`, the same change
//! the offline `--set-progression` makes, while the character is out of a
//! game; `dev CHARACTER set-progression N` is the same line. The answer is
//! one line, `dev ok TEXT` or `dev err TEXT`. A server that doesn't run
//! `dev = test` refuses every line: `dev err disabled`.

use std::io::{BufRead, Write};

use d2_proto::dev::{self, DevCommand};

use crate::accounts::Accounts;
use crate::games::Games;

/// One line as typed: a dev console command, or the character's stored
/// progression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    /// `dev CHARACTER COMMAND`: a console command in that character's game.
    Dev {
        /// The character whose game takes the command.
        character: String,
        /// The command line, as the console writes it.
        command: String,
    },
    /// `set-progression CHARACTER N` (or the `dev` form): the character's
    /// progression bits, set while it is out of a game.
    SetProgression {
        /// The character whose stored progression changes.
        character: String,
        /// Progression bits, 0 to 31.
        progression: u8,
    },
}

/// One request, ready to run: the console command parsed, or the
/// progression to set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// A dev console command in `character`'s game.
    Dev {
        /// The character whose game takes the command.
        character: String,
        /// The parsed console command.
        command: DevCommand,
    },
    /// A character's stored progression bits.
    SetProgression {
        /// The character whose stored progression changes.
        character: String,
        /// Progression bits, 0 to 31.
        progression: u8,
    },
}

/// Splits one line; a blank line is not a request.
pub fn parse(line: &str) -> Result<Option<Line>, String> {
    let mut toks = line.split_whitespace();
    let Some(first) = toks.next() else {
        return Ok(None);
    };
    if first.eq_ignore_ascii_case("set-progression") {
        let character = toks.next().ok_or_else(set_progression_usage)?.to_owned();
        let n = toks.next().ok_or_else(set_progression_usage)?;
        if toks.next().is_some() {
            return Err(set_progression_usage());
        }
        return Ok(Some(Line::SetProgression {
            character,
            progression: progression(n)?,
        }));
    }
    if first != "dev" {
        return Err("expected `dev CHARACTER COMMAND` or `set-progression CHARACTER N`".into());
    }
    let Some(character) = toks.next() else {
        return Err("usage: dev CHARACTER COMMAND".into());
    };
    let command = toks.collect::<Vec<_>>().join(" ");
    if command.is_empty() {
        return Err("usage: dev CHARACTER COMMAND".into());
    }
    // `dev CHARACTER set-progression N` is the same line as
    // `set-progression CHARACTER N` (contracts 10.1).
    let mut words = command.split_whitespace();
    if words
        .next()
        .is_some_and(|k| k.eq_ignore_ascii_case("set-progression"))
    {
        let n = words.next().ok_or_else(set_progression_usage)?;
        if words.next().is_some() {
            return Err(set_progression_usage());
        }
        return Ok(Some(Line::SetProgression {
            character: character.to_owned(),
            progression: progression(n)?,
        }));
    }
    Ok(Some(Line::Dev {
        character: character.to_owned(),
        command,
    }))
}

/// The whole `set-progression` line's usage.
fn set_progression_usage() -> String {
    "usage: set-progression CHARACTER N (N 0..=31)".into()
}

/// Progression bits as typed.
fn progression(s: &str) -> Result<u8, String> {
    s.parse::<u8>()
        .ok()
        .filter(|n| *n <= 0x1F)
        .ok_or_else(|| format!("set-progression: {s:?} is not a number in 0..=31"))
}

/// The answer to one line; `None` when the line is blank.
pub fn answer(
    line: &str,
    enabled: bool,
    run: impl FnOnce(&Request) -> Result<String, String>,
) -> Option<String> {
    let t = line.trim();
    if t.is_empty() {
        return None;
    }
    if !enabled {
        return Some("dev err disabled".into());
    }
    let req = match parse(t) {
        Ok(Some(line)) => line,
        Ok(None) => return None,
        Err(e) => return Some(format!("dev err {e}")),
    };
    let req = match req {
        Line::Dev { character, command } => match dev::parse(&command) {
            Ok(Some(command)) => Request::Dev { character, command },
            Ok(None) => return Some(format!("dev err {command} is a client-only command")),
            Err(e) => return Some(format!("dev err {e}")),
        },
        Line::SetProgression {
            character,
            progression,
        } => Request::SetProgression {
            character,
            progression,
        },
    };
    match run(&req) {
        Ok(text) => Some(format!("dev ok {text}")),
        Err(text) => Some(format!("dev err {text}")),
    }
}

/// Runs one request: a dev command in the character's game (contracts
/// 10.1), or the character's progression bits, refused while it is in a
/// game.
pub fn run_command(games: &Games, accounts: &Accounts, req: &Request) -> Result<String, String> {
    match req {
        Request::Dev { character, command } => games.dev_command(character, command),
        Request::SetProgression {
            character,
            progression,
        } => {
            if games.character_in_game(&character.to_ascii_lowercase()) {
                return Err(format!(
                    "{character} is in a game; the progression is set while it is out"
                ));
            }
            accounts.set_progression(character, *progression)?;
            Ok(format!("{character} progression set to {progression}"))
        }
    }
}

/// Reads the server's standard input and answers each line on standard
/// output, until it ends.
pub fn serve(games: Games, accounts: Accounts, enabled: bool) {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        let Some(reply) = answer(&line, enabled, |req| run_command(&games, &accounts, req)) else {
            continue;
        };
        if writeln!(out, "{reply}").and_then(|()| out.flush()).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_names_the_character_and_the_command() {
        assert_eq!(
            parse("dev Sorceress gold 500"),
            Ok(Some(Line::Dev {
                character: "Sorceress".into(),
                command: "gold 500".into(),
            }))
        );
        assert_eq!(
            parse("  dev  Rogue   item Short Sword  "),
            Ok(Some(Line::Dev {
                character: "Rogue".into(),
                command: "item Short Sword".into(),
            }))
        );
        assert_eq!(parse("   "), Ok(None));
        assert!(parse("dev").unwrap_err().contains("usage"));
        assert!(parse("dev Sorceress").unwrap_err().contains("usage"));
        assert!(parse("gold 5").unwrap_err().contains("expected"));
    }

    #[test]
    fn the_progression_line_has_both_forms() {
        assert_eq!(
            parse("set-progression Frankpal 5"),
            Ok(Some(Line::SetProgression {
                character: "Frankpal".into(),
                progression: 5,
            }))
        );
        assert_eq!(
            parse("dev  Frankpal  set-progression  5"),
            Ok(Some(Line::SetProgression {
                character: "Frankpal".into(),
                progression: 5,
            }))
        );
        for bad in [
            "set-progression",
            "set-progression Frankpal",
            "set-progression Frankpal 5 now",
            "dev Frankpal set-progression",
            "dev Frankpal set-progression 5 now",
            "set-progression Frankpal 32",
            "dev Frankpal set-progression -1",
            "set-progression Frankpal x",
        ] {
            assert!(parse(bad).unwrap_err().contains("0..=31"), "{bad}");
        }
    }

    #[test]
    fn the_answer_is_one_dev_line() {
        assert_eq!(answer("", true, |_| unreachable!()), None);
        assert_eq!(answer("   ", true, |_| unreachable!()), None);
        let line = answer("dev Hero gold 5", true, |req| match req {
            Request::Dev { character, command } => {
                assert_eq!(character, "Hero");
                assert_eq!(command, &DevCommand::Gold(5));
                Err("no character named \"Hero\" in a game".into())
            }
            other => panic!("{other:?}"),
        });
        assert_eq!(
            line.as_deref(),
            Some("dev err no character named \"Hero\" in a game")
        );
        let line = answer("dev Hero heal", true, |req| match req {
            Request::Dev { command, .. } => {
                assert_eq!(command, &DevCommand::Heal);
                Ok("healed".into())
            }
            other => panic!("{other:?}"),
        });
        assert_eq!(line.as_deref(), Some("dev ok healed"));
        let line = answer("set-progression Hero 5", true, |req| match req {
            Request::SetProgression {
                character,
                progression,
            } => {
                assert_eq!((character.as_str(), *progression), ("Hero", 5));
                Ok(format!("{character} progression set to {progression}"))
            }
            other => panic!("{other:?}"),
        });
        assert_eq!(line.as_deref(), Some("dev ok Hero progression set to 5"));
        let line = answer("dev Hero fly", true, |_| Ok("unused".into()));
        assert!(
            line.as_deref()
                .is_some_and(|l| l.starts_with("dev err ") && l.contains("unknown command")),
            "{line:?}"
        );
        let line = answer("dev Hero save a1", true, |_| Ok("unused".into()));
        assert_eq!(
            line.as_deref(),
            Some("dev err save a1 is a client-only command")
        );
    }

    #[test]
    fn without_dev_test_every_line_is_refused() {
        assert_eq!(
            answer("dev Hero gold 5", false, |_| unreachable!()).as_deref(),
            Some("dev err disabled")
        );
        assert_eq!(
            answer("set-progression Hero 5", false, |_| unreachable!()).as_deref(),
            Some("dev err disabled")
        );
        assert_eq!(
            answer("gold 5", false, |_| unreachable!()).as_deref(),
            Some("dev err disabled")
        );
    }
}
