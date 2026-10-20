//! Keeping the player's own paths and name out of a bug report.
//!
//! The game's log and state text can mention files by their full path, and a
//! full path on a player's PC usually holds their home folder and with it
//! their account name (`C:\Users\<name>\Saved Games\...`). A report must
//! never carry either, so every text a report holds (the note, the log tail,
//! the state dump) passes through a [`Redactor`] before it is written, and
//! the launcher runs it again before sending:
//! - the home folder, in any spelling a log may show it (`\`, `/`, or `\\` as
//!   in escaped debug output; any letter case), becomes `[home]`;
//! - the account name, where it stands as a word of its own, becomes `[user]`.
//!
//! Redaction only replaces text; it never adds anything the player hasn't
//! seen, so the launcher's preview shows exactly what is sent. The marks use
//! square brackets rather than angle brackets, so they read as plain text
//! and are never mistaken for markup.

use crate::state::STATE_HEADER;
use crate::{clip, log_tail, state_text, Bundle, FileKind, ReportError, MAX_NOTE_CHARS};

/// What a home folder becomes in a report.
pub const HOME_MARK: &str = "[home]";
/// What the account name becomes in a report.
pub const USER_MARK: &str = "[user]";

/// Account names shorter than this are left alone: one or two letters as a
/// word would wipe out ordinary text, and they identify no one.
const MIN_USER_CHARS: usize = 3;
/// Home folders shorter than this (`/`, `C:\`) are left alone for the same
/// reason.
const MIN_HOME_CHARS: usize = 4;

/// Replaces one player's home folder and account name in report text; see
/// the module header.
#[derive(Clone, Debug, Default)]
pub struct Redactor {
    /// Home folder spellings, lowercased, longest first.
    homes: Vec<String>,
    /// The account name, lowercased, if long enough to redact.
    user: Option<String>,
}

impl Redactor {
    /// A redactor for this home folder and account name. Either may be
    /// absent; a redactor with neither changes nothing.
    pub fn new(home: Option<&str>, user: Option<&str>) -> Redactor {
        let mut homes = Vec::new();
        if let Some(h) = home {
            let h = h.trim().trim_end_matches(['\\', '/']).to_ascii_lowercase();
            if h.chars().count() >= MIN_HOME_CHARS {
                let back = h.replace('/', "\\");
                let fwd = h.replace('\\', "/");
                let double = back.replace('\\', "\\\\");
                for s in [double, back, fwd] {
                    if !homes.contains(&s) {
                        homes.push(s);
                    }
                }
                homes.sort_by_key(|s| std::cmp::Reverse(s.len()));
            }
        }
        let user = user
            .map(|u| u.trim().to_ascii_lowercase())
            .filter(|u| u.chars().count() >= MIN_USER_CHARS);
        Redactor { homes, user }
    }

    /// A redactor for the player running this program, from the usual
    /// environment variables (`USERPROFILE` or `HOME`, `USERNAME` or `USER`).
    pub fn from_env() -> Redactor {
        let var = |names: &[&str]| {
            names
                .iter()
                .find_map(|n| std::env::var(n).ok().filter(|v| !v.trim().is_empty()))
        };
        Redactor::new(
            var(&["USERPROFILE", "HOME"]).as_deref(),
            var(&["USERNAME", "USER"]).as_deref(),
        )
    }

    /// `text` with the home folder and account name replaced.
    pub fn redact(&self, text: &str) -> String {
        let mut out = text.to_string();
        for h in &self.homes {
            out = replace_ascii_ci(&out, h, HOME_MARK, false);
        }
        if let Some(u) = &self.user {
            out = replace_ascii_ci(&out, u, USER_MARK, true);
        }
        out
    }

