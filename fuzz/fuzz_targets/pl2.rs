#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    d2_fuzz::arm("pl2", data);
    let _ = d2_formats::palette::Pl2::parse(data);
});
