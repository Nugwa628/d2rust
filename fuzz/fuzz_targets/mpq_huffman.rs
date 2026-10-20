#![no_main]
use libfuzzer_sys::fuzz_target;

// Input: u32 LE max_out, then the Huffman stream.
fuzz_target!(|data: &[u8]| {
    d2_fuzz::arm("mpq_huffman", data);
    let Some((head, rest)) = data.split_first_chunk::<4>() else {
        return;
    };
    let _ = d2_formats::mpq::fuzz_api::huffman_decompress(rest, u32::from_le_bytes(*head) as usize);
});
