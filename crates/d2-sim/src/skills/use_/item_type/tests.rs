use super::*;

/// An item: its types (equivalence already expanded), flags, `shoots`,
/// stats 157 / 70 and whether it stacks.
#[derive(Clone, Default)]
struct It {
    types: Vec<i32>,
    ammo_free: bool,
    shoots: i32,
    magicarrow: i32,
    quantity: i32,
    stackable: bool,
}

#[derive(Default)]
struct Hands {
    a: Option<It>,
    b: Option<It>,
    no_inventory: bool,
}

impl ItemTypeView for Hands {
    type Item = u8;
    fn hand_item(&self, loc: u8) -> Option<u8> {
        match loc {
            4 => self.a.as_ref().map(|_| 4),
            5 => self.b.as_ref().map(|_| 5),
            _ => None,
        }
    }
    fn has_inventory(&self) -> bool {
        !self.no_inventory
    }
    fn weapon_in_use(&self) -> Option<u8> {
        self.hand_item(4)
    }
    fn is_a(&self, i: u8, t: i32) -> bool {
        self.it(i).types.contains(&t)
    }
    fn ammo_free(&self, i: u8) -> bool {
        self.it(i).ammo_free
    }
    fn shoots(&self, i: u8) -> i32 {
        self.it(i).shoots
    }
    fn stat(&self, i: u8, stat: u16) -> i32 {
        match stat {
            157 => self.it(i).magicarrow,
            70 => self.it(i).quantity,
            _ => 0,
        }
    }
    fn stackable(&self, i: u8) -> bool {
        self.it(i).stackable
    }
}

impl Hands {
    fn it(&self, i: u8) -> &It {
        if i == 4 {
            self.a.as_ref().unwrap()
        } else {
            self.b.as_ref().unwrap()
        }
    }
}

const BOW: i32 = 27;
const BOWQ: i32 = 5;
const MISS: i32 = 74;

fn bow() -> It {
    It {
        types: vec![BOW, MISS, WEAP],
        shoots: BOWQ,
        ..It::default()
    }
}
fn quiver(q: i32) -> It {
    It {
        types: vec![BOWQ],
        quantity: q,
        stackable: true,
        ..It::default()
    }
}
fn skill(skill: i32, a1: i32, noammo: bool) -> ItemTypeSkill {
    ItemTypeSkill {
        skill,
        a: HandSet {
            itypes: [a1, 0, 0],
            etypes: [0, 0],
        },
        noammo,
        ..ItemTypeSkill::default()
    }
}

// Attack (`itypea1 weap`, not noammo) with a bow and no quiver fails the
// shoots rule, both orders (measured Amazon join: use state 2);
// Magic Arrow (`itypea1 miss`, noammo) passes; Cold Arrow (miss, ammo)
// fails.
#[test]
fn bow_without_quiver() {
    let h = Hands {
        a: Some(bow()),
        ..Hands::default()
    };
    assert!(!item_type_test(&h, &skill(0, WEAP, false)));
    assert!(item_type_test(&h, &skill(6, MISS, true)));
    assert!(!item_type_test(&h, &skill(11, MISS, false)));
}

// With a quiver the ammo skills pass; an empty quiver fails them.
#[test]
fn bow_with_quiver() {
    let h = Hands {
        a: Some(bow()),
        b: Some(quiver(100)),
        ..Hands::default()
    };
    assert!(item_type_test(&h, &skill(0, WEAP, false)));
    assert!(item_type_test(&h, &skill(11, MISS, false)));
    let h = Hands {
        a: Some(bow()),
        b: Some(quiver(0)),
        ..Hands::default()
    };
    assert!(!item_type_test(&h, &skill(11, MISS, false)));
}

// Attack's exceptions: stat 157 on the launcher or the ammo, and an
// item flagged 0x4000 / 0x100 passes for Attack only.
#[test]
fn attack_exceptions() {
    let h = Hands {
        a: Some(It {
            magicarrow: 1,
            ..bow()
        }),
        ..Hands::default()
    };
    assert!(item_type_test(&h, &skill(0, WEAP, false)));
    assert!(!item_type_test(&h, &skill(11, MISS, false)));
    let h = Hands {
        a: Some(It {
            ammo_free: true,
            ..bow()
        }),
        ..Hands::default()
    };
    assert!(item_type_test(&h, &skill(0, WEAP, false)));
    assert!(!item_type_test(&h, &skill(6, MISS, true)));
}

// Step 1: no set a → pass without an inventory; step 2: a set and no
// inventory → fail; bare hands meet a weapon set.
#[test]
fn rules_1_2_and_bare_hands() {
    let none = Hands {
        no_inventory: true,
        ..Hands::default()
    };
    assert!(item_type_test(&none, &skill(0, 0, false)));
    assert!(!item_type_test(&none, &skill(0, WEAP, false)));
    assert!(item_type_test(&Hands::default(), &skill(0, WEAP, false)));
    assert!(!item_type_test(&Hands::default(), &skill(6, MISS, true)));
}

// A set that lists no type passes a present item at once: a bow with no
// ammo in the hand of an empty set b is not tested for ammo (the body of
// returns before).
#[test]
fn an_unlisted_set_skips_the_ammo_rule() {
    let h = Hands {
        a: Some(It {
            types: vec![WEAP],
            ..It::default()
        }),
        b: Some(bow()),
        ..Hands::default()
    };
    assert!(item_type_test(&h, &skill(1, WEAP, false)));
}
