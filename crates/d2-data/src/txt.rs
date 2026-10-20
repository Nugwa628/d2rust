//! Strict reader for the excel `.txt` tables.
//!
//! This module turns a `.txt` file's bytes into a header plus records of raw cells
//! (line splitting, header, record numbering, removed rows), binds header columns
//! to a field list, and reports failures as numbered [`ErrorCode`]s. Cell
//! conversion into typed values is in `compile`; `compile`, `sounds`, `patch` and
//! `bin` take their input from [`TxtTable`].
//!
//! Text stays bytes: the reader never decodes, trims, unquotes or
//! normalizes a cell.

use std::fmt;

/// Most header columns, field-list entries, and columns + missing fields.
pub const MAX_COLUMNS: usize = 280;

/// The numbered ways a `.txt` file or field list can be rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ErrorCode {
    /// No CR LF pair in the file.
    E1,
    /// Nothing after the header.
    E2,
    /// No records after removing `Expansion` lines.
    E3,
    /// A CR not followed by LF.
    E4,
    /// An LF not preceded by CR.
    E5,
    /// Bytes after the last CR LF.
    E6,
    /// More than 280 header columns.
    E7,
    /// A record's cell count differs from the header.
    E8,
    /// A NUL byte.
    E9,
    /// A byte-order mark.
    E10,
    /// A byte >= 0x80 in a name key.
    E11,
    /// A code registration index that is too large.
    E12,
    /// An invalid field list.
    E13,
    /// Columns plus missing fields exceed 280.
    E14,
    /// A callback value is missing, or a write passes the end of the record.
    E15,
}

impl ErrorCode {
    /// The rejection condition, in words.
    pub fn describe(self) -> &'static str {
        match self {
            ErrorCode::E1 => "no CR LF pair in the file",
            ErrorCode::E2 => "nothing after the header",
            ErrorCode::E3 => "no records after removing Expansion lines",
            ErrorCode::E4 => "CR not followed by LF",
            ErrorCode::E5 => "LF not preceded by CR",
            ErrorCode::E6 => "bytes after the last CR LF",
            ErrorCode::E7 => "more than 280 header columns",
            ErrorCode::E8 => "record cell count differs from the header",
            ErrorCode::E9 => "NUL byte",
            ErrorCode::E10 => "byte-order mark",
            ErrorCode::E11 => "byte >= 0x80 in a name key",
            ErrorCode::E12 => "code registration index too large",
            ErrorCode::E13 => "invalid field list",
            ErrorCode::E14 => "columns + missing fields exceed 280",
            ErrorCode::E15 => "callback value missing or write past the record",
        }
    }
}

/// A rejected file or field list. Lines are numbered from 1 (the header is
/// line 1), columns from 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxtError {
    /// The file (or field list) that was rejected.
    pub file: String,
    /// Which rule failed.
    pub code: ErrorCode,
    /// Line of the failure, if it has one.
    pub line: Option<usize>,
    /// Column of the failure, if it has one.
    pub column: Option<usize>,
    /// Field name involved, if any.
    pub field: Option<String>,
    /// Extra detail (may be empty).
    pub detail: String,
}

impl TxtError {
    /// An error with no position, field or detail.
    pub fn new(file: &str, code: ErrorCode) -> TxtError {
        TxtError {
            file: file.to_owned(),
            code,
            line: None,
            column: None,
            field: None,
            detail: String::new(),
        }
    }

    /// Sets the line number.
    pub fn at_line(mut self, line: usize) -> TxtError {
        self.line = Some(line);
        self
    }

    /// Sets the column number.
    pub fn at_column(mut self, column: usize) -> TxtError {
        self.column = Some(column);
        self
    }

    /// Sets the field name (lossily decoded).
    pub fn with_field(mut self, field: &[u8]) -> TxtError {
        self.field = Some(String::from_utf8_lossy(field).into_owned());
        self
    }

    /// Sets the extra detail text.
    pub fn with_detail(mut self, detail: impl Into<String>) -> TxtError {
        self.detail = detail.into();
        self
    }
}

impl fmt::Display for TxtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.file)?;
        if let Some(line) = self.line {
            write!(f, ", line {line}")?;
        }
        if let Some(column) = self.column {
            write!(f, ", column {column}")?;
        }
        if let Some(field) = &self.field {
            write!(f, ", field `{field}`")?;
        }
        write!(f, ": {:?} {}", self.code, self.code.describe())?;
        if !self.detail.is_empty() {
            write!(f, " ({})", self.detail)?;
        }
        Ok(())
    }
}

impl std::error::Error for TxtError {}

/// One record: its cells (exactly `C`) and its line number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxtRecord {
    /// Line number in the file (the header is line 1).
    pub line: usize,
    /// Raw cell bytes, one per header column.
    pub cells: Vec<Vec<u8>>,
}

