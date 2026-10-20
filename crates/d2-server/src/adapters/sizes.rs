//! [`MessageSizes`] on `d2-proto`'s generated size tables.
//!
//! The server's transport (`crate::seams`) needs the byte length of each wire
//! message to split a received buffer and to pack outgoing ones. `d2-proto`
//! owns the size rules; this adapter exposes them through the seam so the
//! server does not depend on how they are generated.

use d2_proto::schema::Size;
use d2_proto::transport;

use crate::seams::{MessageSizes, SizeError};

/// The size rules of `tables/sim/client-messages.tsv` (`transport_size`)
/// and `server-messages.tsv` (`size`), as generated into `d2-proto`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProtoSizes;

fn to_seam(size: Size) -> Result<usize, SizeError> {
    match size {
        Size::Bytes(n) => Ok(n),
        Size::Invalid => Err(SizeError::Invalid),
        Size::Incomplete => Err(SizeError::Incomplete),
        Size::Negative(n) => Err(SizeError::Negative(n)),
    }
}

impl MessageSizes for ProtoSizes {
    fn client_size(&self, msg: &[u8]) -> Result<usize, SizeError> {
        to_seam(transport::client_size(msg))
    }

    fn server_size(&self, msg: &[u8]) -> Result<usize, SizeError> {
        to_seam(transport::server_size(msg))
    }
}
