//! Synthetic save seeds written by the production writer, never from game files.
use d2_formats::d2s::{self, SaveTables, StatSave};
struct Tables;
impl SaveTables for Tables {
    fn stat_save(&self, id: u16) -> Option<StatSave> {
        (id < 16).then_some(StatSave {
            bits: 16,
            param: 0,
            signed: false,
        })
    }
    fn item_entry_len(&self, _: &[u8]) -> Result<usize, String> {
        Err("no synthetic items".into())
    }
}
fn main() {
    let root = std::path::PathBuf::from("seeds/save_loader");
    std::fs::create_dir_all(&root).unwrap();
    for (name, body, expansion) in [
        ("new", false, false),
        ("classic", true, false),
        ("expansion", true, true),
    ] {
        let mut save = d2s::D2s::new_stub(
            b"Synthetic",
            0,
            if expansion { d2s::status::EXPANSION } else { 0 },
            0,
        )
        .unwrap();
        if body {
            save.header.status &= !d2s::status::NEW;
            save.body = Some(d2s::Body {
                skills: vec![0; save.header.skill_count as usize],
                ..Default::default()
            });
        }
        let bytes = d2s::write(&save, &Tables).unwrap();
        d2s::read(
            &bytes,
            &d2s::ReadOptions {
                expansion,
                game: None,
            },
            &Tables,
        )
        .expect("seed must load");
        std::fs::write(root.join(name), bytes).unwrap();
    }
}
