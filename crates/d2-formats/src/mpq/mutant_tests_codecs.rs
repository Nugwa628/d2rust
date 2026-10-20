//! Exact-value tests for the MPQ codecs (ADPCM, the cipher, PKWARE DCL,
//! Huffman, the bit reader): hand-worked vectors and a reference model pin
//! each step of the decoders tightly enough that changing an operator or a
//! constant makes a test fail. Test-only; the codecs under test decode the
//! player's game archives.

use super::adpcm;
use super::bits::BitWriter;
use super::crypto::{decrypt, HASH_TABLE_KEY};
use super::explode::explode;
use super::tables::{
    ADPCM_CHANGE_TABLE, ADPCM_STEP_SIZE, CH_BITS, CH_CODE, DIST_BITS, DIST_CODE, EX_LEN_BITS,
    LEN_BASE, LEN_BITS, LEN_CODE,
};

// ---------------------------------------------------------------------
// IMA ADPCM

fn samples(bytes: &[u8]) -> Vec<i16> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| i16::from_le_bytes(*c))
        .collect()
}

/// Initial samples, read literally: one initial sample per channel, stopping
/// when the input or the output runs out.
#[test]
fn adpcm_initial_samples_stop_only_when_input_or_output_runs_out() {
    // Exactly enough input for the initial samples.
    assert_eq!(samples(&adpcm::decompress(&[0, 0, 5, 0], 1, 64)), [5]);
    assert_eq!(
        samples(&adpcm::decompress(&[0, 0, 5, 0, 7, 0], 2, 64)),
        [5, 7]
    );
    // One byte short of the last channel's sample.
    assert_eq!(samples(&adpcm::decompress(&[0, 0, 5, 0, 7], 2, 64)), [5]);
    // Output room for exactly one sample, or none.
    assert_eq!(samples(&adpcm::decompress(&[0, 0, 5, 0, 0x80], 1, 2)), [5]);
    assert!(adpcm::decompress(&[0, 0, 5, 0, 0x80], 1, 1).is_empty());
    assert!(adpcm::decompress(&[0, 0, 5, 0, 0x80], 1, 0).is_empty());
    assert_eq!(
        samples(&adpcm::decompress(&[0, 0, 5, 0, 7, 0], 2, 3)),
        [5],
        "no room for the second channel's sample"
    );
}

/// A plain reference model of the IMA ADPCM decoder.
fn adpcm_model(input: &[u8], c: usize, max_out: usize) -> Vec<i16> {
    let mut out = Vec::new();
    if input.len() < 2 {
        return out;
    }
    let shift = u32::from(input[1]);
    let mut sample = vec![0i32; c];
    let mut index = vec![44i32; c];
    let mut pos = 2;
    for ch in 0..c {
        if pos + 2 > input.len() || 2 * (out.len() + 1) > max_out {
            return out;
        }
        sample[ch] = i32::from(i16::from_le_bytes([input[pos], input[pos + 1]]));
        out.push(sample[ch] as i16);
        index[ch] = 44;
        pos += 2;
    }
    let mut ch = 0;
    for &op in &input[pos..] {
        if 2 * (out.len() + 1) > max_out {
            break;
        }
        if op & 0x80 != 0 {
            match op & 0x7F {
                0 => {
                    if index[ch] > 0 {
                        index[ch] -= 1;
                    }
                    out.push(sample[ch] as i16);
                    ch = (ch + 1) % c;
                }
                1 => index[ch] = (index[ch] + 8).min(88),
                2 => ch = (ch + 1) % c,
                _ => index[ch] = (index[ch] - 8).max(0),
            }
        } else {
            let base = ADPCM_STEP_SIZE[index[ch] as usize];
            let mut diff = base >> shift;
            for k in 0..=5 {
                if op & (1 << k) != 0 {
                    diff += base >> k;
                }
            }
            sample[ch] = if op & 0x40 != 0 {
                (sample[ch] - diff).max(-32768)
            } else {
                (sample[ch] + diff).min(32767)
            };
            out.push(sample[ch] as i16);
            index[ch] = (index[ch] + ADPCM_CHANGE_TABLE[usize::from(op & 0x1F)]).clamp(0, 88);
            ch = (ch + 1) % c;
        }
    }
    out
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 32) as u32
    }
}

