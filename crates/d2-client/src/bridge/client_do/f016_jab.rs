//! cltdofunc 16 (Jab): one function of the client skill do table
//! (`super::client_do`); the shared helpers live in `super`.

use super::*;

/// `cltdofunc` 16 (Jab, skill 10): only
/// when U's action frame event (+0x4E) is 3, a player U requests
/// `dosound a` and a monster U `dosound b`, each when > 0, on U with
/// delay 0 (the audio model's `jab_do`). An invalid skill
/// row gives 0 there; else 1. No missile, no flag 0x40.
pub(super) fn jab(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, skill: u16) -> i32 {
    let Some(u) = w.units.get(&key) else {
        return 1;
    };
    if u.action_frame != 3 {
        return 1;
    }
    let Some(row) = inputs.tables.skills.get(usize::from(skill)) else {
        return 0;
    };
    let id = match key.unit_type {
        PLAYER => row.dosound_a,
        MONSTER => row.dosound_b,
        _ => 0,
    };
    if id > 0 {
        w.objclient.missile_sounds.push(MissileSound::Request {
            id: i32::from(id),
            missile: key,
        });
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::client_do::test_util::*;

    fn requests(w: &ClientWorld) -> Vec<(i32, UnitKey)> {
        w.objclient
            .missile_sounds
            .iter()
            .filter_map(|s| match s {
                MissileSound::Request { id, missile } => Some((*id, *missile)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_jab_do_requests_dosound_a_on_its_action_frame() {
        let (mut w, key) = world();
        let mut i = inputs(16);
        let row = &mut i.tables.skills[usize::from(FROST_NOVA)];
        (row.dosound_a, row.dosound_b) = (301, 302);
        // Not the action frame: 1, no request.
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 1);
        assert!(requests(&w).is_empty());
        // Action frame 3 on a player: `dosound a` on U.
        w.units.get_mut(&key).unwrap().action_frame = 3;
        assert_eq!(client_do(&mut w, &i, key, FROST_NOVA, 1).unwrap(), 1);
        assert_eq!(requests(&w), vec![(301, key)]);
        assert!(!w.units[&key].flag_40);
        // A monster requests `dosound b`; none (≤ 0) requests nothing.
        w.objclient.missile_sounds.clear();
        let m = UnitKey::new(MONSTER, 7);
        let mut c = ClientUnit::new(m);
        c.action_frame = 3;
        w.units.insert(m, c);
        assert_eq!(jab(&mut w, &i, m, FROST_NOVA), 1);
        assert_eq!(requests(&w), vec![(302, m)]);
        w.objclient.missile_sounds.clear();
        i.tables.skills[usize::from(FROST_NOVA)].dosound_b = -1;
        assert_eq!(jab(&mut w, &i, m, FROST_NOVA), 1);
        assert!(requests(&w).is_empty());
        // An invalid row on the action frame: 0.
        assert_eq!(jab(&mut w, &i, m, 999), 0);
    }
}
