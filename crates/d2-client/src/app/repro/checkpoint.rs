//! Goal-boundary checkpoints with the warm session.
//!
//! D2RUST's sim is deterministic from the game data, the seed, the
//! character and the inputs. A checkpoint therefore stores the run's
//! canonical scenario prefix plus the server `state-1` line at the
//! boundary: restoring replays the prefix on a warm [`Session`] (no data
//! reload) and the state bytes must match exactly. The same bytes make a
//! silent divergence a hard error instead of a wrong result.
//!
//! The format is `checkpoint 1` (JSON): `at` (script ticks after tick 0),
//! `script` (the canonical `scenario 2` prefix) and `state` (the `state-1`
//! line). [`Checkpoint::chain`] appends a step-only tail (a generated
//! goal), shifting its ticks past the boundary; [`Checkpoint::save`] and
//! [`Checkpoint::restore`] run the prefix and check the state.
//!
//! This is the API G3 uses: `save` after a passed goal, `chain` to build
//! the next goal's scenario, `restore` to prove the boundary state.
use super::*;
use crate::app::{save, soak::roundtrip};
use crate::scenario_script::{Scenario, Step, SCENARIO_FORMAT_V2};

/// The checkpoint file's format word.
pub const FORMAT: &str = "checkpoint 1";

/// The artifact [`Checkpoint::capture`] reads (written by
/// [`RunOpts::capture_state`]).
pub const STATE_ARTIFACT: &str = "state-end.jsonl";

/// A saved goal boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    /// Script ticks after tick 0.
    pub at: u32,
    /// The canonical [`Scenario::to_text`] of the prefix.
    pub script: String,
    /// The server `state-1` line at the boundary.
    pub state: String,
}

/// A passed restore: the run's report and the state it reproduced.
#[derive(Clone, Debug)]
pub struct Restored {
    pub report: ReproReport,
    pub state: String,
}

impl Checkpoint {
    /// Run `s` up to `at` on the warm `session` and save the boundary.
    /// Artifacts go under `opts.out/checkpoint-<at>`.
    pub fn save(
        session: &mut Session,
        s: &Scenario,
        at: u32,
        opts: &RunOpts,
    ) -> Result<Self, ReproError> {
        let prefix = prefix(s, at)?;
        let mut local = opts.clone();
        local.out = opts.out.join(format!("checkpoint-{at}"));
        local.capture_state = true;
        local.queries.clear();
        local.query_at.clear();
        let report = session.run(&prefix, &local)?;
        if report.exit != exit::PASS {
            return Err(ReproError::Invalid(format!(
                "checkpoint at {at}: the prefix run did not pass"
            )));
        }
        Self::capture(&local.out, &prefix, at)
    }

    /// Build a checkpoint from the `state-end.jsonl` of a run that ended at
    /// `at` (`s.end == at`). Use it after a goal run with
    /// [`RunOpts::capture_state`] set, so the goal is not run twice.
    pub fn capture(out: &Path, s: &Scenario, at: u32) -> Result<Self, ReproError> {
        let prefix = prefix(s, at)?;
        if s.end != at {
            return Err(ReproError::Invalid(format!(
                "checkpoint capture at {at}: the run ended at {}",
                s.end
            )));
        }
        let state = read_state(&out.join(STATE_ARTIFACT))?;
        Ok(Self {
            at,
            script: prefix.to_text(),
            state,
        })
    }

    /// Replay the prefix on the warm `session` and prove the state bytes.
    /// Artifacts go under `opts.out/restore-<at>`.
    pub fn restore(&self, session: &mut Session, opts: &RunOpts) -> Result<Restored, ReproError> {
        let s = self.scenario()?;
        let mut local = opts.clone();
        local.out = opts.out.join(format!("restore-{}", self.at));
        local.capture_state = true;
        local.queries.clear();
        local.query_at.clear();
        let report = session.run(&s, &local)?;
        if report.exit != exit::PASS {
            return Err(ReproError::Invalid(format!(
                "checkpoint at {}: the prefix run did not pass",
                self.at
            )));
        }
        let state = read_state(&local.out.join(STATE_ARTIFACT))?;
        if state != self.state {
            return Err(ReproError::Run(anyhow!(
                "checkpoint at {}: state differs ({})",
                self.at,
                state_difference(&self.state, &state)
            )));
        }
        Ok(Restored { report, state })
    }

