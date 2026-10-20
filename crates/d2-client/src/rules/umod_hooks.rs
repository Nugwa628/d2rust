//! The client hooks of monster umods, and the order a monster's umods run them in.
//!
//! A unique or champion monster carries a list of up to nine umods (its special abilities:
//! extra strong, fire enchanted, ...). On the client some umods have hooks that add effects
//! (client missiles, overlays, sounds) at one of five phases of the monster's client update.
//! [`HOOKS`] says which hook (if any) each umod runs at each phase, and [`hooks_for`] which
//! hooks a monster's list calls in a phase, in order. The hook bodies belong to the effect
//! layer.
//!
//! A hook is named by an opaque id, the one `tables/monsters/umods.tsv` lists in its
//! `cl_phase0`-`cl_phase4` columns. Ids only tell hooks apart: umods that share an id share
//! one hook body.

/// Umods with a row in the table (0-42).
pub const UMODS: usize = 43;
/// Client update phases 0-4.
pub const PHASES: usize = 5;

const fn row(
    u: usize,
    phase: usize,
    hook: u32,
    mut t: [[u32; PHASES]; UMODS],
) -> [[u32; PHASES]; UMODS] {
    t[u][phase] = hook;
    t
}

/// The hook id of every (umod, phase), 0 for no hook.
pub const HOOKS: [[u32; PHASES]; UMODS] = {
    let t = [[0; PHASES]; UMODS];
    let t = row(9, 2, 0x004A_D1A0, t);
    let t = row(10, 2, 0x004A_D510, t);
    let t = row(11, 2, 0x004A_D5F0, t);
    let t = row(17, 2, 0x004A_D890, t);
    let t = row(17, 3, 0x004A_D8E0, t);
    let t = row(18, 2, 0x004A_D510, t);
    let t = row(20, 2, 0x004A_D920, t);
    let t = row(22, 1, 0x004A_D2F0, t);
    let t = row(23, 0, 0x004A_D890, t);
    let t = row(29, 4, 0x004A_D970, t);
    let t = row(31, 2, 0x004A_D0C0, t);
    let t = row(32, 2, 0x004A_DB20, t);
    let t = row(33, 2, 0x004A_DBC0, t);
    let t = row(40, 2, 0x004A_DD80, t);
    row(42, 2, 0x004A_D510, t)
};

/// The hooks a monster's 9-byte umod list calls in `phase`, as (umod, hook id) in list
/// order. Every byte is visited, **including bytes after a 0**: an empty entry does not end
/// the list (umod 0 simply has no hooks). A byte of 43 or more names no umod, and the whole
/// list is refused (`None`); so is a phase past the table.
pub fn hooks_for(umods: &[u8; 9], phase: usize) -> Option<Vec<(u8, u32)>> {
    let mut out = Vec::new();
    for &u in umods {
        let hook = *HOOKS.get(usize::from(u))?.get(phase)?;
        if hook != 0 {
            out.push((u, hook));
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TSV: &str = include_str!("../../../../tables/monsters/umods.tsv");

    /// The (umod, phase, hook) mismatches between `umods.tsv`'s
    /// `cl_phase*` columns and [`HOOKS`].
    fn mismatches(tsv: &str) -> Vec<(usize, usize)> {
        let mut lines = tsv.lines();
        let header: Vec<&str> = lines.next().unwrap().split('\t').collect();
        let col = |n: &str| header.iter().position(|h| *h == n).unwrap();
        let id = col("id");
        let cols: Vec<usize> = (0..PHASES).map(|p| col(&format!("cl_phase{p}"))).collect();
        let mut seen = [false; UMODS];
        let mut out = Vec::new();
        for l in lines {
            let f: Vec<&str> = l.split('\t').collect();
            let u: usize = f[id].parse().unwrap();
            if u >= UMODS {
                out.push((u, PHASES));
                continue;
            }
            seen[u] = true;
            for (p, &c) in cols.iter().enumerate() {
                let want = match f[c] {
                    "-" => 0,
                    s => u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap(),
                };
                if HOOKS[u][p] != want {
                    out.push((u, p));
                }
            }
        }
        out.extend((0..UMODS).filter(|&u| !seen[u]).map(|u| (u, PHASES)));
        out
    }

    #[test]
    fn hooks_match_the_tsv() {
        assert_eq!(mismatches(TSV), []);
        // A changed cell is reported exactly.
        let bad = TSV.replacen("0x004AD970", "0x004AD971", 1);
        assert_ne!(bad, TSV);
        assert_eq!(mismatches(&bad), [(29, 4)]);
    }

    #[test]
    fn dispatch_visits_every_byte_in_order() {
        // Bytes after a 0 still run; 0 and hookless umods add nothing.
        let list = [17, 0, 9, 1, 17, 0, 0, 0, 42];
        assert_eq!(
            hooks_for(&list, 2),
            Some(vec![
                (17, 0x004A_D890),
                (9, 0x004A_D1A0),
                (17, 0x004A_D890),
                (42, 0x004A_D510)
            ])
        );
        assert_eq!(
            hooks_for(&list, 3),
            Some(vec![(17, 0x004A_D8E0), (17, 0x004A_D8E0)])
        );
        assert_eq!(hooks_for(&[0; 9], 4), Some(vec![]));
        assert_eq!(hooks_for(&[43, 0, 0, 0, 0, 0, 0, 0, 0], 0), None);
    }
}
