#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    d2_fuzz::arm("tbl", data);
    if let Ok(t) = d2_formats::tbl::StringTable::parse(data) {
        for i in 0..8 {
            let _ = t.element(i);
        }
        let _ = t.get(b"axe");
        let _ = t.find_slot(b"");
        if data.len() > 8 {
            let _ = t.get(&data[..8]);
        }
    }
});