    /// The prefix scenario.
    pub fn scenario(&self) -> Result<Scenario, ReproError> {
        let s = Scenario::parse(&self.script)
            .map_err(|e| ReproError::Invalid(format!("checkpoint script: {e}")))?;
        if s.version != SCENARIO_FORMAT_V2 {
            return Err(ReproError::Invalid(
                "checkpoint script is not `scenario 2`".into(),
            ));
        }
        if s.end != self.at {
            return Err(ReproError::Invalid(format!(
                "checkpoint at {}: script ends at {}",
                self.at, s.end
            )));
        }
        Ok(s)
    }

    /// The checkpoint with `tail` appended after the boundary: the tail's
    /// steps move by `at`, and its character, save, seed and difficulty
    /// must match the checkpoint's (only the steps are taken).
    pub fn chain(&self, tail: &Scenario) -> Result<Scenario, ReproError> {
        let mut s = self.scenario()?;
        if tail.version != SCENARIO_FORMAT_V2 {
            return Err(ReproError::Invalid("the tail is not `scenario 2`".into()));
        }
        for (what, same) in [
            ("character", tail.character == s.character),
            ("save", tail.save == s.save),
            ("save file", tail.save_file == s.save_file),
            ("tick_rate", tail.tick_rate == s.tick_rate),
            ("seed", tail.seed == s.seed),
            ("init", tail.init == s.init),
            ("difficulty", tail.difficulty == s.difficulty),
            ("expansion", tail.expansion == s.expansion),
            ("variant", tail.variant == s.variant),
        ] {
            if !same {
                return Err(ReproError::Invalid(format!(
                    "the tail's {what} differs from the checkpoint's"
                )));
            }
        }
        s.end = s
            .end
            .checked_add(tail.end)
            .ok_or_else(|| ReproError::Invalid("chained scenario is too long".into()))?;
        for step in &tail.steps {
            let tick = self
                .at
                .checked_add(step.tick)
                .ok_or_else(|| ReproError::Invalid("chained step is too late".into()))?;
            s.steps.push(Step {
                tick,
                msg: step.msg.clone(),
            });
        }
        s.steps.sort_by_key(|step| step.tick);
        s.allow.extend(tail.allow.iter().cloned());
        if let Some(n) = tail.snapshot_every {
            s.snapshot_every = Some(s.snapshot_every.map_or(n, |m| m.min(n)));
        }
        for &t in &tail.snapshot_at {
            if let Some(t) = self.at.checked_add(t) {
                s.snapshot_at.push(t);
            }
        }
        s.snapshot_at.sort_unstable();
        s.snapshot_at.dedup();
        Ok(s)
    }

    /// Fork one case from the checkpoint: a fresh server game and client
    /// join on the warm `session` (game data is not reloaded), with
    /// `tail`'s steps appended at the boundary.
    pub fn fork_case(
        &self,
        session: &mut Session,
        tail: &Scenario,
        opts: &RunOpts,
    ) -> Result<ReproReport, ReproError> {
        let s = self.chain(tail)?;
        session.run(&s, opts)
    }

    /// The file's bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>, ReproError> {
        Ok(serde_json::to_vec_pretty(&self.to_json())?)
    }

    /// The file's JSON.
    pub fn to_json(&self) -> Value {
        json!({
            "format": FORMAT,
            "at": self.at,
            "script": self.script,
            "state": self.state,
        })
    }

    /// Parses a checkpoint file (`checkpoint 1`).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ReproError> {
        let v: Value = serde_json::from_slice(bytes)
            .map_err(|e| ReproError::Invalid(format!("checkpoint: {e}")))?;
        if v["format"] != FORMAT {
            return Err(ReproError::Invalid(format!(
                "checkpoint format {:?}, want {FORMAT}",
                v["format"]
            )));
        }
        let at = v["at"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| ReproError::Invalid("checkpoint: no `at` tick".into()))?;
        let script = v["script"]
            .as_str()
            .ok_or_else(|| ReproError::Invalid("checkpoint: no `script`".into()))?
            .to_owned();
        let state = v["state"]
            .as_str()
            .ok_or_else(|| ReproError::Invalid("checkpoint: no `state`".into()))?
            .to_owned();
        if state.is_empty() {
            return Err(ReproError::Invalid("checkpoint: empty `state`".into()));
        }
        Ok(Self { at, script, state })
    }

    /// Writes the checkpoint file.
    pub fn write(&self, path: &Path) -> Result<(), ReproError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_bytes()?)?;
        Ok(())
    }

    /// Reads a checkpoint file.
    pub fn read(path: &Path) -> Result<Self, ReproError> {
        Self::from_bytes(&std::fs::read(path)?)
    }

    /// The file's size.
    pub fn size_bytes(&self) -> Result<usize, ReproError> {
        Ok(self.to_bytes()?.len())
    }
}

