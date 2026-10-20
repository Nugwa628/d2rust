//! The server's side beyond the stream layer: its lasting identity and
//! address validation.
//!
//! - **Identity.** A server's certificate is made once and kept in its
//!   identity folder ([`Identity::load_or_create`]): `cert.der` and
//!   `key.der` (the key readable by the server's user only). Clients pin the
//!   certificate's [`crate::fingerprint`], which the launcher manifest
//!   carries, so the same certificate must come back on every start and
//!   every version.
//! - **Retry.** A connection attempt from an address QUIC hasn't
//!   validated yet is answered with a retry ([`answer`]): the client must
//!   echo a token before the server spends a handshake on it, so spoofed
//!   source addresses can't make the server work or send for them.

use std::io;
use std::path::{Path, PathBuf};

use crate::{ConnectError, Identity};

/// The certificate's file in an identity folder.
pub const CERT_FILE: &str = "cert.der";
/// The private key's file in an identity folder.
pub const KEY_FILE: &str = "key.der";

impl Identity {
    /// The identity kept in `dir`, or a new one made and written there when
    /// the folder holds none. Returns it and whether it was made now. A
    /// folder with only one of the two files is an error: replacing half an
    /// identity would silently change the fingerprint.
    pub fn load_or_create(dir: &Path) -> Result<(Identity, bool), ConnectError> {
        let (cert, key) = (dir.join(CERT_FILE), dir.join(KEY_FILE));
        match (cert.exists(), key.exists()) {
            (true, true) => {
                let id = Identity {
                    cert: std::fs::read(&cert)?,
                    key: std::fs::read(&key)?,
                };
                Ok((id, false))
            }
            (false, false) => {
                let id = Identity::self_signed()?;
                std::fs::create_dir_all(dir)?;
                write_private(&key, &id.key)?;
                std::fs::write(&cert, &id.cert)?;
                Ok((id, true))
            }
            _ => Err(ConnectError::Io(io::Error::other(format!(
                "{} holds only one of {CERT_FILE} and {KEY_FILE}",
                dir.display()
            )))),
        }
    }

    /// The identity kept in `dir`; an error when there is none.
    pub fn load(dir: &Path) -> Result<Identity, ConnectError> {
        Ok(Identity {
            cert: std::fs::read(dir.join(CERT_FILE))?,
            key: std::fs::read(dir.join(KEY_FILE))?,
        })
    }
}

/// Writes `bytes` to a new file only its owner can read (on Unix).
fn write_private(path: &PathBuf, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// What the server does with a connection attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Ask the client to prove its address first.
    Retry,
    /// Go on with the handshake.
    Accept,
}

/// The answer to an attempt from an address QUIC has (`validated`) or
/// hasn't validated, when a retry is possible (`may_retry`).
pub fn answer(validated: bool, may_retry: bool) -> Answer {
    if !validated && may_retry {
        Answer::Retry
    } else {
        Answer::Accept
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_identity_is_made_once_and_kept() {
        let dir = std::env::temp_dir().join(format!("d2-quic-identity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (made, new) = Identity::load_or_create(&dir).unwrap();
        assert!(new);
        let (again, new) = Identity::load_or_create(&dir).unwrap();
        assert!(!new);
        assert_eq!(
            crate::fingerprint(&made.cert),
            crate::fingerprint(&again.cert)
        );
        assert_eq!(made.key, again.key);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(KEY_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "the key is the owner's only");
        }
        // Half an identity is refused, not replaced.
        std::fs::remove_file(dir.join(KEY_FILE)).unwrap();
        assert!(Identity::load_or_create(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unvalidated_addresses_are_retried_once() {
        assert_eq!(answer(false, true), Answer::Retry);
        // The retry's echo comes back validated (and can't be retried again).
        assert_eq!(answer(true, false), Answer::Accept);
        assert_eq!(answer(false, false), Answer::Accept);
    }
}