    /// The bundle with its note, log and state redacted. The screenshot is
    /// left as it is (the player previews it and may leave it out).
    ///
    /// Redaction never makes a valid bundle invalid. A mark can be longer
    /// than the name it replaces, so the redacted log is cut back to the
    /// log limits (oldest lines go first) and each state line to the state
    /// line limit. The state dump's header line is the format's own and is
    /// kept as written, so an account named like a word in it (`state`)
    /// can't break the header.
    pub fn redact_bundle(&self, bundle: Bundle) -> Result<Bundle, ReportError> {
        let note = clip(&self.redact(&bundle.manifest().note), MAX_NOTE_CHARS);
        let mut out = bundle.with_note(note)?;
        if let Some(bytes) = out.file(FileKind::Log) {
            let text = String::from_utf8_lossy(bytes);
            let clean = self.redact(&text);
            if clean != text {
                let lines: Vec<&str> = clean.lines().collect();
                out = out.with_file(FileKind::Log, log_tail(&lines))?;
            }
        }
        if let Some(bytes) = out.file(FileKind::State) {
            let text = String::from_utf8_lossy(bytes);
            let mut lines = text.lines();
            let header = lines.next();
            let clean: Vec<String> = lines.map(|l| self.redact(l)).collect();
            let rebuilt = match header {
                Some(STATE_HEADER) => state_text(&clean),
                // Not a dump this format wrote: redact it whole, and let the
                // bundle's own check decide.
                _ => self.redact(&text).into_bytes(),
            };
            if rebuilt != bytes {
                out = out.with_file(FileKind::State, rebuilt)?;
            }
        }
        Ok(out)
    }
}

