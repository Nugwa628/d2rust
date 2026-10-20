#![no_main]
use libfuzzer_sys::fuzz_target;

// Input: u32 LE block flags, u32 LE expected size, then the sector bytes
// (mask byte + payload, or a bare PKWARE stream under IMPLODE).
fuzz_target!(|data: &[u8]| {
    d2_fuzz::arm("mpq_sector", data);
    let Some((flags, rest)) = data.split_first_chunk::<4>() else {
        return;
    };
    let Some((exp, rest)) = rest.split_first_chunk::<4>() else {
        return;
    };
    let _ = d2_formats::mpq::fuzz_api::sector(
        u32::from_le_bytes(*flags),
        rest,
        u32::from_le_bytes(*exp) as usize,
    );
});
