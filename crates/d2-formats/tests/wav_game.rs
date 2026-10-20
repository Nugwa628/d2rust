//! `.wav` parsing against the user's own game install: known sound files must parse to
//! fixed channel, frame, sum and CRC values, and every file `sounds.txt` names must be
//! playable. Reads archives through `d2_formats::mpq`. Ignored by default; run with
//! `cargo test -p d2-formats --test wav_game -- --ignored`.

use std::path::PathBuf;

use d2_formats::mpq::ArchiveSet;
use d2_formats::wav::Wav;

fn set() -> ArchiveSet {
    let dir = PathBuf::from(std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set"));
    ArchiveSet::open_dir(dir).unwrap()
}

/// (path, archive, channels, frames, sum, first 8, CRC of the `data` bytes).
type Row = (&'static str, &'static str, u16, usize, i64, [i16; 8], u32);

/// CRC-32 (IEEE, as zlib), bitwise.
fn crc32(bytes: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in bytes {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 {
                (c >> 1) ^ 0xEDB8_8320
            } else {
                c >> 1
            };
        }
    }
    !c
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn known_sound_files() {
    let rows: &[Row] = &[
        (
            r"data\global\sfx\cursor\pass.wav",
            "d2sfx.mpq",
            1,
            1_124,
            95_592,
            [0, -3, 1, 16, 30, 44, 26, -41],
            0x236e8962,
        ),
        (
            r"data\global\sfx\item\gem.wav",
            "patch_d2.mpq",
            1,
            15_627,
            -4_923,
            [-4, -2, -21, -43, -49, -39, -37, -46],
            0x03050e69,
        ),
        (
            r"data\global\sfx\cursor\leveluphireling.wav",
            "d2exp.mpq",
            1,
            15_065,
            -6_073_869,
            [0, 0, 1, 1, 2, 3, 5, 6],
            0x06de9042,
        ),
        (
            r"data\global\sfx\cursor\intro\amazon select.wav",
            "d2sfx.mpq",
            2,
            54_397,
            245_877,
            [0; 8],
            0xffff477e,
        ),
        (
            r"data\local\sfx\act1\amazon\ama_act1_find_tristram.wav",
            "d2speech.mpq",
            1,
            65_184,
            41_165,
            [-6, -6, -6, -6, -6, -6, -15, -7],
            0xe3cfd2ff,
        ),
        (
            r"data\local\sfx\act1\druid\dru_act1_find_tristram.wav",
            "d2xtalk.mpq",
            1,
            106_857,
            -3_897,
            [-55, -70, -74, -106, -90, -65, -73, -60],
            0x6f43da9e,
        ),
        (
            r"data\global\music\act1\crypt.wav",
            "d2music.mpq",
            2,
            5_971_968,
            -993_081_113,
            [79, 209, -73, -11, -185, -189, -300, -384],
            0x106ab6a9,
        ),
        (
            r"data\global\music\act5\baal.wav",
            "d2xmusic.mpq",
            2,
            5_792_521,
            -14_599_737,
            [2, 0, 3, 0, 1, 0, 2, 0],
            0x4c723350,
        ),
    ];
    let set = set();
    let mut bad = Vec::new();
    for &(path, archive, ch, frames, sum, first, crc) in rows {
        let (source, bytes) = set.read_with_source(path).unwrap().expect(path);
        let w = Wav::parse(&bytes).unwrap();
        let data: Vec<u8> = w.samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let got = (
            source.as_str(),
            w.channels,
            w.frames(),
            w.samples.iter().map(|&s| i64::from(s)).sum::<i64>(),
            <[i16; 8]>::try_from(&w.samples[..8]).unwrap(),
            crc32(&data),
            w.is_playable(),
        );
        let want = (archive, ch, frames, sum, first, crc, true);
        if got != want {
            bad.push(format!("{path}: got {got:?}, want {want:?}"));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// Every `sounds.txt` row whose file exists parses as a playable sound
/// (4,508 resolve in the checked install: 4,434 mono, 74 stereo). A sound's id is its
/// data-line index in `sounds.txt`.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn every_sounds_txt_file() {
    let set = set();
    let txt = set.read(r"data\global\excel\sounds.txt").unwrap();
    let txt = String::from_utf8_lossy(&txt);
    let mut lines = txt.split("\r\n").filter(|l| !l.is_empty());
    let header: Vec<&str> = lines.next().unwrap().split('\t').collect();
    let col = header.iter().position(|h| *h == "FileName").unwrap();
    let (mut resolved, mut mono, mut stereo) = (0, 0, 0);
    let mut bad = Vec::new();
    for (id, line) in lines.enumerate() {
        let name = line.split('\t').nth(col).unwrap_or("");
        let local = (2934..=4656).contains(&id);
        let music = (4657..=4698).contains(&id);
        let path = format!(
            "{}{}{name}",
            if local { r"DATA\LOCAL" } else { r"DATA\GLOBAL" },
            if music { r"\MUSIC\" } else { r"\SFX\" }
        );
        let Some((_, bytes)) = set.read_with_source(&path).unwrap() else {
            continue;
        };
        resolved += 1;
        match Wav::parse(&bytes) {
            Ok(w) if w.is_playable() => {
                if w.channels == 1 {
                    mono += 1
                } else {
                    stereo += 1
                }
            }
            other => bad.push(format!(
                "{id} {path}: {:?}",
                other.map(|w| (w.format_tag, w.channels, w.rate, w.bits))
            )),
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
    assert_eq!((resolved, mono, stereo), (4_508, 4_434, 74));
}