/// Every ADPCM command (repeat, index up/down, channel switch,
/// encoded samples with and without the sign bit), mono and stereo,
/// against the reference model above.
#[test]
fn adpcm_matches_reference_model() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for case in 0..400 {
        let c = 1 + case % 2;
        let len = (rng.next() % 80) as usize;
        let mut input = Vec::with_capacity(len);
        for i in 0..len {
            let b = match i {
                0 => rng.next() as u8,
                1 => (rng.next() % 8) as u8,
                2..=5 => rng.next() as u8,
                _ => match rng.next() % 8 {
                    0 => 0x80,
                    1 => 0x81,
                    2 => 0x82,
                    3 => 0x83 + (rng.next() % 0x7D) as u8,
                    _ => (rng.next() & 0x7F) as u8,
                },
            };
            input.push(b);
        }
        let max_out = if case % 5 == 0 {
            (rng.next() % 40) as usize
        } else {
            1024
        };
        let got = adpcm::decompress(&input, c, max_out);
        assert!(got.len() <= max_out, "case {case}: output exceeds max_out");
        assert_eq!(
            samples(&got),
            adpcm_model(&input, c, max_out),
            "case {case}: input {input:02X?} channels {c} max_out {max_out}"
        );
    }
}

/// Hand-worked ADPCM vectors for single commands (mono, shift 0, start 0).
#[test]
fn adpcm_single_commands() {
    let run = |ops: &[u8]| {
        let mut input = vec![0, 0, 0, 0];
        input.extend_from_slice(ops);
        samples(&adpcm::decompress(&input, 1, 64))
    };
    // Encoded 0x00 at index 44: diff = 0x1EE >> 0 = 0x1EE; ChangeTable[0] = -1.
    // Encoded 0x00 then at index 43: + StepSize[43] = 0x1C1.
    assert_eq!(run(&[0x00, 0x00]), [0, 0x1EE, 0x1EE + 0x1C1]);
    // 0x80 decrements the index: next step uses StepSize[43].
    assert_eq!(run(&[0x80, 0x00]), [0, 0, 0x1C1]);
    // 0x81: index 44 + 8 = 52 -> StepSize[52] = 0x424.
    assert_eq!(run(&[0x81, 0x00]), [0, 0x424]);
    // 0x81 x6: capped at 88 -> StepSize[88] = 0x7FFF, clamps to 32767.
    assert_eq!(
        run(&[0x81; 6].iter().chain(&[0x00]).copied().collect::<Vec<_>>()),
        [0, 32767]
    );
    // 0x83: index 44 - 8 = 36 -> StepSize[36] = 0xE6.
    assert_eq!(run(&[0x83, 0x00]), [0, 0xE6]);
    // 0x82 in mono: ch = (0 + 1) mod 1 = 0, nothing else changes.
    assert_eq!(run(&[0x82, 0x00]), [0, 0x1EE]);
    // Bits 0..5 of op add base >> k: op 0x3F at index 44.
    let b = 0x1EE;
    let want = b + b + (b >> 1) + (b >> 2) + (b >> 3) + (b >> 4) + (b >> 5);
    assert_eq!(run(&[0x3F]), [0, want as i16]);
    // Sign bit: 0x40 subtracts.
    assert_eq!(run(&[0x40]), [0, -0x1EE]);
    // ChangeTable[3] = +4: index 48 -> StepSize[48] = 0x2D4 (with base >> 0
    // twice from shift 0 and bit 0, bit 1).
    let b = 0x1EE;
    let first = b + b + (b >> 1);
    assert_eq!(
        run(&[0x03, 0x00]),
        [0, first as i16, (first + 0x2D4) as i16]
    );
}

/// Stereo: commands act on the current channel, and 0x82 / samples /
/// repeats advance it.
#[test]
fn adpcm_stereo_channel_rotation() {
    let run = |ops: &[u8]| {
        let mut input = vec![0, 0, 10, 0, 20, 0];
        input.extend_from_slice(ops);
        samples(&adpcm::decompress(&input, 2, 64))
    };
    // 0x82 switches to channel 1; the repeat then outputs channel 1's sample.
    assert_eq!(run(&[0x82, 0x80]), [10, 20, 20]);
    // Encoded sample on ch 0, then a repeat on ch 1.
    assert_eq!(run(&[0x00, 0x80]), [10, 20, 10 + 0x1EE, 20]);
    // 0x81 raises only ch 0's index; ch 1 still uses StepSize[44].
    assert_eq!(run(&[0x81, 0x00, 0x00]), [10, 20, 10 + 0x424, 20 + 0x1EE]);
}

// ---------------------------------------------------------------------
// Decryption: dwords after the first depend on the key schedule.

