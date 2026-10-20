#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    d2_fuzz::arm("dc6", data);
    if let Ok(d) = d2_formats::dc6::Dc6::parse(data) {
        for dir in 0..4 {
            for f in 0..4 {
                let _ = d.frame(dir, f);
            }
        }
    }
});
