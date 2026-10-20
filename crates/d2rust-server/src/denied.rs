//! A refused request: what the server answers when accounts, the lobby or
//! a game turn a request down (contracts section 4). It carries the
//! error code, how long to wait for a limit or a lock, and the words to
//! send; the session adds the request's kind to make the `ERROR` answer.

use d2_net::msg::{ErrorCode, ErrorMsg};

/// A refused request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Denied {
    pub code: ErrorCode,
    /// Seconds to wait, for [`ErrorCode::RATE_LIMITED`] and
    /// [`ErrorCode::ACCOUNT_LOCKED`]; 0 otherwise.
    pub wait_s: u32,
    /// The server's words: the code's own, or which rule broke.
    pub text: String,
}

impl Denied {
    /// A refusal with the code's own words.
    pub fn new(code: ErrorCode) -> Self {
        Self {
            code,
            wait_s: 0,
            text: code.text().to_owned(),
        }
    }

    /// A refusal that says more than the code's words (which rule broke).
    pub fn because(code: ErrorCode, text: &str) -> Self {
        Self {
            code,
            wait_s: 0,
            text: text.to_owned(),
        }
    }

    /// A limit or a lock: try again after `wait_s`.
    pub fn wait(code: ErrorCode, wait_s: u32) -> Self {
        Self {
            code,
            wait_s,
            text: format!("{} Try again in {wait_s} s.", code.text()),
        }
    }

    /// The `ERROR` answer to the request of kind `request_kind`.
    pub fn to_msg(&self, request_kind: u8) -> ErrorMsg {
        ErrorMsg {
            request_kind,
            code: self.code,
            wait_s: self.wait_s,
            text: self.text.clone(),
        }
    }
}

impl From<ErrorCode> for Denied {
    fn from(code: ErrorCode) -> Self {
        Self::new(code)
    }
}

impl std::fmt::Display for Denied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_carry_the_code_wait_and_words() {
        let m = Denied::wait(ErrorCode::RATE_LIMITED, 6).to_msg(0x21);
        assert_eq!(
            (m.request_kind, m.code, m.wait_s),
            (0x21, ErrorCode::RATE_LIMITED, 6)
        );
        assert!(m.text.ends_with("Try again in 6 s."), "{}", m.text);
        let d: Denied = ErrorCode::WRONG_LOGIN.into();
        assert_eq!(
            (d.wait_s, d.text.as_str()),
            (0, ErrorCode::WRONG_LOGIN.text())
        );
        let d = Denied::because(ErrorCode::BAD_CHAR_NAME, "Too long.");
        assert_eq!(
            ErrorMsg::parse(&d.to_msg(0x26).body()).unwrap().text,
            "Too long."
        );
    }
}