/// The prefix of `s` up to `at` ticks: setup kept, later steps dropped.
fn prefix(s: &Scenario, at: u32) -> Result<Scenario, ReproError> {
    if s.version != SCENARIO_FORMAT_V2 {
        return Err(ReproError::Invalid(
            "checkpoints need a `scenario 2` script".into(),
        ));
    }
    if at > s.end {
        return Err(ReproError::Invalid(format!(
            "checkpoint at {at}: the scenario ends at {}",
            s.end
        )));
    }
    let mut p = s.clone();
    p.end = at;
    p.steps.retain(|step| step.tick <= at);
    p.snapshot_at.retain(|&t| t <= at);
    Ok(p)
}

/// The `state-1` line in a `state-end.jsonl` artifact.
fn read_state(path: &Path) -> Result<String, ReproError> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    text.lines()
        .rev()
        .find(|line| line.starts_with("{\"k\":\"snap\""))
        .map(str::to_owned)
        .ok_or_else(|| {
            ReproError::Run(anyhow!(
                "{}: no `state-1` line (was capture_state set?)",
                path.display()
            ))
        })
}

/// The first difference between two state lines, for an error message.
fn state_difference(want: &str, got: &str) -> String {
    if want.len() != got.len() {
        return format!("length {} != {}", want.len(), got.len());
    }
    match want.bytes().zip(got.bytes()).position(|(a, b)| a != b) {
        Some(i) => format!(
            "byte {i}: want 0x{:02x}, got 0x{:02x}",
            want.as_bytes()[i],
            got.as_bytes()[i]
        ),
        None => "identical".into(),
    }
}

// ---------------------------------------------------------------------------
// Q3: read-only save round trips authored with `at N checkpoint`.
//
// Unlike the goal-boundary checkpoints above, these capture no script. At
// the authored tick they read the live save state, apply it to the base
// character save, write and read the bytes, join an independent game and
// compare the live state and its second save with the soak round-trip rules.

/// Whether a temporary state list (a buff with an expiry) is attached to the
/// local player. A save carries the character's base stats, not temporary
/// state-list bonuses (Battle Orders, Shout, ...): reloading a save taken
/// while one is active restores the unbuffed maxima and clamps the vitals,
/// so comparing that live state with the reload would report a difference
/// the save cannot carry.
fn temporary_effect(host: &Host) -> Result<bool> {
    host.server()
        .lock()
        .map_err(|_| anyhow!("server lock"))?
        .with(|link| {
            let sim = &mut link.host_mut().game;
            let Some((player, _)) = crate::app::single_player::local_player(sim) else {
                return false;
            };
            let stats = &sim.events.action.sys.stats;
            let Some(list) = stats.unit_list(player) else {
                return false;
            };
            let (mut active, _) = stats.heads(list);
            while let Some(list) = active {
                if stats.expire(list) > 0 {
                    return true;
                }
                active = stats.next(list);
            }
            false
        })
        .map_err(|e| anyhow!("{e:?}"))
}

fn read_live(host: &Host) -> Result<save::Live> {
    host.server()
        .lock()
        .map_err(|_| anyhow!("server lock"))?
        .with(|link| save::read_live(&mut link.host_mut().game))
        .map_err(|e| anyhow!("{e:?}"))?
        .map_err(Into::into)
}

fn compare_live(
    host: &Host,
    session: &Session,
    base: &D2s,
    s: &Scenario,
) -> Result<Vec<crate::app::checks::Violation>> {
    let GameData::Live(live) = &session.data;
    let before = read_live(host)?;
    let first = save::apply_live(base, &before, 0);
    let bytes = d2s::write(&first, &live.save)?;
    let first = live.read_save(&bytes, s.difficulty.index())?;
    let character = Character::Save(
        Box::new(first.clone()),
        d2_server::adapters::character::LoadContext {
            difficulty: s.difficulty.index(),
            map_seed_applies: false,
        },
    );
    let again = Host::start_with(session.data.clone(), character, None, false, Some(s.seed))?;
    let after = read_live(&again)?;
    let mut violations = roundtrip::diff_live(&before, &after);
    let second = save::apply_live(&first, &after, 0);
    let second = live.read_save(&d2s::write(&second, &live.save)?, s.difficulty.index())?;
    violations.extend(roundtrip::diff_saves(&first, &second));
    Ok(violations)
}

