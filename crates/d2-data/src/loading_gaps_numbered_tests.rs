//! Edge-case test for the `.txt` reader: a file whose last line has no line
//! terminator is rejected rather than read with a dropped record. Synthetic bytes
//! only.

use crate::txt::{ErrorCode, TxtTable};

fn fails(data: &[u8]) -> (ErrorCode, Option<usize>) {
    let e = TxtTable::parse("data\\global\\excel\\t.txt", data).unwrap_err();
    (e.code, e.line)
}

/// A file with an unterminated last line is rejected, so no record is silently
/// dropped. In the first case the `Expansion` row is not the last line, the
/// unterminated `3\t4` is, and the file fails with E6 at line 4 instead of
/// yielding one record.
#[test]
fn unterminated_last_lines_are_rejected() {
    assert_eq!(
        fails(b"a\tb\r\n1\t2\r\nExpansion\r\n3\t4"),
        (ErrorCode::E6, Some(4))
    );
    // A final unterminated `Expansion` line is rejected too, so the last real
    // record is not lost.
    assert_eq!(
        fails(b"a\tb\r\n1\t2\r\n3\t4\r\nExpansion"),
        (ErrorCode::E6, Some(4))
    );
    // Terminated, the same rows load: `Expansion` removed, both records kept.
    let t = TxtTable::parse("t.txt", b"a\tb\r\n1\t2\r\n3\t4\r\nExpansion\r\n").unwrap();
    assert_eq!(t.records.len(), 2);
    assert_eq!(t.records[1].cells, [b"3".to_vec(), b"4".to_vec()]);
}
