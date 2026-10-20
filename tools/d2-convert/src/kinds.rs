//! The kinds this binary converts.
//!
//! The image and text kinds are adapters over the `d2-native`
//! reader and writer of that kind ([`Native`]); the verbatim kinds,
//! `excel` and `colormap`, are copies checked by byte
//! equality.

use d2_native::asset::{AssetError, AssetKind};
use d2_native::kind::NativeFile;

use crate::kind::{Failure, Kind, NativeData, Written};

/// All kinds this build converts, in registry order.
pub fn builtin() -> Vec<Box<dyn Kind>> {
    let mut v: Vec<Box<dyn Kind>> = vec![Box::new(Excel), Box::new(ColorMap)];
    v.extend(
        AssetKind::ALL
            .into_iter()
            .map(|k| Box::new(Native(k)) as Box<dyn Kind>),
    );
    v
}

/// Audio is not converted for now (an open choice: no sound for now): no
/// kind claims `.wav`, so the report lists these files with this reason.
pub const SKIPPED_EXTENSIONS: &[(&str, &str)] =
    &[("wav", "audio not converted for now (native-assets.md OQ3)")];

/// Files a kind claims by name that the game never loads: not converted,
/// counted as skipped and listed in the report with the reason. Canonical
/// paths.
pub const NEVER_LOADED: &[(&str, &str)] = &[
    (
        "data/global/chars/am/cof/amblxbow.cof",
        "not a COF; its name breaks the token + mode + class pattern (cof.md)",
    ),
    (
        "data/global/tiles/act1/barracks/barracks.dt1",
        "version-4 DT1 named by no LvlTypes, LvlPrest or DS1 (dt1.md)",
    ),
    (
        "data/global/tiles/act1/barracks/gargtrap.dt1",
        "version-4 DT1 named by no LvlTypes, LvlPrest or DS1 (dt1.md)",
    ),
    (
        "data/global/tiles/act1/catacomb/catacombs.dt1",
        "version-4 DT1 named by no LvlTypes, LvlPrest or DS1 (dt1.md)",
    ),
    (
        "data/global/tiles/act1/cathedrl/cathedrl.dt1",
        "version-4 DT1 named by no LvlTypes, LvlPrest or DS1 (dt1.md)",
    ),
    (
        "data/global/tiles/act1/court/court.dt1",
        "version-4 DT1 named by no LvlTypes, LvlPrest or DS1 (dt1.md)",
    ),
    (
        "data/global/tiles/act1/outdoors/outdoor1.dt1",
        "version-4 DT1 named by no LvlTypes, LvlPrest or DS1 (dt1.md)",
    ),
    (
        "data/local/font/latin/default.tbl",
        "plain text, not a font table; Game.exe never names it (tbl.md OQ2)",
    ),
    (
        "data/local/font/latin/fonter.tbl",
        "plain text, not a font table; Game.exe never names it (tbl.md OQ2)",
    ),
];

/// The reason `canon` is never loaded, if it is on [`NEVER_LOADED`].
pub fn never_loaded(canon: &str) -> Option<&'static str> {
    NEVER_LOADED
        .iter()
        .find(|(p, _)| *p == canon)
        .map(|(_, why)| *why)
}

/// Adapter over `d2_native::asset::AssetKind` (the image kinds and the
/// text kinds).
pub struct Native(pub AssetKind);

fn fail(e: AssetError) -> Failure {
    Failure::new(e.check, e.detail)
}

impl Kind for Native {
    fn name(&self) -> &'static str {
        self.0.name()
    }
    fn native_version(&self) -> u32 {
        self.0.native_version()
    }
    fn phase(&self) -> u8 {
        self.0.phase()
    }
    fn claims(&self, canon: &str) -> bool {
        self.0.claims(canon)
    }
    fn has_references(&self) -> bool {
        self.0.has_references()
    }
    fn references(&self, canon: &str, src: &[u8]) -> Vec<String> {
        self.0.references(canon, src)
    }
    fn write(&self, canon: &str, src: &[u8]) -> Result<Written, Failure> {
        let w = self.0.write(canon, src).map_err(fail)?;
        Ok(Written {
            files: w
                .files
                .into_iter()
                .map(|f| NativeData {
                    name: f.path,
                    bytes: f.bytes,
                })
                .collect(),
            notes: w.notes,
        })
    }
    fn check(&self, canon: &str, src: &[u8], native: &[NativeData]) -> Result<(), Failure> {
        let files: Vec<NativeFile> = native
            .iter()
            .map(|n| NativeFile {
                path: n.name.clone(),
                bytes: n.bytes.clone(),
            })
            .collect();
        self.0.check(canon, src, &files).map_err(fail)
    }
}