/// A parsed `.txt` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxtTable {
    /// Column names, byte-exact.
    pub header: Vec<Vec<u8>>,
    /// Records in file order; record `i` is `records[i]`.
    pub records: Vec<TxtRecord>,
    /// Line numbers of removed `Expansion` lines.
    pub removed_lines: Vec<usize>,
}

/// A record whose first cell is this is removed.
const EXPANSION: &[u8] = b"Expansion";

impl TxtTable {
    /// Splits `data` into header and records under strict line, header and
    /// record rules, checking in a fixed order (byte-order mark first, then line
    /// endings, header, records). `file` names the file in errors.
    pub fn parse(file: &str, data: &[u8]) -> Result<TxtTable, TxtError> {
        let err = |code: ErrorCode| TxtError::new(file, code);

        // 1. E10: a byte-order mark.
        if data.starts_with(&[0xEF, 0xBB, 0xBF])
            || data.starts_with(&[0xFF, 0xFE])
            || data.starts_with(&[0xFE, 0xFF])
        {
            return Err(err(ErrorCode::E10).at_line(1));
        }
        // 2. E1: no CR LF pair.
        if !data.windows(2).any(|w| w == b"\r\n") {
            return Err(err(ErrorCode::E1));
        }
        // 3. One scan in byte order: NUL, lone CR, lone LF.
        let mut line = 1;
        let mut i = 0;
        while i < data.len() {
            match data[i] {
                0 => return Err(err(ErrorCode::E9).at_line(line)),
                b'\r' => {
                    if data.get(i + 1) == Some(&b'\n') {
                        line += 1;
                        i += 2;
                        continue;
                    }
                    return Err(err(ErrorCode::E4).at_line(line));
                }
                b'\n' => return Err(err(ErrorCode::E5).at_line(line)),
                _ => {}
            }
            i += 1;
        }
        // 4. E6: bytes after the last CR LF (`line` is that line's number).
        if !data.ends_with(b"\r\n") {
            return Err(err(ErrorCode::E6).at_line(line));
        }

        // Every CR is now followed by LF and every LF preceded by CR.
        let mut lines: Vec<&[u8]> = Vec::new();
        let mut start = 0;
        for (pos, &b) in data.iter().enumerate() {
            if b == b'\r' {
                lines.push(&data[start..pos]);
                start = pos + 2;
            }
        }

        // 5. E7: header columns.
        let header: Vec<Vec<u8>> = lines[0]
            .split(|&b| b == b'\t')
            .map(<[u8]>::to_vec)
            .collect();
        let columns = header.len();
        if columns > MAX_COLUMNS {
            return Err(err(ErrorCode::E7)
                .at_line(1)
                .with_detail(format!("{columns} columns")));
        }
        // 6. E2: no line after the header.
        if lines.len() == 1 {
            return Err(err(ErrorCode::E2));
        }
        // 7. Records, E8 at the first bad line.
        let mut records = Vec::with_capacity(lines.len() - 1);
        let mut removed_lines = Vec::new();
        for (k, l) in lines[1..].iter().enumerate() {
            let line = k + 2;
            let cells: Vec<Vec<u8>> = l.split(|&b| b == b'\t').map(<[u8]>::to_vec).collect();
            if cells[0] == EXPANSION {
                removed_lines.push(line);
                continue;
            }
            if cells.len() != columns {
                return Err(err(ErrorCode::E8)
                    .at_line(line)
                    .with_detail(format!("{} cells, header has {columns}", cells.len())));
            }
            records.push(TxtRecord { line, cells });
        }
        // 8. E3: no records.
        if records.is_empty() {
            return Err(err(ErrorCode::E3));
        }
        Ok(TxtTable {
            header,
            records,
            removed_lines,
        })
    }

    /// Header column count `C`.
    pub fn columns(&self) -> usize {
        self.header.len()
    }
}

/// Column-name equality: same length, same bytes after mapping
/// `A`–`Z` to `a`–`z`.
pub fn names_equal(a: &[u8], b: &[u8]) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Result of binding header columns to a field list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// For each header column, the field it binds.
    pub column_field: Vec<Option<usize>>,
    /// For each field, its column; `None` = missing.
    pub field_column: Vec<Option<usize>>,
    /// Columns whose name equals an earlier column's (diagnostic
    /// DupColumn).
    pub duplicate_columns: Vec<usize>,
}

impl Binding {
    /// Indices of missing fields, in field-list order.
    pub fn missing_fields(&self) -> impl Iterator<Item = usize> + '_ {
        self.field_column
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_none())
            .map(|(f, _)| f)
    }
}

