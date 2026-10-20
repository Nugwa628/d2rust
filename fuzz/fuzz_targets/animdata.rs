#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    d2_fuzz::arm("animdata", data);
    if let Ok(a) = d2_formats::animdata::AnimData::parse(data) {
        let _ = a.find(b"A1HTH");
        let _ = a.info(b"ZZZ");
        if data.len() > 4 {
            let _ = a.find(&data[..4]);
            let _ = a.record(&data[..4]);
            let _ = a.info(&data[..4]);
        }
    }
});