/// Plaintexts of four zero dwords, worked by hand from the key schedule;
/// trailing bytes stay as they are.
#[test]
fn decrypt_key_schedule_vectors() {
    for (key, want) in [
        (
            HASH_TABLE_KEY,
            [0x863C_CFCC_u32, 0xEE09_F6A4, 0x9C71_4C55, 0x6B1B_85F5],
        ),
        (
            0xDEAD_BEEF,
            [0x5DD0_16B8, 0x58DB_50D0, 0xD2AD_3720, 0x7299_EBD3],
        ),
        (0, [0x0829_9586, 0x31C9_1E96, 0x0435_8EC1, 0xE3B2_DEB8]),
    ] {
        // Two trailing bytes (len mod 4) are left unchanged.
        let mut data = [0u8; 18];
        data[16..].copy_from_slice(&[0xAB, 0xCD]);
        decrypt(&mut data, key);
        assert_eq!(data[16..], [0xAB, 0xCD], "key {key:#010X}");
        let got: Vec<u32> = data[..16]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| u32::from_le_bytes(*c))
            .collect();
        assert_eq!(got, want, "key {key:#010X}");
    }
}

// ---------------------------------------------------------------------
// PKWARE DCL

/// Builds a DCL stream: header, then the bits `body` writes, then the
/// end marker (length symbol 15 with all 8 extra bits set).
fn dcl(mode: u8, dict: u8, body: impl FnOnce(&mut BitWriter)) -> Vec<u8> {
    let mut w = BitWriter::default();
    body(&mut w);
    w.write(1, 1);
    w.write(u32::from(LEN_CODE[15]), u32::from(LEN_BITS[15]));
    w.write(0xFF, u32::from(EX_LEN_BITS[15]));
    let mut out = vec![mode, dict];
    out.extend_from_slice(&w.bytes);
    out
}

fn literal(w: &mut BitWriter, b: u8) {
    w.write(0, 1);
    w.write(u32::from(b), 8);
}

/// A copy of `n` (3..=9) bytes from `dist + 1` back, with `d` dictionary bits.
fn copy(w: &mut BitWriter, n: u32, dist: u32, d: u32) {
    let l = (n - 2) as usize;
    assert!(l < 8 && EX_LEN_BITS[l] == 0 && u32::from(LEN_BASE[l]) == n - 2);
    w.write(1, 1);
    w.write(u32::from(LEN_CODE[l]), u32::from(LEN_BITS[l]));
    let dsym = (dist >> d) as usize;
    w.write(u32::from(DIST_CODE[dsym]), u32::from(DIST_BITS[dsym]));
    w.write(dist & ((1 << d) - 1), d);
}

/// Literal mode 1 (ASCII): literals are `ChCode/ChBits` symbols.
#[test]
fn explode_ascii_literals() {
    let text = b"Hello, DCL!\x00\xFF\n";
    let stream = dcl(1, 4, |w| {
        for &b in text {
            w.write(0, 1);
            w.write(
                u32::from(CH_CODE[usize::from(b)]),
                u32::from(CH_BITS[usize::from(b)]),
            );
        }
    });
    assert_eq!(explode(&stream, 64).unwrap(), text);
}

/// Literals past the requested size are discarded.
#[test]
fn explode_literals_are_capped() {
    let stream = dcl(0, 4, |w| {
        for &b in b"ABCDE" {
            literal(w, b);
        }
    });
    assert_eq!(explode(&stream, 64).unwrap(), b"ABCDE");
    for cap in 0..5 {
        assert_eq!(
            explode(&stream, cap).unwrap(),
            &b"ABCDE"[..cap],
            "cap {cap}"
        );
    }
}

/// Distances beyond one symbol's worth: distance = (d << D) | r.
#[test]
fn explode_long_distance_copies() {
    let lits: Vec<u8> = (0..200u8).collect();
    for d in 4..=6u32 {
        for dist in [17u32, 63, 100, 150] {
            if dist >= 64 << d || dist as usize >= lits.len() {
                continue;
            }
            let stream = dcl(0, d as u8, |w| {
                for &b in &lits {
                    literal(w, b);
                }
                copy(w, 5, dist, d);
            });
            let mut want = lits.clone();
            let start = lits.len() - dist as usize - 1;
            want.extend_from_slice(&lits[start..start + 5]);
            assert_eq!(explode(&stream, 1024).unwrap(), want, "D {d} dist {dist}");
        }
    }
}
