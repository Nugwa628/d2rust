//! Every top-level integration test must belong to exactly one enabled target.
use std::collections::BTreeMap;
use std::path::Path;

fn suggested_target(file: &str) -> &'static str {
    if file.starts_with("app_") {
        "app"
    } else if file.starts_with("front_") || file.starts_with("smoke_frontend") {
        "front_end"
    } else if file.starts_with("prop_") {
        "properties"
    } else if file.starts_with("ui_") || file.starts_with("game_") {
        "ui"
    } else {
        "world"
    }
}

fn problems(files: &[String], registrations: &BTreeMap<String, Vec<String>>) -> Vec<String> {
    files.iter().filter_map(|file| {
        match registrations.get(file).map(Vec::as_slice).unwrap_or_default() {
            [] => Some(format!("tests/{file} is unregistered: add #[path = \"../{file}\"] mod {}; to tests/{}/main.rs", file.trim_end_matches(".rs"), suggested_target(file))),
            [_] => None,
            targets => Some(format!("tests/{file} is registered more than once: {}", targets.join(", "))),
        }
    }).collect()
}

#[test]
fn every_integration_file_is_registered_exactly_once() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(root.join("Cargo.toml"))
        .unwrap()
        .parse::<toml_edit::DocumentMut>()
        .unwrap();
    assert_eq!(manifest["package"]["autotests"].as_bool(), Some(false));
    let targets = manifest["test"]
        .as_array_of_tables()
        .expect("explicit test targets");
    let mut registrations = BTreeMap::<String, Vec<String>>::new();
    for target in targets {
        let path = target["path"].as_str().expect("target path");
        if let Some(file) = path
            .strip_prefix("tests/")
            .filter(|file| !file.contains('/') && file.ends_with(".rs"))
        {
            registrations
                .entry(file.to_owned())
                .or_default()
                .push(path.to_owned());
        }
        let source = std::fs::read_to_string(root.join(path)).expect("target exists");
        for line in source.lines() {
            if let Some(file) = line
                .trim()
                .strip_prefix("#[path = \"../")
                .and_then(|s| s.strip_suffix("\"]"))
            {
                registrations
                    .entry(file.to_owned())
                    .or_default()
                    .push(path.to_owned());
            }
        }
    }
    let mut files: Vec<_> = std::fs::read_dir(root.join("tests"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|e| e == "rs"))
        .map(|path| path.file_name().unwrap().to_str().unwrap().to_owned())
        .collect();
    files.sort();
    let failures = problems(&files, &registrations);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn an_unregistered_file_names_the_file_and_target() {
    let failures = problems(&["app_new_feature.rs".into()], &BTreeMap::new());
    assert_eq!(failures.len(), 1);
    assert!(failures[0].contains("tests/app_new_feature.rs"));
    assert!(failures[0].contains("tests/app/main.rs"));
    assert!(failures[0].contains("mod app_new_feature;"));
}

#[test]
fn duplicate_registration_is_rejected() {
    let registrations = BTreeMap::from([("sample.rs".into(), vec!["a".into(), "b".into()])]);
    assert!(problems(&["sample.rs".into()], &registrations)[0].contains("more than once"));
}
