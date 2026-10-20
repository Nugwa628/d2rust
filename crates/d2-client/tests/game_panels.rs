//! Facts about the user's game panel files (sizes, frames and
//! constants). Needs the game files:
//! `D2_GAME_DIR=<install> cargo test -p d2-client --test game_panels -- --ignored`

use d2_client::ui::layout::{panel_layout, PanelKey, RowKind};
use d2_client::ui::panels::UiFiles;
use d2_formats::dc6::Dc6;
use d2_formats::mpq::ArchiveSet;

fn set() -> ArchiveSet {
    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
    ArchiveSet::open_dir(&dir).expect("archives in D2_GAME_DIR open")
}

fn dc6(set: &ArchiveSet, file: &str) -> Dc6 {
    let path = format!("data\\global\\ui\\{file}.dc6");
    let bytes = set.read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    Dc6::parse(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn panel_quads_have_the_stated_sizes_and_zero_offsets() {
    let set = set();
    let rows = panel_layout().unwrap();
    let files = UiFiles::new(&rows);
    // Every `art0` row starts a quad set: frames f … f + 3.
    for r in rows
        .iter()
        .filter(|r| r.kind == RowKind::Draw && r.item == "art0")
    {
        let d2_client::ui::layout::FrameSpec::Index(f) = r.frame else {
            panic!("line {}: art0 frame", r.line);
        };
        let names: Vec<String> = match r.panel {
            PanelKey::Ui(4) => (0..7)
                .filter_map(|c| files.row_file(r, Some(c)))
                .map(|i| files.name(i).unwrap().to_string())
                .collect(),
            _ => vec![r.file.clone().unwrap()],
        };
        for name in names {
            let d = dc6(&set, &name);
            let sizes: Vec<_> = (f..f + 4)
                .map(|i| {
                    let fr = &d.frames[i as usize];
                    (fr.width, fr.height, fr.offset_x, fr.offset_y)
                })
                .collect();
            assert_eq!(
                sizes,
                [
                    (256, 256, 0, 0),
                    (64, 256, 0, 0),
                    (256, 176, 0, 0),
                    (64, 176, 0, 0)
                ],
                "{name} frames {f}…"
            );
        }
    }
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn panel_files_frame_counts_and_sizes() {
    let set = set();
    // (file, frames, Some(frame size) when every frame has it)
    type Case = (&'static str, usize, Option<(u32, u32)>);
    let cases: &[Case] = &[
        ("panel\\invchar", 8, None),
        ("panel\\invchar6", 8, None),
        ("panel\\bank", 4, None),
        ("panel\\tradestash", 4, None),
        ("panel\\supertransmogrifier", 4, None),
        ("panel\\buysell", 4, None),
        ("panel\\buysellbtn", 23, None),
        ("panel\\buyselltabs", 8, Some((79, 31))),
        ("panel\\800borderframe", 10, None),
        ("panel\\ctrlpnl7", 6, None),
        ("panel\\800ctrlpnl7", 7, None),
        ("panel\\level", 3, Some((30, 30))),
        ("panel\\levelsocket", 1, Some((35, 36))),
        ("panel\\skillpoints", 1, Some((135, 23))),
        ("panel\\miniconvert", 2, Some((32, 32))),
        ("menu\\waygatebackground", 4, None),
        ("menu\\waygatetabs", 8, Some((78, 30))),
        ("menu\\expwaygatetabs", 10, Some((63, 31))),
        ("menu\\waygateicons", 5, Some((30, 30))),
        ("menu\\horadric", 31, None),
    ];
    for &(file, n, size) in cases {
        let d = dc6(&set, file);
        assert_eq!(d.frames.len(), n, "{file}");
        for (i, fr) in d.frames.iter().enumerate() {
            // Every file but `menu\horadric` has zero offsets; its
            // own vectors are checked below (its test vectors).
            if file != "menu\\horadric" {
                assert_eq!((fr.offset_x, fr.offset_y), (0, 0), "{file} frame {i}");
            }
            if let Some(s) = size {
                assert_eq!((fr.width, fr.height), s, "{file} frame {i}");
            }
        }
    }
    // The cube's test vectors: frames 0 and 30 are 2 × 2 at (0, 0), frame 1
    // is 92 × 121 at (−205, 17), frame 15 sits at (−280, 82).
    let h = dc6(&set, "menu\\horadric");
    for i in [0, 30] {
        let fr = &h.frames[i];
        assert_eq!(
            (fr.width, fr.height, fr.offset_x, fr.offset_y),
            (2, 2, 0, 0),
            "menu\\horadric frame {i}"
        );
    }
    let f1 = &h.frames[1];
    assert_eq!(
        (f1.width, f1.height, f1.offset_x, f1.offset_y),
        (92, 121, -205, 17)
    );
    let f15 = &h.frames[15];
    assert_eq!((f15.offset_x, f15.offset_y), (-280, 82));
    for c in ['a', 's', 'n', 'p', 'b', 'd', 'i'] {
        let file = format!("spells\\skltree_{c}_back");
        assert_eq!(dc6(&set, &file).frames.len(), 16, "{file}");
    }
}
