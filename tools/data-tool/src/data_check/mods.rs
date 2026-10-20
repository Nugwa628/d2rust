use anyhow::{bail, Result};
use std::{collections::BTreeSet, path::Path};
fn visit(
    name: &str,
    read: &impl Fn(&str) -> Result<Vec<String>>,
    visiting: &mut BTreeSet<String>,
    done: &mut BTreeSet<String>,
    out: &mut Vec<String>,
) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':']) {
        bail!("invalid mod name {name:?}");
    }
    if done.contains(name) {
        return Ok(());
    }
    if !visiting.insert(name.into()) {
        bail!("cyclic mod dependency at {name}");
    }
    for dependency in read(name)? {
        visit(&dependency, read, visiting, done, out)?;
    }
    visiting.remove(name);
    done.insert(name.into());
    out.push(name.into());
    Ok(())
}
fn resolve(name: &str, read: impl Fn(&str) -> Result<Vec<String>>) -> Result<Vec<String>> {
    let mut out = Vec::new();
    visit(
        name,
        &read,
        &mut BTreeSet::new(),
        &mut BTreeSet::new(),
        &mut out,
    )?;
    Ok(out)
}
pub(super) fn order(root: &Path, name: &str) -> Result<Vec<String>> {
    resolve(name, |n| {
        let file = root.join("mods").join(n).join("mod.toml");
        let info = d2_native::layers::ModInfo::from_toml(
            &file.display().to_string(),
            &std::fs::read_to_string(&file)?,
        )?;
        Ok(info.requires)
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dependencies_precede_selected_mod_without_duplicates() {
        let result = resolve("selected", |n| {
            Ok(match n {
                "selected" => vec!["base-mod".into(), "other".into()],
                "other" => vec!["base-mod".into()],
                _ => vec![],
            })
        })
        .unwrap();
        assert_eq!(result, ["base-mod", "other", "selected"]);
    }
    #[test]
    fn cycles_and_paths_are_rejected() {
        assert!(resolve("cycle", |_| Ok(vec!["cycle".into()])).is_err());
        for name in ["../bad", "a/b", "a\\b", "C:", "..", ""] {
            assert!(resolve(name, |_| Ok(vec![])).is_err());
        }
    }
}
