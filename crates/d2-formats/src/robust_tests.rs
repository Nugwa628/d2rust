//! Robustness property tests (METHODS M07) for the file-format parsers
//! outside `mpq`, using the helpers in `robust`: arbitrary bytes into every `parse`,
//! and accessors that take caller data. Mutations of valid files live in each parser's
//! own test module, next to its builder.
//!
//! Default case counts are small so `cargo test` stays fast; set
//! `PROPTEST_CASES` to hunt harder (it overrides every default here).

use proptest::prelude::*;
use proptest::test_runner::Config;

use crate::robust::{bounded, bytes};

/// Proptest config with `default` cases, or `PROPTEST_CASES` when set.
/// (`ProptestConfig::with_cases` would ignore the env var.)
pub(crate) fn config(default: u32) -> Config {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default);
    Config {
        cases,
        // Minimized failures become `regress_*` unit tests instead.
        failure_persistence: None,
        ..Config::default()
    }
}

/// Runs `parse` on `data` under [`bounded`]; the result may be Ok or Err.
pub(crate) fn check<T: 'static, E: 'static>(data: Vec<u8>, parse: fn(&[u8]) -> Result<T, E>) {
    bounded(move || {
        let _ = parse(&data);
    });
}

proptest! {
    #![proptest_config(config(64))]

    #[test]
    fn palette_bytes(data in bytes(1024)) {
        check(data, crate::palette::Palette::parse);
    }

    #[test]
    fn pl2_bytes(data in bytes(1024)) {
        check(data, crate::palette::Pl2::parse);
    }

    #[test]
    fn dc6_bytes(data in bytes(512)) {
        check(data, crate::dc6::Dc6::parse);
    }

    #[test]
    fn dcc_bytes(data in bytes(512)) {
        check(data, crate::dcc::Dcc::parse);
    }

    #[test]
    fn dt1_bytes(data in bytes(1024)) {
        check(data, crate::dt1::Dt1::parse);
    }

    #[test]
    fn ds1_bytes(data in bytes(512)) {
        check(data, crate::ds1::Ds1::parse);
    }

    #[test]
    fn cof_bytes(data in bytes(512)) {
        check(data, crate::cof::Cof::parse);
    }

    #[test]
    fn tbl_bytes(data in bytes(512)) {
        check(data, crate::tbl::StringTable::parse);
    }

    #[test]
    fn font_bytes(data in bytes(512)) {
        check(data, crate::font::FontTable::parse);
    }

    #[test]
    fn animdata_bytes(data in bytes(2048)) {
        check(data, crate::animdata::AnimData::parse);
    }
}
