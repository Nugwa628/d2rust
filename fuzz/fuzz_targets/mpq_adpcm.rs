#![no_main]
use libfuzzer_sys::fuzz_target;

// Input: u32 LE max_out, 1 byte (low bit: channels - 1), then the stream.
fuzz_target!(|data: &[u8]| {
    d2_fuzz::arm("mpq_adpcm", data);
    let Some((head, rest)) = data.split_first_chunk::<4>() else {
        return;
    };
    let Some((ch, rest)) = rest.split_first() else {
        return;
    };
    let channels = 1 + usize::from(ch & 1);
    let _ = d2_formats::mpq::fuzz_api::adpcm_decompress(
        rest,
        channels,
        u32::from_le_bytes(*head) as usize,
    );
});
