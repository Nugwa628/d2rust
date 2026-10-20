//! Typed records of the 73 runtime tables, decoded from record bytes with
//! explicit little-endian reads at the `fields.tsv` offsets.
//!
//! The structs are generated ([`crate::codegen`]) into `generated.rs`; this module
//! holds the decode helpers and the [`Record`] trait. The simulation and server
//! take their table data from these records, which are decoded from a
//! [`crate::bin::BinTable`].
//!
//! Integer, link and key fields keep their raw stored values (a missed
//! link stays all-ones); resolving and validating links is separate.

use crate::bin::BinTable;

#[rustfmt::skip]
mod generated;
pub use generated::*;

/// A typed record of one table.
pub trait Record: Sized {
    /// `tables.tsv` name.
    const TABLE: &'static str;
    /// Record size in bytes.
    const SIZE: usize;
    /// Decodes one record. `r` is exactly [`Record::SIZE`] bytes.
    fn decode(r: &[u8]) -> Self;
}

/// Decoding a table of the wrong name or record size.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{got} ({got_size}-byte records) is not {want} ({want_size}-byte records)")]
pub struct WrongTable {
    /// Name of the table that was given.
    pub got: String,
    /// Record size of the table that was given.
    pub got_size: usize,
    /// Name of the table the record type decodes.
    pub want: &'static str,
    /// Record size the record type expects.
    pub want_size: usize,
}

/// Decodes every record of `t` as `T`.
pub fn decode_all<T: Record>(t: &BinTable) -> Result<Vec<T>, WrongTable> {
    if t.name != T::TABLE || t.record_size != T::SIZE {
        return Err(WrongTable {
            got: t.name.clone(),
            got_size: t.record_size,
            want: T::TABLE,
            want_size: T::SIZE,
        });
    }
    Ok(t.iter().map(T::decode).collect())
}

fn u16_at(r: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([r[o], r[o + 1]])
}

fn u32_at(r: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([r[o], r[o + 1], r[o + 2], r[o + 3]])
}

fn bytes<const N: usize>(r: &[u8], o: usize) -> [u8; N] {
    r[o..o + N].try_into().expect("N bytes")
}

fn bit(r: &[u8], byte: usize, mask: u8) -> bool {
    r[byte] & mask != 0
}

/// The text of a string field: its bytes up to the first NUL.
pub fn text(field: &[u8]) -> &[u8] {
    let n = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    &field[..n]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::schema;

    #[test]
    fn decodes_at_schema_offsets() {
        let mut r = vec![0u8; Skills::SIZE];
        r[190] = 7; // pettype, link8
        r[0..2].copy_from_slice(&0x1234u16.to_le_bytes()); // skill id
        let s = Skills::decode(&r);
        assert_eq!(s.pettype, 7);
        let mut r = vec![0u8; Uniqueitems::SIZE];
        r[2..6].copy_from_slice(b"Abc\0");
        r[40..44].copy_from_slice(b"rin ");
        r[44] = 0b1001; // enabled (bit 0), ladder (bit 3)
        let u = Uniqueitems::decode(&r);
        assert_eq!(text(&u.index), b"Abc");
        assert_eq!(&u.code, b"rin ");
        assert!(u.enabled && u.ladder && !u.nolimit);
    }

    #[test]
    fn decode_all_checks_the_table() {
        let t = BinTable {
            name: "belts".into(),
            source: "t".into(),
            count: 2,
            record_size: Belts::SIZE,
            records: vec![0; 2 * Belts::SIZE],
        };
        assert_eq!(decode_all::<Belts>(&t).unwrap().len(), 2);
        assert!(decode_all::<Inventory>(&t).is_err());
    }

    #[test]
    fn one_struct_per_runtime_table() {
        let names: Vec<&str> = schema().runtime().map(|t| t.name.as_str()).collect();
        assert_eq!(names.len(), 73);
        assert_eq!(TABLES, names.as_slice());
    }
}
