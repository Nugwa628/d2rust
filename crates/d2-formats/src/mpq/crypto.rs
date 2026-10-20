//! Name hash, crypt table and block cipher of the MPQ format.
//!
//! The 0x500-dword crypt table is built at compile time. [`hash`] is the string hash used
//! for hash-table lookup and file keys; [`decrypt`] and [`encrypt`] are the dword stream
//! cipher for the tables and encrypted files. [`super::Archive`] uses these to read
//! archives; the test-only writer uses them to build them.

/// The 0x500-dword crypt table, generated at compile time.
pub(crate) const CRYPT_TABLE: [u32; 0x500] = build_crypt_table();

const fn build_crypt_table() -> [u32; 0x500] {
    let mut table = [0u32; 0x500];
    let mut seed: u32 = 0x0010_0001;
    let mut i = 0;
    while i < 0x100 {
        let mut t = 0;
        while t < 5 {
            // seed < 0x2AAAAB, so seed * 125 + 3 fits in a u32.
            seed = (seed * 125 + 3) % 0x2A_AAAB;
            let hi = seed & 0xFFFF;
            seed = (seed * 125 + 3) % 0x2A_AAAB;
            let lo = seed & 0xFFFF;
            table[t * 0x100 + i] = (hi << 16) | lo;
            t += 1;
        }
        i += 1;
    }
    table
}

/// Which of the four hash functions to apply (the crypt-table section used).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashType {
    /// Starting index into the hash table.
    TableOffset = 0,
    /// First name check value.
    NameA = 1,
    /// Second name check value.
    NameB = 2,
    /// Encryption key.
    FileKey = 3,
}

/// Uppercases ASCII letters and turns `/` into `\`; other bytes are unchanged.
fn normalize(b: u8) -> u8 {
    match b {
        b'a'..=b'z' => b - (b'a' - b'A'),
        b'/' => b'\\',
        _ => b,
    }
}

/// The MPQ string hash of `name`, of the given `kind`. Names are compared without
/// regard to ASCII case or path separator, so `data/Global` and `DATA\GLOBAL` hash
/// alike.
pub fn hash(name: &[u8], kind: HashType) -> u32 {
    let base = kind as usize * 0x100;
    let mut seed1: u32 = 0x7FED_7FED;
    let mut seed2: u32 = 0xEEEE_EEEE;
    for &b in name {
        let b = normalize(b);
        seed1 = CRYPT_TABLE[base + b as usize] ^ seed1.wrapping_add(seed2);
        seed2 = u32::from(b)
            .wrapping_add(seed1)
            .wrapping_add(seed2)
            .wrapping_add(seed2 << 5)
            .wrapping_add(3);
    }
    seed1
}

/// Key of the encrypted hash table: `hash("(hash table)", FileKey)`.
pub const HASH_TABLE_KEY: u32 = 0xC3AF_3770;
/// Key of the encrypted block table: `hash("(block table)", FileKey)`.
pub const BLOCK_TABLE_KEY: u32 = 0xEC83_B3A3;

/// The key for the next dword: the cipher rotates and mixes the key after every dword
/// so that equal plaintext dwords encrypt differently.
fn next_key(key: u32) -> u32 {
    ((!key << 21).wrapping_add(0x1111_1111)) | (key >> 11)
}

/// Which way [`apply_cipher`] runs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Decrypt,
    Encrypt,
}

/// Runs the dword stream cipher over `data` in place. The keystream depends on the
/// plaintext seen so far (the running `seed` folds in each plain dword), so one loop
/// serves both directions: only which side of the XOR is the plaintext differs. Trailing
/// bytes past the last whole dword are left unchanged.
fn apply_cipher(data: &mut [u8], mut key: u32, direction: Direction) {
    let mut seed: u32 = 0xEEEE_EEEE;
    for chunk in data.as_chunks_mut::<4>().0 {
        seed = seed.wrapping_add(CRYPT_TABLE[0x400 + (key & 0xFF) as usize]);
        let input = u32::from_le_bytes(*chunk);
        let output = input ^ key.wrapping_add(seed);
        let plain = match direction {
            Direction::Decrypt => output,
            Direction::Encrypt => input,
        };
        key = next_key(key);
        seed = plain
            .wrapping_add(seed)
            .wrapping_add(seed << 5)
            .wrapping_add(3);
        *chunk = output.to_le_bytes();
    }
}

/// Decrypts `data` in place. Trailing bytes past the last whole dword are
/// left unchanged.
pub fn decrypt(data: &mut [u8], key: u32) {
    apply_cipher(data, key, Direction::Decrypt);
}

/// Encrypts `data` in place (inverse of [`decrypt`]). Used by tests, the
/// test-support writer and `data-tool variant` (tables of a rewritten
/// archive).
pub fn encrypt(data: &mut [u8], key: u32) {
    apply_cipher(data, key, Direction::Encrypt);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crypt_table_spot_values() {
        assert_eq!(CRYPT_TABLE[0x000], 0x55C6_36E2);
        assert_eq!(CRYPT_TABLE[0x001], 0x02BE_0170);
        assert_eq!(CRYPT_TABLE[0x100], 0x76F8_C1B1);
        assert_eq!(CRYPT_TABLE[0x400], 0x193A_A698);
        assert_eq!(CRYPT_TABLE[0x4FF], 0x7303_286C);
    }

    #[test]
    fn table_keys() {
        assert_eq!(hash(b"(hash table)", HashType::FileKey), HASH_TABLE_KEY);
        assert_eq!(hash(b"(block table)", HashType::FileKey), BLOCK_TABLE_KEY);
    }

    #[test]
    fn hash_is_case_and_separator_insensitive() {
        for kind in [HashType::TableOffset, HashType::NameA, HashType::NameB] {
            assert_eq!(
                hash(br"data\global\excel\armor.txt", kind),
                hash(b"DATA/GLOBAL/EXCEL/ARMOR.TXT", kind)
            );
        }
    }

    #[test]
    fn encrypt_decrypt_round_trip() {
        let original: Vec<u8> = (0..=255u8).cycle().take(1027).collect();
        for key in [0, 1, 0xDEAD_BEEF, HASH_TABLE_KEY] {
            let mut data = original.clone();
            encrypt(&mut data, key);
            assert_ne!(data[..1024], original[..1024]);
            assert_eq!(data[1024..], original[1024..], "tail bytes untouched");
            decrypt(&mut data, key);
            assert_eq!(data, original);
        }
    }
}
