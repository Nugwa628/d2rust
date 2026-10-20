#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    d2_fuzz::arm("cof", data);
    if let Ok(c) = d2_formats::cof::Cof::parse(data) {
        for d in 0..3 {
            for f in 0..3 {
                for s in 0..16 {
                    let _ = c.component_at(d, f, s);
                }
            }
        }
    }
});