/// Binds columns left to right: a column binds the not-yet-bound field
/// with an equal name; otherwise it is unbound.
pub fn bind<N: AsRef<[u8]>>(header: &[Vec<u8>], field_names: &[N]) -> Binding {
    let mut column_field = vec![None; header.len()];
    let mut field_column = vec![None; field_names.len()];
    let mut duplicate_columns = Vec::new();
    for (c, name) in header.iter().enumerate() {
        if header[..c].iter().any(|earlier| names_equal(earlier, name)) {
            duplicate_columns.push(c);
        }
        if let Some(f) = field_names
            .iter()
            .enumerate()
            .position(|(f, n)| field_column[f].is_none() && names_equal(n.as_ref(), name))
        {
            field_column[f] = Some(c);
            column_field[c] = Some(f);
        }
    }
    Binding {
        column_field,
        field_column,
        duplicate_columns,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(data: &[u8]) -> Result<TxtTable, TxtError> {
        TxtTable::parse("t.txt", data)
    }

    fn cells(t: &TxtTable, i: usize) -> Vec<&[u8]> {
        t.records[i].cells.iter().map(Vec::as_slice).collect()
    }

    fn fails(data: &[u8], code: ErrorCode, line: Option<usize>) {
        let e = parse(data).unwrap_err();
        assert_eq!((e.code, e.line), (code, line), "input {data:?}");
    }

    #[test]
    fn reader_vectors() {
        let t = parse(b"a\tb\r\n1\t2\r\n").unwrap();
        assert_eq!(t.header, [b"a".to_vec(), b"b".to_vec()]);
        assert_eq!(cells(&t, 0), [b"1", b"2"]);
        assert_eq!(t.records[0].line, 2);

        let t = parse(b"code\tv\r\nx\t1\r\nExpansion\t\r\ny\t2\r\n").unwrap();
        assert_eq!(cells(&t, 0), [b"x", b"1"]);
        assert_eq!(cells(&t, 1), [b"y", b"2"]);
        assert_eq!((t.records[0].line, t.records[1].line), (2, 4));
        assert_eq!(t.removed_lines, [3]);

        let t = parse(b"code\tv\r\nExpansion\t9\r\nx\t1\r\n").unwrap();
        assert_eq!(t.records.len(), 1);
        assert_eq!(cells(&t, 0), [b"x", b"1"]);

        let t = parse(b"a\tb\r\nExpansion\r\n1\t2\r\n").unwrap();
        assert_eq!(cells(&t, 0), [b"1", b"2"]);
        assert_eq!(t.removed_lines, [2]);

        let t = parse(b"Expansion\r\n1\r\n").unwrap();
        assert_eq!(t.header, [b"Expansion".to_vec()]);
        assert_eq!(cells(&t, 0), [b"1"]);

        let t = parse(b"c\r\nEXPANSION\r\nexpansion\r\nExpansion \r\nExpansion\r\n").unwrap();
        assert_eq!(t.records.len(), 3);
        assert_eq!(cells(&t, 0), [b"EXPANSION"]);
        assert_eq!(cells(&t, 1), [b"expansion"]);
        assert_eq!(cells(&t, 2), [b"Expansion "]);
        assert_eq!(t.removed_lines, [5]);

        let t = parse(b"a\tb\r\n\t\r\n").unwrap();
        assert_eq!(cells(&t, 0), [b"", b""]);
        let t = parse(b"a\r\n\r\n").unwrap();
        assert_eq!(cells(&t, 0), [b""]);
        let t = parse(b"n\r\n\"a,b\"\r\n  x \r\n").unwrap();
        assert_eq!(cells(&t, 0), [b"\"a,b\""]);
        assert_eq!(cells(&t, 1), [b"  x "]);
        let t = parse(b"n\r\n\x85\x92\r\n").unwrap();
        assert_eq!(cells(&t, 0), [b"\x85\x92"]);
    }

    #[test]
    fn reader_errors() {
        fails(b"a\tb\r\n\r\n", ErrorCode::E8, Some(2));
        fails(b"a\tb\r\n1\t2\t\r\n", ErrorCode::E8, Some(2));
        fails(b"a\r\n1\r\n2", ErrorCode::E6, Some(3));
        fails(b"a\r\nb", ErrorCode::E6, Some(2));
        fails(b"a\r\n1\r2\r\n", ErrorCode::E4, Some(2));
        fails(b"a\r\n1\r", ErrorCode::E4, Some(2));
        fails(b"a\r\n1\n2\r\n", ErrorCode::E5, Some(2));
        fails(b"a\r\n1\n", ErrorCode::E5, Some(2));
        fails(b"a\r\nx\x00y\r\n", ErrorCode::E9, Some(2));
        fails(b"a\r\n1\r\n\x00", ErrorCode::E9, Some(3));
        fails(b"a\n1\n", ErrorCode::E1, None);
        fails(b"", ErrorCode::E1, None);
        fails(b"a\r", ErrorCode::E1, None);
        fails(b"a\r\n", ErrorCode::E2, None);
        fails(b"a\r\nExpansion\r\n", ErrorCode::E3, None);
        fails(b"\xEF\xBB\xBFa\r\n1\r\n", ErrorCode::E10, Some(1));
        fails(b"\xFF\xFEa\x00\r\x00\n\x00", ErrorCode::E10, Some(1));
        // Fixed example inputs.
        fails(b"a\tb\r\n1\t2\r\nExpansion\r\n3\t4", ErrorCode::E6, Some(4));
        fails(b"a\tb\r1\t2\r\n", ErrorCode::E4, Some(1));
    }

    #[test]
    fn column_limit() {
        let mut ok = vec![b'c'; 1];
        for _ in 1..MAX_COLUMNS {
            ok.extend_from_slice(b"\tc");
        }
        let mut data = ok.clone();
        data.extend_from_slice(b"\r\n");
        data.extend(std::iter::repeat_n(b'\t', MAX_COLUMNS - 1));
        data.extend_from_slice(b"\r\n");
        assert_eq!(parse(&data).unwrap().columns(), 280);

        let mut data = ok;
        data.extend_from_slice(b"\tc\r\n");
        data.extend(std::iter::repeat_n(b'\t', MAX_COLUMNS));
        data.extend_from_slice(b"\r\n");
        fails(&data, ErrorCode::E7, Some(1));
    }

    fn header(names: &[&[u8]]) -> Vec<Vec<u8>> {
        names.iter().map(|n| n.to_vec()).collect()
    }

    #[test]
    fn binding_vectors() {
        let b = bind(&header(&[b"Name", b"LEVEL", b"name"]), &["name", "level"]);
        assert_eq!(b.field_column, [Some(0), Some(1)]);
        assert_eq!(b.column_field, [Some(0), Some(1), None]);
        assert_eq!(b.duplicate_columns, [2]);

        let b = bind(&header(&[b" name", b"name "]), &["name"]);
        assert_eq!(b.field_column, [None]);
        assert_eq!(b.missing_fields().collect::<Vec<_>>(), [0]);

        let b = bind(&header(&[b"mindam", b"maxdam", b"mindam"]), &["mindam"]);
        assert_eq!(b.field_column, [Some(0)]);
        assert_eq!(b.duplicate_columns, [2]);

        let b = bind(&header(&[b"", b"a"]), &["a"]);
        assert_eq!(b.column_field, [None, Some(0)]);
    }

    #[test]
    fn lines_after_header_are_data() {
        let t = parse(b"a\tb\r\nx\ty\r\na\tb\r\n").unwrap();
        assert_eq!(t.header, [b"a".to_vec(), b"b".to_vec()]);
        assert_eq!(t.records.len(), 2);
        assert_eq!(cells(&t, 0), [b"x", b"y"]);
        // A line equal to the header is data too.
        assert_eq!(cells(&t, 1), [b"a", b"b"]);
        assert_eq!((t.records[0].line, t.records[1].line), (2, 3));
    }

    #[test]
    fn name_equality() {
        assert!(names_equal(b"MinDam", b"mindam"));
        assert!(names_equal(b"A-Z_09", b"a-z_09"));
        assert!(names_equal(b"", b""));
        // Same length required; no trimming.
        assert!(!names_equal(b"name", b"name "));
        assert!(!names_equal(b" name", b"name"));
        assert!(!names_equal(b"nam", b"name"));
        // ASCII only: other bytes compare as they are.
        assert!(!names_equal(b"caf\xC9", b"caf\xE9"));
        assert!(names_equal(b"caf\xE9", b"CAF\xE9"));
        assert!(!names_equal(b"[", b"{"));
        assert!(!names_equal(b"@", b"`"));
        let b = bind(&header(&[b"CAF\xC9", b"Caf\xE9"]), &[&b"caf\xE9"[..]]);
        assert_eq!(b.field_column, [Some(1)]);
    }

    #[test]
    fn unterminated_expansion_and_trailing_cr_are_rejected() {
        // A final unterminated `Expansion` line: E6, no record is lost.
        fails(b"a\tb\r\n1\t2\r\nExpansion", ErrorCode::E6, Some(3));
        fails(b"a\tb\r\n1\t2\r\nExpansion\t", ErrorCode::E6, Some(3));
        // A CR as the last byte: E4.
        fails(b"a\r\n1\r\n2\r", ErrorCode::E4, Some(3));
    }
}
