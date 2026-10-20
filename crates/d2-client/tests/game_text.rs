//! Text test vectors on the user's game font files (the
//! `text-fonts.tsv` vectors). Needs the game files:
//! `D2_GAME_DIR=<install> cargo test -p d2-client --test game_text -- --ignored`

use d2_client::ui::text::{line_width, max_width, width_a, width_c, wrap, OriginalText};
use d2_client::ui::{GlyphLookup, Point, TextOpts, TextRules, TextStyle, FONTS};
use d2_formats::dc6::Dc6;
use d2_formats::font::FontTable;
use d2_formats::mpq::ArchiveSet;

fn set() -> ArchiveSet {
    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
    ArchiveSet::open_dir(&dir).expect("archives in D2_GAME_DIR open")
}

fn font(set: &ArchiveSet, id: usize) -> FontTable {
    let name = FONTS[id].tbl_path;
    FontTable::parse(&set.read(name).unwrap_or_else(|e| panic!("{name}: {e}"))).unwrap()
}

fn u(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn font_files_match_text_fonts_tsv() {
    let set = set();
    for (id, info) in FONTS.iter().enumerate() {
        let f = font(&set, id);
        assert_eq!(
            (f.height, f.count, f.glyphs.len()),
            (info.height, 256, 256),
            "{}",
            info.name
        );
        for (i, g) in f.glyphs.iter().enumerate() {
            assert_eq!(
                (usize::from(g.code), usize::from(g.frame)),
                (i, i),
                "{} record {i}",
                info.name
            );
        }
        let dc6 = Dc6::parse(&set.read(info.dc6_path).unwrap()).unwrap();
        for fr in &dc6.frames {
            assert_eq!(
                (fr.width, fr.height, fr.offset_x, fr.offset_y, fr.flip),
                (info.frame_w, info.frame_h, 0, 0, 0),
                "{}",
                info.name
            );
        }
    }
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn metric_and_wrap_vectors() {
    let set = set();
    let f16 = font(&set, 1);
    let g = GlyphLookup::new(&f16);
    assert_eq!(width_a(&g, &u("Stash")), Ok(40));
    assert_eq!(max_width(&g, &u("AB\nC")), Ok(19));
    let at: Vec<_> = OriginalText
        .place(
            &g,
            &u("AB\nC"),
            Point::new(100, 200),
            TextStyle::default(),
            &TextOpts::centered(),
        )
        .unwrap()
        .into_iter()
        .map(|p| (p.at.x, p.at.y))
        .collect();
    assert_eq!(at, [(104, 200), (116, 200), (109, 184)]);

    let f8 = font(&set, 0);
    let g = GlyphLookup::new(&f8);
    assert_eq!(width_a(&g, &u("ÿc4Gold")), Ok(49));
    assert_eq!(width_c(&g, &u("ÿc4Gold"), 0, 7), Ok(28));
    assert_eq!(width_c(&g, &u("ÿc9Rare"), 0, 7), Ok(50));
    assert_eq!(line_width(&g, &u("ÿc9Rare"), 0), Ok(29));
    assert_eq!(line_width(&g, &u("ÿmX"), 0), Ok(12));

    let f10 = font(&set, 4);
    let g = GlyphLookup::new(&f10);
    let lines = |t: &str, m| -> Vec<String> {
        let t = u(t);
        wrap(&g, &t, m)
            .unwrap()
            .into_iter()
            .map(|l| String::from_utf16(l).unwrap())
            .collect()
    };
    let fox = "The quick brown fox jumps over the lazy dog";
    assert_eq!(
        lines(fox, 100),
        ["The quick brown ", "fox jumps over ", "the lazy dog"]
    );
    assert_eq!(
        lines(fox, 60),
        [
            "The quick ",
            "brown fox ",
            "jumps ",
            "over the ",
            "lazy dog"
        ]
    );
    assert_eq!(
        lines("Supercalifragilistic", 50),
        ["Supercal", "ifragilisti", "c"]
    );
    assert_eq!(width_a(&g, &u("The quick brown ")), Ok(94));
}