pub(super) fn tick(
    host: &Host,
    session: &Session,
    base: &D2s,
    s: &Scenario,
    checkpoints: &[u32],
    t: u32,
    log: &StepLog,
) {
    for _ in checkpoints.iter().filter(|&&at| at == t) {
        if temporary_effect(host).unwrap_or(false) {
            push(
                log,
                Some(t),
                "checkpoint",
                "roundtrip".into(),
                true,
                "temporary effect active: save comparison skipped".into(),
            );
            continue;
        }
        match compare_live(host, session, base, s) {
            Ok(v) if v.is_empty() => push(
                log,
                Some(t),
                "checkpoint",
                "roundtrip".into(),
                true,
                "live state and second save match".into(),
            ),
            Ok(v) => {
                for v in v {
                    push(log, Some(t), "checkpoint", v.signature(), false, v.detail);
                }
            }
            Err(e) => push(
                log,
                Some(t),
                "checkpoint",
                "roundtrip:error".into(),
                false,
                format!("{e:#}"),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = "scenario 2
tick_rate 60
name checkpoint
seed 7
end 20
char class 0
at 2 input click 440 300
at 10 input click 400 300
at 20 input click 440 300
";

    fn scenario() -> Scenario {
        Scenario::parse(SCRIPT).unwrap()
    }

    #[test]
    fn the_prefix_keeps_setup_and_steps_up_to_the_boundary() {
        let p = prefix(&scenario(), 10).unwrap();
        assert_eq!(p.end, 10);
        assert_eq!(p.steps.iter().map(|s| s.tick).collect::<Vec<_>>(), [2, 10]);
        assert_eq!(p.character, scenario().character);
    }

    #[test]
    fn a_checkpoint_past_the_end_is_invalid() {
        let e = prefix(&scenario(), 21).unwrap_err();
        assert!(e.to_string().contains("ends at 20"), "{e}");
    }

    #[test]
    fn chain_shifts_tail_steps_and_extends_the_end() {
        let cp = Checkpoint {
            at: 10,
            script: prefix(&scenario(), 10).unwrap().to_text(),
            state: "{\"k\":\"snap\"}".into(),
        };
        let tail = Scenario::parse(
            "scenario 2
tick_rate 60
name goal
seed 7
end 5
char class 0
at 3 input click 420 320
by 5 check player.mode == town_neutral
",
        )
        .unwrap();
        let merged = cp.chain(&tail).unwrap();
        assert_eq!(merged.end, 15);
        assert_eq!(
            merged.steps.iter().map(|s| s.tick).collect::<Vec<_>>(),
            [2, 10, 13, 15]
        );
        assert_eq!(merged.character, scenario().character);
    }

    #[test]
    fn a_tail_cannot_bring_its_own_character_or_seed() {
        let cp = Checkpoint {
            at: 10,
            script: prefix(&scenario(), 10).unwrap().to_text(),
            state: "{\"k\":\"snap\"}".into(),
        };
        for text in [
            "scenario 2\ntick_rate 60\nname t\nseed 7\nend 1\nchar class 1\n",
            "scenario 2\ntick_rate 60\nname t\nseed 8\nend 1\nchar class 0\n",
            "scenario 2\ntick_rate 25\nname t\nseed 7\nend 1\nchar class 0\n",
        ] {
            let tail = Scenario::parse(text).unwrap();
            assert!(cp.chain(&tail).is_err(), "{text}");
        }
    }

    #[test]
    fn the_file_round_trips_and_rejects_another_format() {
        let cp = Checkpoint {
            at: 10,
            script: prefix(&scenario(), 10).unwrap().to_text(),
            state: "{\"k\":\"snap\",\"f\":8,\"seed\":[1,666],\"units\":[]}".into(),
        };
        assert_eq!(Checkpoint::from_bytes(&cp.to_bytes().unwrap()).unwrap(), cp);
        let bad = br#"{"format":"checkpoint 2","at":1,"script":"x","state":"y"}"#;
        assert!(Checkpoint::from_bytes(bad).is_err());
    }

    #[test]
    fn a_checkpoint_script_names_the_same_tick() {
        let cp = Checkpoint {
            at: 10,
            script: prefix(&scenario(), 5).unwrap().to_text(),
            state: "{\"k\":\"snap\"}".into(),
        };
        assert!(cp.scenario().is_err());
    }
}
