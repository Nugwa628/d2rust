#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    d2_fuzz::arm("palette", data);
    let _ = d2_formats::palette::Palette::parse(data);
});
