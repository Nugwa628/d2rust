//! The server-do / server-hit bodies implemented here are exactly the
//! non-null catalogue rows whose `status` is `spec'd-here` (METHODS M05):
//! a row promoted in a TSV without a body here, or a body landing for a
//! row not yet fully covered, fails this check.

use super::catalogue::{SRVDO_TSV, SRVHIT_TSV, SRV_DO_IMPLEMENTED, SRV_HIT_IMPLEMENTED};

/// Indices of the non-null (`addr_114d` ≠ `-`) rows with status
/// `spec'd-here`.
fn spec_d_here(tsv: &str) -> Vec<i16> {
    tsv.lines()
        .skip(1)
        .filter_map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            (c[1] != "-" && c[8] == "spec'd-here").then(|| c[0].parse().unwrap())
        })
        .collect()
}

fn check(tsv: &str, implemented: &[i16]) -> Result<(), String> {
    let spec = spec_d_here(tsv);
    let mut ours = implemented.to_vec();
    ours.sort_unstable();
    if spec == ours {
        Ok(())
    } else {
        Err(format!("spec'd-here {spec:?}, implemented {ours:?}"))
    }
}

#[test]
fn bodies_match_catalogue_status() {
    check(SRVDO_TSV, &SRV_DO_IMPLEMENTED).unwrap();
    check(SRVHIT_TSV, &SRV_HIT_IMPLEMENTED).unwrap();
}

#[test]
fn bodies_check_catches_perturbations() {
    // M08: a non-null row promoted to `spec'd-here` without a body is
    // reported. No catalogue row is left below `spec'd-here`, so the
    // fixture appends a synthetic one (index 53, past the 53-entry
    // table, so no body can exist for it).
    let bad = format!("{SRVDO_TSV}53\t0x00000000\t-\t0\t-\t-\t-\tsynthetic\tspec'd-here\n");
    let mut want: Vec<i16> = SRV_DO_IMPLEMENTED.to_vec();
    want.sort_unstable();
    let ours = format!("{want:?}");
    want.push(53);
    assert_eq!(
        check(&bad, &SRV_DO_IMPLEMENTED),
        Err(format!("spec'd-here {want:?}, implemented {ours}"))
    );
    // The same row with any other status is not counted.
    let other = bad.replace("\tsynthetic\tspec'd-here\n", "\tsynthetic\tsummarized\n");
    assert_eq!(check(&other, &SRV_DO_IMPLEMENTED), Ok(()));
    // Demoting a real row (server-hit 2) while its body stays is reported.
    let row2 = SRVHIT_TSV
        .lines()
        .find(|l| l.starts_with("2\t"))
        .unwrap()
        .to_string();
    let demoted = SRVHIT_TSV.replacen(&row2, &row2.replace("\tspec'd-here", "\tsummarized"), 1);
    let mut spec: Vec<i16> = SRV_HIT_IMPLEMENTED.to_vec();
    spec.sort_unstable();
    let ours = format!("{spec:?}");
    spec.retain(|&i| i != 2);
    assert_eq!(
        check(&demoted, &SRV_HIT_IMPLEMENTED),
        Err(format!("spec'd-here {spec:?}, implemented {ours}"))
    );
    // A body without a spec'd-here row is reported.
    let mut extra = SRV_HIT_IMPLEMENTED.to_vec();
    extra.push(30);
    assert!(check(SRVHIT_TSV, &extra).is_err());
    // A null row (`addr_114d` `-`) is never counted, whatever its status.
    assert!(!spec_d_here(SRVHIT_TSV).contains(&30));
}
