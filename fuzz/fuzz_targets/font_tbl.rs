#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    d2_fuzz::arm("font_tbl", data);
    let _ = d2_formats::font::FontTable::is_font_table(data);
    let _ = d2_formats::font::FontTable::parse(data);
});
