#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    d2_fuzz::arm("txt", data);
    if let Ok(t) = d2_data::txt::TxtTable::parse("t.txt", data) {
        assert!(t.records.iter().all(|r| r.cells.len() == t.columns()));
        let _ = d2_data::txt::bind(&t.header, &["name", "code", "lvl"]);
    }
});