/// `hay` with every ASCII-case-insensitive match of the lowercase `needle`
/// replaced by `with`. With `word`, a match counts only when the characters
/// on both sides are not letters, digits or `_`.
fn replace_ascii_ci(hay: &str, needle: &str, with: &str, word: bool) -> String {
    // ASCII lowercasing keeps every byte offset, so offsets found in `lower`
    // are offsets in `hay`, and always on character boundaries.
    let lower = hay.to_ascii_lowercase();
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::with_capacity(hay.len());
    let mut done = 0;
    let mut from = 0;
    while let Some(i) = lower[from..].find(needle).map(|i| i + from) {
        let end = i + needle.len();
        let bounded = !word
            || (!hay[..i].chars().next_back().is_some_and(is_word)
                && !hay[end..].chars().next().is_some_and(is_word));
        if bounded {
            out.push_str(&hay[done..i]);
            out.push_str(with);
            done = end;
            from = end;
        } else {
            from = i + hay[i..].chars().next().map_or(1, char::len_utf8);
        }
    }
    out.push_str(&hay[done..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{log_tail, state_text, Meta};

    fn win() -> Redactor {
        Redactor::new(Some(r"C:\Users\Kai Smith"), Some("Kai Smith"))
    }

    #[test]
    fn every_spelling_of_the_home_folder_goes() {
        let r = win();
        for line in [
            r"loading C:\Users\Kai Smith\Saved Games\D2RUST\Hero.d2s",
            r"loading c:\users\KAI SMITH\Saved Games\D2RUST\Hero.d2s",
            "loading C:/Users/Kai Smith/Saved Games/D2RUST/Hero.d2s",
            r#"path: "C:\\Users\\Kai Smith\\Saved Games\\D2RUST\\Hero.d2s""#,
        ] {
            let out = r.redact(line);
            assert!(out.contains("[home]"), "{out}");
            assert!(!out.to_ascii_lowercase().contains("kai"), "{out}");
            assert!(out.contains("Hero.d2s"), "{out}");
        }
    }

    #[test]
    fn the_name_goes_only_as_a_word() {
        let r = Redactor::new(None, Some("ann"));
        assert_eq!(r.redact("Ann said hi"), "[user] said hi");
        assert_eq!(r.redact("planned annually"), "planned annually");
        assert_eq!(r.redact("/home/ann/x"), "/home/[user]/x");
        assert_eq!(r.redact("ann_x annx ann"), "ann_x annx [user]");
    }

    #[test]
    fn short_or_missing_values_change_nothing() {
        let text = r"C:\x and al, /home";
        for r in [
            Redactor::default(),
            Redactor::new(Some("/"), Some("al")),
            Redactor::new(Some(r"C:\"), None),
        ] {
            assert_eq!(r.redact(text), text);
        }
    }

    #[test]
    fn unicode_text_around_matches_survives() {
        let r = Redactor::new(Some("/home/jörg"), Some("jörg"));
        // Non-ASCII letters match case-sensitively (ASCII case folding only).
        assert_eq!(r.redact("ü /home/jörg/ü ✓"), "ü [home]/ü ✓");
        assert_eq!(r.redact("é jörg é"), "é [user] é");
    }

    #[test]
    fn a_bundle_comes_out_without_the_home_folder_or_name() {
        let r = win();
        let b = Bundle::new(Meta {
            game_version: "0.1.0.1".into(),
            launcher_version: None,
            os: "windows".into(),
            time_unix: 1,
            note: r"crash after Kai Smith saved to C:\Users\Kai Smith\x".into(),
        })
        .unwrap()
        .with_file(
            FileKind::Log,
            log_tail(&[r"INFO a: new character stub: C:\Users\Kai Smith\Saved Games\Hero.d2s"]),
        )
        .unwrap()
        .with_file(FileKind::State, state_text(&["character Hero.d2s"]))
        .unwrap();
        let out = r.redact_bundle(b).unwrap();
        let all = format!(
            "{}{}{}",
            String::from_utf8_lossy(&out.manifest().to_json()),
            String::from_utf8_lossy(out.file(FileKind::Log).unwrap()),
            String::from_utf8_lossy(out.file(FileKind::State).unwrap()),
        );
        assert!(!all.to_ascii_lowercase().contains("kai"), "{all}");
        assert!(all.contains("[home]") && all.contains("[user]"));
        // The manifest's hashes follow the redacted files.
        let dir = std::env::temp_dir().join(format!("d2rust-redact-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        out.write_dir(&dir).unwrap();
        assert_eq!(Bundle::read_dir(&dir).unwrap(), out);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn bundle_with(log: Vec<u8>, state: Vec<u8>) -> Bundle {
        Bundle::new(Meta {
            game_version: "0.1.0.1".into(),
            launcher_version: None,
            os: "windows".into(),
            time_unix: 1,
            note: String::new(),
        })
        .unwrap()
        .with_file(FileKind::Log, log)
        .unwrap()
        .with_file(FileKind::State, state)
        .unwrap()
    }

    #[test]
    fn an_account_named_like_the_state_header_keeps_the_header() {
        let r = Redactor::new(None, Some("state"));
        let b = bundle_with(
            log_tail(&["INFO a: state saved"]),
            state_text(&["character state.d2s"]),
        );
        let out = r.redact_bundle(b).unwrap();
        assert_eq!(
            out.file(FileKind::State).unwrap(),
            b"d2rust-state 1\ncharacter [user].d2s\n"
        );
        assert_eq!(out.file(FileKind::Log).unwrap(), b"INFO a: [user] saved\n");
    }

    #[test]
    fn marks_longer_than_the_name_still_fit_the_limits() {
        // "[user]" is twice as long as "abc": full-length lines grow past
        // the limits and are cut back rather than rejected.
        let r = Redactor::new(None, Some("abc"));
        let state_line = "abc ".repeat(crate::state::MAX_STATE_LINE_CHARS / 4);
        let log_line = "abc ".repeat(1000);
        let log_lines: Vec<&str> = vec![log_line.as_str(); crate::MAX_LOG_LINES];
        let b = bundle_with(log_tail(&log_lines), state_text(&[state_line.as_str()]));
        let out = r.redact_bundle(b).unwrap();
        let log = out.file(FileKind::Log).unwrap();
        assert!(log.len() as u64 <= crate::MAX_LOG_BYTES);
        assert!(!String::from_utf8_lossy(log).contains("abc"));
        let state = crate::StateDump::parse(out.file(FileKind::State).unwrap()).unwrap();
        assert_eq!(
            state.lines[0].chars().count(),
            crate::state::MAX_STATE_LINE_CHARS
        );
    }
}
