#![no_main]
use d2_formats::d2s::{self, SaveTables, StatSave};
use libfuzzer_sys::fuzz_target;
struct Synthetic;
impl SaveTables for Synthetic {
    fn stat_save(&self, id: u16) -> Option<StatSave> {
        (id < 16).then_some(StatSave {
            bits: 16,
            param: 0,
            signed: false,
        })
    }
    fn item_entry_len(&self, _: &[u8]) -> Result<usize, String> {
        Err("synthetic tables have no item records".into())
    }
}
fuzz_target!(|data: &[u8]| {
    d2_fuzz::arm("save_loader", data);
    for expansion in [false, true] {
        let options = d2s::ReadOptions {
            expansion,
            game: None,
        };
        let _ = d2s::read(data, &options, &Synthetic);
        // A valid checksum allows mutations to reach body parsing too.
        if data.len() >= d2s::HEADER_SIZE {
            let mut checked = data.to_vec();
            d2s::finish(&mut checked);
            let _ = d2s::read(&checked, &options, &Synthetic);
        }
    }
});
