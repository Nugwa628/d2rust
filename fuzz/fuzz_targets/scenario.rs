#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    d2_fuzz::arm("scenario", data);
    if let Ok(text) = std::str::from_utf8(data) {
        if let Ok(scenario) = d2_client::scenario_script::Scenario::parse(text) {
            let canonical = scenario.to_text();
            let parsed = d2_client::scenario_script::Scenario::parse(&canonical)
                .expect("writer must produce valid scenario");
            assert_eq!(canonical, parsed.to_text());
        }
    }
});
