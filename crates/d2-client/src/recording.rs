//! Rate contracts for tick-indexed replay inputs. Missing metadata is a
//! legacy 25 Hz recording; replay must never silently change its speed.

use d2_sim::tick::TickRate;

/// Require the recorded rate to agree with the consuming game.
pub fn validate(rate: Option<u32>, expected: TickRate) -> Result<(), String> {
    match rate {
        Some(hz) if hz == expected.hz() => Ok(()),
        Some(hz) => Err(format!("recording is {hz} Hz; game is {} Hz. Re-record at {} Hz before replay.", expected.hz(), expected.hz())),
        None => Err(format!("recording has no tick_rate (legacy 25 Hz); game is {} Hz. Re-record with explicit tick_rate before replay.", expected.hz())),
    }
}

/// Read a unique `# tick_rate HZ` metadata line from a tabular input.
pub fn text_rate(text: &str) -> Result<Option<u32>, String> {
    let mut rate = None;
    for line in text.lines().filter_map(|l| l.trim().strip_prefix('#')) {
        let mut w = line.split_whitespace();
        if w.next() == Some("tick_rate") {
            if rate.is_some() {
                return Err("recording: duplicate tick_rate".into());
            }
            rate = Some(
                w.next()
                    .ok_or("recording: missing tick_rate value")?
                    .parse()
                    .map_err(|_| "recording: invalid tick_rate")?,
            );
            if w.next().is_some() {
                return Err("recording: extra tick_rate fields".into());
            }
        }
    }
    Ok(rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_and_mismatched_rates_are_refused() {
        assert!(validate(None, TickRate::GAME)
            .unwrap_err()
            .contains("legacy 25 Hz"));
        assert!(validate(Some(25), TickRate::GAME)
            .unwrap_err()
            .contains("Re-record"));
        assert!(validate(Some(60), TickRate::GAME).is_ok());
        assert!(validate(Some(60), TickRate::BASE).is_err());
        assert_eq!(text_rate("# tick_rate 60\ntick\tnow"), Ok(Some(60)));
        assert!(text_rate("# tick_rate 60\n# tick_rate 25").is_err());
        assert!(text_rate("# tick_rate nope").is_err());
    }
    #[test]
    fn replay_boundaries_refuse_legacy_and_25_hz_inputs() {
        use crate::app::{soak::log::SoakLog, state_dump};
        use crate::world_view::present::FrameSchedule;
        for line in ["", "# tick_rate 25\n"] {
            let schedule =
                format!("# frame-schedule 1\n{line}# cursor_last 0\n# cursor_idle 0\n1\t16\n");
            assert!(FrameSchedule::parse(&schedule)
                .unwrap_err()
                .contains("Re-record"));
            let clock = format!("# send-clock 1\n{line}1\t16\n");
            assert!(state_dump::parse_send_clock(&clock, TickRate::GAME)
                .unwrap_err()
                .to_string()
                .contains("Re-record"));
            let seed = format!("# player-seed 1\n{line}1\t1\t2\n");
            assert!(state_dump::parse_player_seed(&seed, TickRate::GAME)
                .unwrap_err()
                .to_string()
                .contains("Re-record"));
        }
        for hz in [None, Some(25)] {
            let mut header = serde_json::json!({"k":"header","format":"state-1"});
            if let Some(hz) = hz {
                header["tick_rate"] = hz.into();
            }
            assert!(
                state_dump::parse_gfx_ready(&header.to_string(), TickRate::GAME)
                    .unwrap_err()
                    .to_string()
                    .contains("Re-record")
            );
        }
        for rate in ["", "tick_rate=25 "] {
            assert!(
                SoakLog::parse(&format!("soak-log 1 {rate}new=amazon\n1 wait 1\n"))
                    .unwrap_err()
                    .contains("Re-record")
            );
        }
    }
}
