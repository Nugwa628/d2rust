//! Tests of the network item reader ([`super`]) on ten real 0x9C / 0x9D
//! messages (vectors B1 to B10: compact items, a weapon, a shield, a
//! superior socketed item, magic items, a stackable and a parametrized stat),
//! plus perturbations of one stream. [`Fixture`] is shared with `save_tests`.
use super::*;

/// The table facts the vectors need: item codes and the `itemstatcost` save
/// columns of the stats they carry.
pub(crate) struct Fixture;

impl ItemLookup for Fixture {
    fn code(&self, code: [u8; 4]) -> Option<CodeFacts> {
        let weapon = CodeFacts {
            weapon: true,
            ..CodeFacts::default()
        };
        let armor = CodeFacts {
            armor: true,
            ..CodeFacts::default()
        };
        Some(match &code {
            b"hp1 " | b"isc " => CodeFacts::default(),
            b"hax " | b"lax " | b"scm " | b"sbw " | b"sst " => weapon,
            b"tkf " => CodeFacts {
                stackable: true,
                ..weapon
            },
            b"buc " | b"cap " => armor,
            _ => return None,
        })
    }

    fn isc(&self, stat: u16) -> Option<IscSave> {
        let (bits, add, param) = match stat {
            9 => (8, 32, 0),
            17 | 18 => (9, 0, 0),
            19 => (10, 0, 0),
            22 => (7, 0, 0),
            31 => (11, 10, 0),
            48 => (8, 0, 0),
            49 => (9, 0, 0),
            60 => (7, 0, 0),
            72 => (9, 0, 0),
            73 => (8, 0, 0),
            75 => (7, 20, 0),
            107 => (3, 0, 9),
            194 => (4, 0, 0),
            356 => (2, 0, 0),
            _ => return None,
        };
        Some(IscSave {
            save_bits: bits,
            save_add: add,
            save_param_bits: param,
        })
    }
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn dec(msg: &str) -> ItemBits {
    let b = hex(msg);
    let from = if b[0] == 0x9C { 8 } else { 13 };
    decode(&b[from..], &Fixture).unwrap()
}

fn vals(l: &[Stat]) -> Vec<(u16, u32, i64)> {
    l.iter().map(|s| (s.stat, s.param, s.value())).collect()
}

fn slot(body: u8, x: u8, y: u8, page1: u8) -> Option<Location> {
    Some(Location::Slot { body, x, y, page1 })
}

// Compact records (B1, B2): flags, version, location, code, bit count.
#[test]
fn b1_b2_compact() {
    let b1 = dec("9c0e1410010000001000a2006508008006170302");
    assert_eq!(b1.flags, 0xA20010);
    assert_eq!((b1.version, b1.mode), (101, 2));
    assert_eq!(b1.location, slot(0, 0, 0, 0));
    assert_eq!(&b1.code, b"hp1 ");
    assert_eq!(b1.bits, 92);
    let b2 = dec("9c041410060000001000a2006500529236370602");
    assert_eq!(b2.mode, 0);
    assert_eq!(b2.location, slot(0, 9, 2, 1));
    assert_eq!(&b2.code, b"isc ");
    assert_eq!(b2.bits, 92);
}

// Normal-quality weapon (B3) and shield (B4): durability, defense, empty list.
#[test]
fn b3_b4_normal_weapon_and_shield() {
    let b3 = dec("9d061e0508000000000100000011008200658408801686078280c0c1e13f");
    assert_eq!(b3.flags, 0x820011);
    assert_eq!(b3.location, slot(4, 4, 0, 0));
    assert_eq!(&b3.code, b"hax ");
    assert_eq!((b3.filled, b3.ilvl, b3.quality), (0, 1, 2));
    assert_eq!((b3.gfx, b3.auto_affix), (None, None));
    assert_eq!(b3.max_durability.unwrap().value(), 28);
    assert_eq!(b3.durability.unwrap().value(), 28);
    assert_eq!(b3.lists, vec![Some(Vec::new())]);
    assert_eq!(b3.bits, 134);
    let b4 = dec("9d0620060900000000010000001100820065a40a205637068280f0000606ff01");
    assert_eq!(&b4.code, b"buc ");
    assert_eq!(b4.location, slot(5, 5, 0, 0));
    let d = b4.defense.unwrap();
    assert_eq!((d.raw, d.value()), (15, 5));
    assert_eq!(b4.max_durability.unwrap().value(), 12);
    assert_eq!(b4.durability.unwrap().value(), 12);
    assert_eq!(b4.bits, 145);
}

// Superior socketed item (B5): file index, socket count, stat list.
#[test]
fn b5_superior_socketed() {
    let b5 = dec("9c0b1e050c000000102880006500c0c416860702c3500f1133218025a2ff");
    assert_eq!(b5.flags, 0x802810);
    assert_eq!(b5.location, slot(0, 0, 6, 2));
    assert_eq!((b5.ilvl, b5.quality), (6, 3));
    assert_eq!(b5.quality_fields.file_index, Some(5));
    assert_eq!(b5.max_durability.unwrap().value(), 30);
    assert_eq!(b5.durability.unwrap().value(), 34);
    assert_eq!(b5.sockets, Some(3));
    assert_eq!(
        vals(b5.lists[0].as_ref().unwrap()),
        vec![(19, 0, 1), (75, 0, 14)]
    );
    assert_eq!(b5.bits, 176);
}

// Magic items (B6, B8, B9): affix ids and grouped / offset stats.
#[test]
fn b6_b8_b9_magic() {
    let b6 = dec("9c0b210511000000102080006500063437d6060203a18b5b5858b010801140e03f");
    assert_eq!(&b6.code, b"scm ");
    assert_eq!(b6.quality_fields.magic, Some((186, 183)));
    assert_eq!(
        vals(b6.lists[0].as_ref().unwrap()),
        vec![(22, 0, 1), (48, 0, 1), (49, 0, 4)]
    );
    assert_eq!(b6.bits, 198);
    let b8 = dec("9c0b21061d00000010208000650070342776070203b10bb0504c88d0a1030fc27f");
    assert_eq!(b8.quality_fields.magic, Some((187, 352)));
    assert_eq!(b8.durability.unwrap().value(), 19);
    assert_eq!(
        vals(b8.lists[0].as_ref().unwrap()),
        vec![(17, 0, 29), (18, 0, 29), (60, 0, 4)]
    );
    assert_eq!(b8.bits, 199);
    let b9 = dec("9c0b1f002300000010208000650040321606070203011300348081418292ff");
    assert_eq!(b9.quality_fields.magic, Some((304, 0)));
    assert_eq!(b9.defense.unwrap().value(), 3);
    let l = b9.lists[0].as_ref().unwrap();
    assert_eq!((l[0].stat, l[0].raw, l[0].value()), (9, 37, 5));
    assert_eq!(b9.bits, 184);
}

// Stack quantity (B7) and a stat with a parameter (B10).
#[test]
fn b7_b10_quantity_and_param() {
    let b7 = dec("9c0b1a05140000001020800065000048b766060283404000d47f");
    assert_eq!(&b7.code, b"tkf ");
    assert_eq!(b7.quantity, Some(160));
    assert_eq!(b7.max_durability.unwrap().value(), 4);
    assert_eq!(b7.bits, 143);
    let b10 = dec("9d062105060000000001000000110082006584083037470782804041610d89fc07");
    assert_eq!(&b10.code, b"sst ");
    assert_eq!(vals(b10.lists[0].as_ref().unwrap()), vec![(107, 36, 1)]);
    assert_eq!(b10.bits, 155);
}

/// The reader's own checks fail on a changed stream (mutation guard).
#[test]
fn perturbed_streams_are_refused() {
    let b = hex("9c0b1a05140000001020800065000048b766060283404000d47f");
    let s = &b[8..];
    // One byte more: more than the padding.
    let mut longer = s.to_vec();
    longer.push(0);
    assert_eq!(decode(&longer, &Fixture), Err(ItemBitsError::Trailing(9)));
    // One byte less: the record does not fit.
    assert!(matches!(
        decode(&s[..s.len() - 1], &Fixture),
        Err(ItemBitsError::Short(_))
    ));
    // A padding bit set (143 bits: bit 143 is padding).
    let mut pad = s.to_vec();
    *pad.last_mut().unwrap() |= 0x80;
    assert_eq!(decode(&pad, &Fixture), Err(ItemBitsError::Padding(143)));
    // An unknown code (bit 64 is bit 4 of the code's first character:
    // `tkf ` → `dkf `).
    let mut code = s.to_vec();
    code[8] ^= 0x01;
    assert_eq!(
        decode(&code, &Fixture),
        Err(ItemBitsError::UnknownCode(*b"dkf "))
    );
}
