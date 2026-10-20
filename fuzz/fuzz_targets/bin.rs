#![no_main]
use libfuzzer_sys::fuzz_target;

// Input: 1 byte selecting a runtime table of the schema, then the .bin bytes.
use d2_data::bin::BinTable;
use d2_data::schema::schema;
use d2_data::strings::StringTables;

fuzz_target!(|data: &[u8]| {
    d2_fuzz::arm("bin", data);
    let Some((sel, rest)) = data.split_first() else {
        return;
    };
    let defs: Vec<_> = schema().runtime().collect();
    let def = defs[usize::from(*sel) % defs.len()];
    if let Ok(t) = BinTable::parse(&def.name, "p", &def.bin_name, rest, def.record_size) {
        assert_eq!(t.records.len(), t.count * t.record_size);
        let _ = d2_data::bin::post_load_check(&t, &[], &StringTables::default(), true);
        let _ = d2_data::bin::post_load_check(&t, &[], &StringTables::default(), false);
    }
});
