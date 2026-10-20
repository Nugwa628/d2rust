use d2_data::{schema::schema, txt::TxtTable};
use std::collections::{BTreeMap, BTreeSet};
type Tables = BTreeMap<String, TxtTable>;
fn cell<'a>(t: &'a TxtTable, row: usize, col: &[u8]) -> &'a [u8] {
    t.header
        .iter()
        .position(|h| h.eq_ignore_ascii_case(col))
        .and_then(|c| t.records[row].cells.get(c))
        .map_or(&[], Vec::as_slice)
}
fn key(b: &[u8]) -> Vec<u8> {
    b.iter().map(u8::to_ascii_lowercase).collect()
}
fn link_key(value: &[u8], kind: Option<d2_data::schema::LinkerKind>) -> Vec<u8> {
    match kind {
        Some(d2_data::schema::LinkerKind::Code) => d2_data::compile::code4(value).to_vec(),
        _ => d2_data::compile::name_key(value).unwrap_or_else(|| value.to_vec()),
    }
}
/// A cell that names something. Blank (or whitespace-only) cells and
/// comment-marked cells (`*...`, e.g. `*enr` or the `*end*` terminator) are
/// not references or keys.
fn named(b: &[u8]) -> bool {
    let b = b.trim_ascii();
    !b.is_empty() && b != b"-1" && !b.starts_with(b"*")
}
fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}
// Same fixed row ranges as d2-client/audio/sound_table/table.rs.
fn sound_path(row: usize, file: &str) -> String {
    let root = if (2934..=4656).contains(&row) {
        "local"
    } else {
        "global"
    };
    let kind = if (4657..=4698).contains(&row) {
        "music"
    } else {
        "sfx"
    };
    format!("data\\{root}\\{kind}\\{file}")
}

/// The sound ids D2RUST's own audio tables request directly (NPC speech,
/// NPC greetings with the three time-of-day lines, object sounds).
fn table_sound_ids() -> BTreeSet<usize> {
    const TSVS: [(&str, &[&str]); 3] = [
        (
            include_str!("../../../../tables/audio/npc-speech.tsv"),
            &["sound"],
        ),
        (
            include_str!("../../../../tables/audio/npc-greetings.tsv"),
            &["greet", "inactive", "time", "return"],
        ),
        (
            include_str!("../../../../tables/audio/object-sounds.tsv"),
            &[
                "mode0", "mode1", "mode2", "mode3", "mode4", "mode5", "mode6", "mode7", "loop_a",
                "loop_b",
            ],
        ),
    ];
    let mut ids = BTreeSet::new();
    for (tsv, cols) in TSVS {
        let mut lines = tsv.lines();
        let header: Vec<&str> = lines.next().unwrap_or_default().split('\t').collect();
        for line in lines {
            let cells: Vec<&str> = line.split('\t').collect();
            for col in cols {
                let Some(id) = header
                    .iter()
                    .position(|h| h == col)
                    .and_then(|c| cells.get(c))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .filter(|&id| id != 0)
                else {
                    continue;
                };
                // The time-of-day greeting picks `time + 0..=2`.
                let span = if *col == "time" { 3 } else { 1 };
                ids.extend(id..id + span);
            }
        }
    }
    ids
}

/// Every named cell of every table (lowercased) with the tables naming it:
/// the generous "referenced anywhere" set that tells used rows from unused.
fn referenced_values(tables: &Tables) -> BTreeMap<Vec<u8>, BTreeSet<String>> {
    let mut out: BTreeMap<Vec<u8>, BTreeSet<String>> = BTreeMap::new();
    for (name, t) in tables {
        for r in &t.records {
            for value in &r.cells {
                if named(value) {
                    out.entry(key(value.trim_ascii()))
                        .or_default()
                        .insert(name.clone());
                }
            }
        }
    }
    out
}

