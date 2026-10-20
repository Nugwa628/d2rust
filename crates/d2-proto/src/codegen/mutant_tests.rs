//! Test written against a surviving mutant (METHODS M08): the generator's
//! rule for typed structs (module doc) on a row the TSVs do not have.
//! Part of the `d2-proto` generator tests; see [`super`].

use super::*;

/// Only fixed fields get a typed struct: a `cstr` at an offset in a
/// fixed-size row does not.
#[test]
fn a_cstr_field_makes_a_row_untyped() {
    let at = |ty| Field {
        name: "a",
        ty,
        offset: Some(1),
    };
    assert_eq!(
        typed("X", &SizeRule::Fixed(5), &[at(FieldType::U32)]),
        Some((5, vec![at(FieldType::U32)]))
    );
    assert_eq!(
        typed("X", &SizeRule::Fixed(5), &[at(FieldType::Cstr)]),
        None
    );
    assert_eq!(
        typed("X", &SizeRule::Fixed(5), &[at(FieldType::Tail)]),
        None
    );
}

/// `bits:` layouts (module doc): the `id:8` field at bit 0 is left out
/// of the struct; any other field over bits 0..8 makes the row untyped.
#[test]
fn packed_id_is_dropped_and_other_low_bits_untype() {
    let packed = |name, bit, width| Field {
        name,
        ty: FieldType::Packed { bit, width },
        offset: None,
    };
    let size = SizeRule::Fixed(4);
    assert_eq!(
        typed("X", &size, &[packed("id", 0, 8), packed("a", 8, 12)]),
        Some((4, vec![packed("a", 8, 12)]))
    );
    assert_eq!(typed("X", &size, &[packed("a", 0, 8)]), None);
    assert_eq!(typed("X", &size, &[packed("id", 0, 7)]), None);
    assert_eq!(typed("X", &size, &[packed("id", 4, 8)]), None);
}
