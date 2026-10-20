#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    d2_fuzz::arm("dcc", data);
    let _ = d2_formats::dcc::Dcc::parse(data);
});