/// `value` is named by a table other than `own`.
fn named_elsewhere(refs: &BTreeMap<Vec<u8>, BTreeSet<String>>, value: &[u8], own: &[&str]) -> bool {
    named(value)
        && refs
            .get(&key(value.trim_ascii()))
            .is_some_and(|by| by.iter().any(|n| !own.contains(&n.as_str())))
}

/// The `sounds.txt` rows the game can play: rows named by another table or
/// requested by the audio tables, plus the members of their sound groups
/// (a pick draws among `id .. id + Group Size`).
fn used_sounds(tables: &Tables, refs: &BTreeMap<Vec<u8>, BTreeSet<String>>) -> BTreeSet<usize> {
    let Some(t) = tables.get("sounds.txt") else {
        return BTreeSet::new();
    };
    let mut roots = table_sound_ids();
    for row in 0..t.records.len() {
        if named_elsewhere(refs, cell(t, row, b"Sound"), &["sounds.txt"]) {
            roots.insert(row);
        }
    }
    let mut used = BTreeSet::new();
    for id in roots.into_iter().filter(|&id| id < t.records.len()) {
        let size = std::str::from_utf8(cell(t, id, b"Group Size"))
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(0)
            .max(1);
        used.extend(id..id + size);
    }
    used
}

/// The item codes the item-producing tables name: treasure class items,
/// cube inputs/outputs (`code,modifiers`), unique and set item bases.
fn item_refs(tables: &Tables) -> BTreeSet<Vec<u8>> {
    let mut out = BTreeSet::new();
    for (name, t) in tables {
        let columns: Vec<usize> = t
            .header
            .iter()
            .enumerate()
            .filter(|(_, h)| {
                let h = text(h).to_ascii_lowercase();
                match name.as_str() {
                    "treasureclassex.txt" => h
                        .strip_prefix("item")
                        .is_some_and(|n| n.parse::<u8>().is_ok()),
                    "cubemain.txt" => h.starts_with("input ") || h.starts_with("output"),
                    "uniqueitems.txt" => h == "code",
                    "setitems.txt" => h == "item",
                    _ => false,
                }
            })
            .map(|(c, _)| c)
            .collect();
        for r in &t.records {
            for &c in &columns {
                let value = r.cells.get(c).map_or(&[][..], Vec::as_slice);
                let code = value.split(|&b| b == b',').next().unwrap_or_default();
                if named(code) {
                    out.insert(key(code.trim_ascii()));
                }
            }
        }
    }
    out
}

/// An item row is used when it can spawn (`spawnable` 1) or an
/// item-producing table names its code.
fn item_used(t: &TxtTable, row: usize, refs: &BTreeSet<Vec<u8>>) -> bool {
    cell(t, row, b"spawnable").trim_ascii() == b"1"
        || refs.contains(&key(cell(t, row, b"code").trim_ascii()))
}

/// Findings fail the check. `info` lists data gaps on rows the game never
/// uses (a missing file no table or audio rule can request): reported only.
#[derive(Debug, Default)]
pub(super) struct Report {
    pub findings: Vec<String>,
    pub info: Vec<String>,
}

#[cfg(test)]
fn check(
    tables: &Tables,
    exists: impl Fn(&str) -> bool,
    animation: impl Fn(&str) -> bool,
) -> Vec<String> {
    report(tables, exists, animation).findings
}

