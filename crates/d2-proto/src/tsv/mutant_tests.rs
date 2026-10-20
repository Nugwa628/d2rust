//! Perturbation tests (METHODS M08) of the strict TSV parser, written
//! against surviving mutants: each malformed cell is rejected, each
//! well-formed edge case accepted. Part of d2-proto's test suite only.

use super::*;

#[test]
fn numbers_are_plain_hex_or_decimal() {
    assert_eq!(number("0x1F"), Ok(0x1F));
    assert_eq!(number("31"), Ok(31));
    // `from_str_radix` alone would take a sign.
    for bad in ["0x+1", "0x", "0xG1", "+1", ""] {
        assert!(number(bad).is_err(), "{bad}");
    }
}

#[test]
fn addresses_have_eight_hex_digits() {
    assert_eq!(addr("0x005497E0"), Ok(Some(0x0054_97E0)));
    assert_eq!(addr("-"), Ok(None));
    for bad in ["0x5497E0", "0x0005497E0", "5497E0"] {
        assert!(addr(bad).is_err(), "{bad}");
    }
}

#[test]
fn message_names_start_upper_and_are_alphanumeric() {
    assert_eq!(message_name("Walk2"), Ok("Walk2"));
    assert_eq!(message_name("-"), Ok("-"));
    for bad in ["walk", "Walk-2", "Wa lk", "2Walk", ""] {
        assert!(message_name(bad).is_err(), "{bad}");
    }
}

#[test]
fn size_options_appear_once() {
    assert!(size_rule("u16@1;cap=509;min=3").is_ok());
    assert!(size_rule("u16@1;cap=509;cap=510").is_err());
    assert!(size_rule("u16@1;min=3;min=4").is_err());
}

#[test]
fn handler_size_ranges_may_be_one_value() {
    assert_eq!(handler_size("5..5"), Ok(HandlerSize::Range(5, 5)));
    assert_eq!(handler_size("4..275"), Ok(HandlerSize::Range(4, 275)));
    assert!(handler_size("6..5").is_err());
}

#[test]
fn bit_field_widths_have_no_leading_zero() {
    assert_eq!(field_type("bit0"), Ok(FieldType::Bit(0)));
    assert_eq!(field_type("bit31"), Ok(FieldType::Bit(31)));
    assert_eq!(field_type("u15"), Ok(FieldType::Bits(15)));
    for bad in ["u07", "bit07", "u", "bit", "bit32", "u32x", "u0"] {
        assert!(field_type(bad).is_err(), "{bad}");
    }
}

#[test]
fn two_fields_on_one_bit_overlap() {
    let fixed = SizeRule::Fixed(5);
    assert!(layout("a:bit7@1 b:bit8@1", &fixed).is_ok());
    let e = layout("a:bit7@1 b:bit7@1", &fixed).unwrap_err();
    assert!(e.contains("overlap"), "{e}");
    let e = layout("a:u16@1 b:bit15@1", &fixed).unwrap_err();
    assert!(e.contains("overlap"), "{e}");
    assert!(layout("a:u15@1 b:bit15@1", &fixed).is_ok());
}

#[test]
fn only_cstr_may_omit_its_offset() {
    assert!(layout("a:cstr@3 b:cstr", &SizeRule::Chat).is_ok());
    let e = layout("a:cstr@3 b:u8", &SizeRule::Chat).unwrap_err();
    assert!(e.contains("bad layout field"), "{e}");
}

#[test]
fn bits_layout_needs_fields() {
    let fixed = SizeRule::Fixed(4);
    assert!(layout("bits: id:8 a:12", &fixed).is_ok());
    for bad in ["bits: ", "bits:", "bits:id:8"] {
        assert!(layout(bad, &fixed).is_err(), "{bad:?}");
    }
}

#[test]
fn byte_arrays_have_a_plain_length_and_fit_the_size() {
    assert_eq!(field_type("bytes96"), Ok(FieldType::Bytes(96)));
    assert_eq!(field_type("bytes1"), Ok(FieldType::Bytes(1)));
    for bad in [
        "bytes",
        "bytes0",
        "bytes096",
        "bytes+1",
        "bytes9x",
        "bytes70000",
    ] {
        assert!(field_type(bad).is_err(), "{bad}");
    }
    let fixed = SizeRule::Fixed(42);
    assert!(layout("s:bytes41@1", &fixed).is_ok());
    let e = layout("s:bytes42@1", &fixed).unwrap_err();
    assert!(e.contains("past the fixed size"), "{e}");
    let e = layout("s:bytes4@1 a:u8@4", &fixed).unwrap_err();
    assert!(e.contains("overlap"), "{e}");
}