/// `data/global/excel/*.txt`: a byte-for-byte copy; the check is
/// byte equality (C-TABLE step 1). The step that compiles the native set
/// against each live `.bin` (C-TABLE step 2, `_bin-overrides.toml`) comes
/// with the native tables (`tables.rs`).
pub struct Excel;

impl Kind for Excel {
    fn name(&self) -> &'static str {
        "excel"
    }
    fn native_version(&self) -> u32 {
        1
    }
    fn phase(&self) -> u8 {
        0
    }
    fn claims(&self, canon: &str) -> bool {
        canon
            .strip_prefix("data/global/excel/")
            .is_some_and(|rest| !rest.contains('/') && rest.ends_with(".txt"))
    }
    fn write(&self, canon: &str, src: &[u8]) -> Result<Written, Failure> {
        Ok(copy(canon, src))
    }
    fn check(&self, canon: &str, src: &[u8], native: &[NativeData]) -> Result<(), Failure> {
        check_copy("C-TABLE", canon, src, native)
    }
}

/// `data/global/items/palette/*.dat` item colour maps: a byte-for-byte
/// copy; the check is byte equality (C-COPY).
pub struct ColorMap;

impl Kind for ColorMap {
    fn name(&self) -> &'static str {
        "colormap"
    }
    fn native_version(&self) -> u32 {
        1
    }
    fn phase(&self) -> u8 {
        1
    }
    fn claims(&self, canon: &str) -> bool {
        d2_native::source::is_colormap(canon)
    }
    fn write(&self, canon: &str, src: &[u8]) -> Result<Written, Failure> {
        Ok(copy(canon, src))
    }
    fn check(&self, canon: &str, src: &[u8], native: &[NativeData]) -> Result<(), Failure> {
        check_copy("C-COPY", canon, src, native)
    }
}

/// The one native file of a verbatim kind: `canon` with the source bytes.
fn copy(canon: &str, src: &[u8]) -> Written {
    Written {
        files: vec![NativeData {
            name: canon.to_owned(),
            bytes: src.to_vec(),
        }],
        notes: Vec::new(),
    }
}

/// A verbatim kind's check: exactly one native file, named `canon`, with
/// the source bytes. Failures carry `check`'s name.
fn check_copy(
    check: &'static str,
    canon: &str,
    src: &[u8],
    native: &[NativeData],
) -> Result<(), Failure> {
    let [one] = native else {
        return Err(Failure::new(
            check,
            format!("{} native files", native.len()),
        ));
    };
    if one.name != canon {
        return Err(Failure::new(check, format!("native name {}", one.name)));
    }
    match src.iter().zip(&one.bytes).position(|(a, b)| a != b) {
        Some(at) => Err(Failure::new(
            check,
            format!("{canon}: first difference at byte {at}"),
        )),
        None if src.len() != one.bytes.len() => Err(Failure::new(
            check,
            format!("{canon}: length {} vs {}", one.bytes.len(), src.len()),
        )),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_loaded_paths_are_canonical_and_claimed() {
        let kinds = builtin();
        assert_eq!(NEVER_LOADED.len(), 9);
        for (p, _) in NEVER_LOADED {
            assert_eq!(crate::names::canonical(p).as_deref(), Some(*p));
            assert!(
                kinds.iter().any(|k| k.claims(p)),
                "{p} is claimed by no kind"
            );
        }
        assert!(never_loaded("data/global/tiles/act1/court/court.dt1").is_some());
        assert!(never_loaded("data/local/font/latin/font8.tbl").is_none());
    }

    #[test]
    fn colormap_is_a_verbatim_copy() {
        let kinds = builtin();
        let p = "data/global/items/palette/invgrey.dat";
        let claim: Vec<_> = kinds.iter().filter(|k| k.claims(p)).collect();
        assert_eq!(claim.len(), 1);
        let k = claim[0];
        assert_eq!(k.name(), "colormap");
        for other in [
            "data/global/palette/act1/pal.dat",
            "data/global/items/palette/sub/x.dat",
        ] {
            assert!(!k.claims(other), "{other}");
        }
        let src: Vec<u8> = (0..21 * 256).map(|i| (i % 251) as u8).collect();
        let w = k.write(p, &src).unwrap();
        assert_eq!(w.files.len(), 1);
        assert_eq!(w.files[0].name, p);
        k.check(p, &src, &w.files).unwrap();
        let mut bad = w.files.clone();
        bad[0].bytes[300] ^= 1;
        assert_eq!(k.check(p, &src, &bad).unwrap_err().check, "C-COPY");
    }
}
