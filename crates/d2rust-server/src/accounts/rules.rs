//! The account rules (contracts section 4): which account names and
//! passwords the server takes. Character names follow
//! [`crate::characters::check_name`].

/// Account names: 2 to 15 characters.
pub const ACCOUNT_NAME_LEN: std::ops::RangeInclusive<usize> = 2..=15;
/// Passwords: 8 to 64 bytes of UTF-8.
pub const PASSWORD_LEN: std::ops::RangeInclusive<usize> = 8..=64;

/// The name in lowercase ASCII: names are unique without regard to case.
pub fn name_key(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// Checks an account name: 2 to 15 ASCII letters, digits, `-` and `_`,
/// starting with a letter. The error says which rule it breaks.
pub fn check_account_name(name: &str) -> Result<(), &'static str> {
    if !ACCOUNT_NAME_LEN.contains(&name.len()) {
        return Err("An account name must be 2 to 15 characters long.");
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("An account name may only have letters, digits, - and _.");
    }
    if !name.as_bytes()[0].is_ascii_alphabetic() {
        return Err("An account name must start with a letter.");
    }
    Ok(())
}

/// Checks a password: 8 to 64 bytes.
pub fn check_password(password: &str) -> Result<(), &'static str> {
    if PASSWORD_LEN.contains(&password.len()) {
        Ok(())
    } else {
        Err("The password must be 8 to 64 bytes long.")
    }
}