pub(super) fn report(
    tables: &Tables,
    exists: impl Fn(&str) -> bool,
    animation: impl Fn(&str) -> bool,
) -> Report {
    let mut out = Vec::new();
    let mut info = Vec::new();
    let refs = referenced_values(tables);
    let sounds_used = used_sounds(tables, &refs);
    let items_named = item_refs(tables);
    let mut keys: BTreeMap<String, BTreeSet<Vec<u8>>> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let unique_tables: BTreeSet<_> = d2_data::patch::rules()
        .into_iter()
        .filter(|r| r.unique)
        .map(|r| r.txt_name)
        .collect();
    // Register keys before references, matching the schema's linker namespaces.
    for def in &schema().tables {
        let Some(t) = tables.get(&def.txt_name) else {
            continue;
        };
        for f in def.fields.iter().filter(|f| f.field_type.is_own_key()) {
            let Some(linker) = f.link.linker() else {
                continue;
            };
            if !seen.insert((def.txt_name.clone(), f.column.clone(), linker.to_owned())) {
                continue;
            }
            let set = keys.entry(linker.to_owned()).or_default();
            for row in 0..t.records.len() {
                let value = cell(t, row, &f.column);
                if named(value)
                    && !set.insert(link_key(value, f.field_type.linker_kind()))
                    && unique_tables.contains(&def.txt_name)
                {
                    // The linker keeps the first row of a name, so every later
                    // row with that name is unreachable: dead data, not a
                    // broken reference. The shipped tables contain one such row
                    // (monstats `cr_lancer8`); report it as info, like any other
                    // unused row. Numeric ids below stay findings.
                    info.push(format!(
                        "duplicate-id {}:{} {}={} (shadowed: the linker keeps the earlier row)",
                        def.txt_name,
                        t.records[row].line,
                        f.name(),
                        text(value)
                    ));
                }
            }
        }
    }
    seen.clear();
    for def in &schema().tables {
        let Some(t) = tables.get(&def.txt_name) else {
            continue;
        };
        for f in def.fields.iter().filter(|f| f.field_type.is_lookup()) {
            let Some(linker) = f.link.linker() else {
                continue;
            };
            if !seen.insert((def.txt_name.clone(), f.column.clone(), linker.to_owned())) {
                continue;
            }
            // Built-in/range/TC linkers have specialized runtime registration.
            let Some(set) = keys.get(linker) else {
                continue;
            };
            for row in 0..t.records.len() {
                let value = cell(t, row, &f.column);
                if named(value) && !set.contains(&link_key(value, f.field_type.linker_kind())) {
                    out.push(format!(
                        "orphan-reference {}:{} {}={} -> {linker}",
                        def.txt_name,
                        t.records[row].line,
                        f.name(),
                        text(value)
                    ));
                }
            }
        }
    }

    for (name, col) in [
        ("levels.txt", "Id"),
        ("lvlprest.txt", "Def"),
        ("skills.txt", "Id"),
    ] {
        if let Some(t) = tables.get(name) {
            let mut ids = BTreeSet::new();
            for row in 0..t.records.len() {
                let value = cell(t, row, col.as_bytes());
                if named(value) && !ids.insert(d2_data::compile::parse_int(value)) {
                    out.push(format!(
                        "duplicate-id {name}:{} {col}={}",
                        t.records[row].line,
                        text(value)
                    ));
                }
            }
        }
    }
    // MonStatsEx selects the mode flags and base weapon animation code.
    if let (Some(monsters), Some(modes)) = (tables.get("monstats.txt"), tables.get("monstats2.txt"))
    {
        let mode_ids: BTreeMap<_, _> = (0..modes.records.len())
            .map(|r| (key(cell(modes, r, b"Id")), r))
            .collect();
        for row in 0..monsters.records.len() {
            let Some(&m) = mode_ids.get(&key(cell(monsters, row, b"MonStatsEx"))) else {
                continue;
            };
            let token = text(cell(monsters, row, b"Code"));
            let weapon = text(cell(modes, m, b"BaseW"));
            if token.is_empty() || weapon.is_empty() {
                continue;
            }
            for mode in [
                "DT", "NU", "WL", "GH", "A1", "A2", "BL", "SC", "S1", "S2", "S3", "S4", "DD", "KB",
                "RN",
            ] {
                if cell(modes, m, format!("m{mode}").as_bytes()) != b"1" {
                    continue;
                }
                // Composite rules use GH for monster knockback and hth for
                // non-composite death. SQ draws sequence modes, not a SQ COF.
                // See d2-client/rules/unit_composite and units/anim sequences.
                let drawn = if mode == "KB" { "GH" } else { mode };
                let weapon =
                    if matches!(mode, "DT" | "DD") && cell(modes, m, b"compositeDeath") != b"1" {
                        "hth"
                    } else {
                        &weapon
                    };
                let path =
                    format!("data\\global\\monsters\\{token}\\cof\\{token}{drawn}{weapon}.cof");
                if !animation(&format!("{token}{drawn}{weapon}")) {
                    out.push(format!("monster-animdata monstats.txt:{} mode={mode} missing {token}{drawn}{weapon}",monsters.records[row].line));
                }
                if !exists(&path) {
                    out.push(format!(
                        "monster-animation monstats.txt:{} mode={mode} missing {path}",
                        monsters.records[row].line
                    ));
                }
            }
        }
    }
    // Presets refer back to levels by numeric id (not a schema lookup field).
    if let (Some(levels), Some(presets)) = (tables.get("levels.txt"), tables.get("lvlprest.txt")) {
        let ids: BTreeSet<_> = (0..levels.records.len())
            .map(|r| d2_data::compile::parse_int(cell(levels, r, b"Id")))
            .collect();
        for row in 0..presets.records.len() {
            let id = cell(presets, row, b"LevelId");
            if named(id) && id != b"0" && !ids.contains(&d2_data::compile::parse_int(id)) {
                out.push(format!(
                    "level-preset lvlprest.txt:{} LevelId={} missing level",
                    presets.records[row].line,
                    text(id)
                ));
            }
        }
    }
    let mut files = BTreeMap::new();
    for (name, t) in tables {
        for row in 0..t.records.len() {
            for (col, value) in t.header.iter().zip(&t.records[row].cells) {
                if value.len() <= 1 || !named(value) {
                    continue;
                }
                let col = text(col).to_ascii_lowercase();
                let value = text(value).replace('/', "\\");
                let paths = if matches!(
                    name.as_str(),
                    "weapons.txt" | "armor.txt" | "misc.txt" | "uniqueitems.txt" | "setitems.txt"
                ) && matches!(
                    col.as_str(),
                    "invfile" | "flippyfile" | "uniqueinvfile" | "setinvfile"
                ) {
                    let dir = "items";
                    vec![format!("data\\global\\{dir}\\{value}.dc6")]
                } else if name == "missiles.txt" && col == "celfile" {
                    vec![
                        format!("data\\global\\missiles\\{value}.dcc"),
                        format!("data\\global\\missiles\\{value}.dc6"),
                    ]
                } else if name == "sounds.txt" && col == "filename" {
                    // No audio exists for these rows; D2RUST never plays them.
                    if value.eq_ignore_ascii_case("none.wav")
                        || d2_data::sounds::has_no_audio(row as u32)
                    {
                        continue;
                    }
                    vec![sound_path(row, &value)]
                } else if matches!(
                    name.as_str(),
                    "lvlprest.txt" | "lvlsub.txt" | "lvltypes.txt"
                ) && col.starts_with("file")
                {
                    vec![format!("data\\global\\tiles\\{value}")]
                } else {
                    continue;
                };
                let present = paths
                    .iter()
                    .any(|p| *files.entry(p.clone()).or_insert_with(|| exists(p)));
                if !present {
                    let line = format!(
                        "missing-file {name}:{} {col}={value} expected {}",
                        t.records[row].line,
                        paths.join(" or ")
                    );
                    let unused = match name.as_str() {
                        "sounds.txt" => !sounds_used.contains(&row),
                        "weapons.txt" | "armor.txt" | "misc.txt" => {
                            !item_used(t, row, &items_named)
                        }
                        _ => false,
                    };
                    if unused {
                        info.push(format!("data gap (row unused): {line}"));
                    } else {
                        out.push(line);
                    }
                }
            }
        }
    }
    // One finding per defect, even when two linkers see the same key.
    let mut once = BTreeSet::new();
    out.retain(|f| once.insert(f.clone()));
    let mut once_info = BTreeSet::new();
    info.retain(|f| once_info.insert(f.clone()));
    Report {
        findings: out,
        info,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn table(s: &str) -> TxtTable {
        TxtTable::parse("synthetic", s.replace('\n', "\r\n").as_bytes()).unwrap()
    }
    #[test]
    fn synthetic_fixture_references_are_consistent() {
        let fixture = test_fixtures::synth::synthetic();
        let tables = fixture
            .tables
            .render()
            .into_iter()
            .map(|(n, b)| {
                let t = TxtTable::parse(&n, &b).unwrap();
                (n, t)
            })
            .collect();
        let out = check(&tables, |_| true, |_| true);
        assert!(out.is_empty(), "{out:#?}");
    }
    #[test]
    fn numeric_ids_detect_duplicates() {
        let t = Tables::from([("levels.txt".into(), table("Id\n1\n01\n"))]);
        assert!(check(&t, |_| true, |_| true)
            .iter()
            .any(|s| s.contains("duplicate-id") && s.contains("01")));
    }
    #[test]
    fn repeated_sequence_keys_are_legal() {
        let t = Tables::from([("monseq.txt".into(), table("sequence\nsequence\nsequence\n"))]);
        assert!(check(&t, |_| true, |_| true).is_empty());
    }
    #[test]
    fn sound_paths_follow_runtime_ranges_and_silence_is_optional() {
        for (row, prefix) in [
            (2933, "global\\sfx"),
            (2934, "local\\sfx"),
            (4656, "local\\sfx"),
            (4657, "global\\music"),
            (4698, "global\\music"),
            (4699, "global\\sfx"),
        ] {
            assert_eq!(
                sound_path(row, "test.wav"),
                format!("data\\{prefix}\\test.wav")
            );
        }
        let t = Tables::from([("sounds.txt".into(), table("FileName\nnone.wav\n"))]);
        assert!(check(&t, |_| false, |_| false).is_empty());
    }
    #[test]
    fn monster_drawn_modes_use_composite_rules() {
        let t = Tables::from([
            (
                "monstats.txt".into(),
                table("Id\tCode\tMonStatsEx\nmonster\tzz\tmode\n"),
            ),
            (
                "monstats2.txt".into(),
                table("Id\tBaseW\tmKB\tmDT\tmSQ\nmode\t1hs\t1\t1\t1\n"),
            ),
        ]);
        let out = check(
            &t,
            |p| p.ends_with("zzGH1hs.cof") || p.ends_with("zzDThth.cof"),
            |n| matches!(n, "zzGH1hs" | "zzDThth"),
        );
        assert!(out.is_empty(), "{out:?}");
    }
    #[test]
    fn planted_orphan_and_duplicate() {
        let mut t = Tables::new();
        t.insert("missiles.txt".into(), table("missile\nbolt\nbolt\n"));
        t.insert(
            "skills.txt".into(),
            table("skill\tsrvmissile\ncast\tabsent\n"),
        );
        let r = report(&t, |_| true, |_| true);
        assert!(
            r.info
                .iter()
                .any(|s| s.contains("duplicate-id") && s.contains("bolt")),
            "{:?}",
            r.info
        );
        assert!(
            r.findings
                .iter()
                .any(|s| s.contains("orphan-reference") && s.contains("absent")),
            "{:?}",
            r.findings
        );
    }
    #[test]
    fn every_file_rule_detects_planted_failure() {
        for (name, col) in [
            ("weapons.txt", "invfile"),
            ("armor.txt", "flippyfile"),
            ("misc.txt", "uniqueinvfile"),
            ("misc.txt", "setinvfile"),
            ("uniqueitems.txt", "invfile"),
            ("setitems.txt", "invfile"),
            ("missiles.txt", "CelFile"),
            ("sounds.txt", "FileName"),
            ("lvlprest.txt", "File1"),
            ("lvlsub.txt", "File"),
            ("lvltypes.txt", "File 1"),
        ] {
            let t = Tables::from([(name.into(), table(&format!("{col}\nsynthetic\n")))]);
            assert!(check(&t, |_| true, |_| true).is_empty(), "{name}");
            // Reported either way: a finding on a used row, info on an
            // unused one.
            let r = report(&t, |_| false, |_| false);
            assert_eq!(r.findings.len() + r.info.len(), 1, "{name}");
        }
    }
    #[test]
    fn comment_and_blank_cells_are_not_references() {
        let t = Tables::from([
            ("properties.txt".into(), table("code\nstr\n")),
            (
                "uniqueitems.txt".into(),
                table("index\tprop1\tprop2\nring\t*enr\tstr\n"),
            ),
            ("elemtypes.txt".into(), table("code\nfire\n")),
            (
                "missiles.txt".into(),
                table("missile\tEType\nbolt\t  \nfireball\tfire\n"),
            ),
            ("monstats.txt".into(), table("Id\nzombie\n")),
            (
                "monequip.txt".into(),
                table("monster\nzombie\n*end*  do not remove\n"),
            ),
        ]);
        assert!(check(&t, |_| true, |_| true).is_empty());
        // A real dangling reference next to them is still found.
        let t = Tables::from([
            ("properties.txt".into(), table("code\nstr\n")),
            ("uniqueitems.txt".into(), table("index\tprop1\nring\tenr\n")),
        ]);
        let out = check(&t, |_| true, |_| true);
        assert!(out.iter().any(|s| s.contains("prop1=enr")), "{out:?}");
    }
    #[test]
    fn a_duplicate_is_reported_once() {
        let t = Tables::from([("monstats.txt".into(), table("Id\nlancer\nlancer\n"))]);
        let r = report(&t, |_| true, |_| true);
        assert!(r.findings.is_empty(), "{:?}", r.findings);
        assert_eq!(
            r.info.iter().filter(|s| s.contains("duplicate-id")).count(),
            1,
            "{:?}",
            r.info
        );
    }
    #[test]
    fn missing_sounds_of_unused_rows_are_info() {
        // Rows: 0 named by monsounds (group of 2: rows 0 and 1), 2 unused,
        // then filler up to the first id the audio tables request.
        let requested = *table_sound_ids().iter().next().unwrap();
        let mut sounds = String::from("Sound\tFileName\tGroup Size\n");
        sounds += "grunt\tgrunt1.wav\t2\ngrunt2\tgrunt2.wav\t0\nunused\tunused.wav\t0\n";
        for row in 3..=requested {
            sounds += &format!("s{row}\tf{row}.wav\t0\n");
        }
        let t = Tables::from([
            ("sounds.txt".into(), table(&sounds)),
            (
                "monsounds.txt".into(),
                table("Id\tNeutral\nzombie\tgrunt\n"),
            ),
        ]);
        let missing = |p: &str| {
            ["grunt2.wav", "unused.wav", &format!("f{requested}.wav")]
                .iter()
                .any(|f| p.ends_with(f))
        };
        let r = report(&t, |p| !missing(p), |_| true);
        assert_eq!(r.findings.len(), 2, "{r:?}");
        assert!(r.findings.iter().any(|s| s.contains("grunt2.wav")), "{r:?}");
        assert!(
            r.findings
                .iter()
                .any(|s| s.contains(&format!("f{requested}.wav"))),
            "{r:?}"
        );
        assert_eq!(r.info.len(), 1, "{r:?}");
        assert!(r.info[0].contains("unused.wav"), "{r:?}");
    }
    #[test]
    fn sounds_without_audio_are_never_reported() {
        // A sounds.txt long enough to hold every excluded row, each row named
        // by monsounds so it counts as used; all their files are missing.
        let last = *d2_data::sounds::SOUNDS_WITHOUT_AUDIO.iter().max().unwrap() as usize;
        let mut sounds = String::from(
            "Sound	FileName	Group Size
",
        );
        let mut mon = String::from(
            "Id	Neutral
",
        );
        for row in 0..=last {
            sounds += &format!(
                "s{row}	f{row}.wav	0
"
            );
            mon += &format!(
                "m{row}	s{row}
"
            );
        }
        let t = Tables::from([
            ("sounds.txt".into(), table(&sounds)),
            ("monsounds.txt".into(), table(&mon)),
        ]);
        let r = report(&t, |_| false, |_| true);
        for id in d2_data::sounds::SOUNDS_WITHOUT_AUDIO {
            let file = format!("f{id}.wav");
            let hit = |s: &String| s.contains(&format!("filename={file} "));
            assert!(!r.findings.iter().any(hit), "finding for {id}");
            assert!(!r.info.iter().any(hit), "info for {id}");
        }
        // Their neighbours are still reported.
        let neighbour = format!("filename=f{}.wav ", last - 1);
        assert!(
            r.findings.iter().any(|s| s.contains(&neighbour)),
            "{}",
            r.findings.len()
        );
    }
    #[test]
    fn missing_item_art_of_unspawnable_rows_is_info() {
        let t = Tables::from([
            (
                "misc.txt".into(),
                table("code\tspawnable\tflippyfile\ntal\t0\tflptail\nhrt\t0\tflphrt\nelx\t1\tflpelx\n"),
            ),
            (
                "cubemain.txt".into(),
                table("description\tinput 1\nheart\thrt,qty=2\n"),
            ),
            // A gem letter that happens to equal an item code names nothing.
            ("gems.txt".into(), table("name\tletter\nTal Rune\tTal\n")),
        ]);
        let r = report(&t, |_| false, |_| true);
        assert_eq!(r.info.len(), 1, "{r:?}");
        assert!(r.info[0].contains("flptail"), "{r:?}");
        assert_eq!(r.findings.len(), 2, "{r:?}");
    }
    #[test]
    fn planted_monster_animation_and_level_preset() {
        let t = Tables::from([
            (
                "monstats.txt".into(),
                table("Id\tCode\tMonStatsEx\nmonster\tzz\tmode\n"),
            ),
            (
                "monstats2.txt".into(),
                table("Id\tBaseW\tmNU\nmode\thth\t1\n"),
            ),
            ("levels.txt".into(), table("Id\n1\n")),
            ("lvlprest.txt".into(), table("LevelId\n99\n")),
        ]);
        let out = check(&t, |_| false, |_| false);
        assert!(
            out.iter()
                .any(|s| s.contains("monster-animation") && s.contains("zzNUhth.cof")),
            "{out:?}"
        );
        assert!(
            out.iter().any(|s| s.contains("monster-animdata")),
            "{out:?}"
        );
        assert!(
            out.iter()
                .any(|s| s.contains("level-preset") && s.contains("99")),
            "{out:?}"
        );
    }
    #[test]
    fn valid_reference_and_empty_optional_reference() {
        let t = Tables::from([
            ("missiles.txt".into(), table("missile\nbolt\n")),
            (
                "skills.txt".into(),
                table("skill\tsrvmissile\ncast\tbolt\nidle\t\n"),
            ),
        ]);
        assert!(check(&t, |_| true, |_| true).is_empty());
    }
    #[test]
    fn planted_monster_mode_and_item_sound_orphans() {
        let t = Tables::from([
            (
                "monstats.txt".into(),
                table("Id\tMonStatsEx\nmonster\tabsent-mode\n"),
            ),
            ("monstats2.txt".into(), table("Id\nmode\n")),
            (
                "weapons.txt".into(),
                table("code\tdropsound\nabc\tabsent-sound\n"),
            ),
            ("sounds.txt".into(), table("Sound\nsound\n")),
        ]);
        let out = check(&t, |_| true, |_| true);
        for value in ["absent-mode", "absent-sound"] {
            assert!(
                out.iter()
                    .any(|s| s.contains("orphan-reference") && s.contains(value)),
                "{out:?}"
            );
        }
    }
}
