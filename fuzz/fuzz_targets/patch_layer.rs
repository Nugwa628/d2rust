#![no_main]
use libfuzzer_sys::fuzz_target;

// Parses the input as a layer and as a stack (and a stack whose every layer
// is the input), then applies the layer to a small synthetic table.
use d2_data::patch::{
    apply_stack, load_stack, parse_layer, parse_stack, KeyKind, Origin, PatchData, PatchTable, Row,
    TableRules, Writer,
};

fn v(s: &str) -> Vec<u8> {
    s.as_bytes().to_vec()
}

fn fixture() -> PatchData {
    let rules = TableRules {
        name: "items".into(),
        txt_name: "items.txt".into(),
        key_field: Some(v("code")),
        kind: KeyKind::Code,
        unique: true,
        fixed: false,
        scope: "items.code".into(),
        lists: vec![["name", "code", "lvl", "dam"]
            .iter()
            .map(|f| v(f))
            .collect()],
    };
    let header: Vec<Vec<u8>> = ["name", "code", "lvl", "dam", "dam", "*note"]
        .iter()
        .map(|h| v(h))
        .collect();
    let rows = ["Axe;axe;1;3;0;", "Club;clb;1;2;0;old", "Axe;ax2;5;7;0;"]
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let cells: Vec<Vec<u8>> = line.split(';').map(v).collect();
            Row {
                origin: Origin::Base(i + 2),
                writers: vec![Writer::Base; cells.len()],
                cells,
            }
        })
        .collect();
    PatchData::from_tables(vec![
        PatchTable::new(&rules, "fixture", header, rows).unwrap()
    ])
}

fuzz_target!(|data: &[u8]| {
    d2_fuzz::arm("patch_layer", data);
    let (layer, _) = parse_layer("a.d2patch", data, 1);
    let _ = parse_stack("s.d2stack", data);
    let _ = load_stack("s.d2stack", data, &mut |_| Some(data.to_vec()));
    let mut d = fixture();
    let _ = apply_stack(&mut d, &[layer.clone(), layer], "s.d2stack");
});
