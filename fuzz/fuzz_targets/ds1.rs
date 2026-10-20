#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    d2_fuzz::arm("ds1", data);
    let _ = d2_formats::ds1::Ds1::parse(data);
});
