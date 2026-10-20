//! Small tests for server-to-client delivery: ids whose size rule is 0 cannot be
//! received, and per-client buffers pop in queue order. Part of the `d2-server`
//! test suite (see `mod.rs`), run against the TSV-driven fakes in `fakes.rs`.

use super::fakes::*;
use crate::buffers::ClientBuffers;
use crate::seams::*;

// Server ids with size 0 are rejected as invalid.
#[test]
fn size_zero_ids_are_not_receivable() {
    let s = TsvSizes::new();
    for id in [0x83u8, 0x84, 0x88] {
        assert_eq!(s.server_size(&[id; 16]), Err(SizeError::Invalid), "{id:#x}");
    }
}

// Messages leave a client's buffer in the order they were queued.
#[test]
fn buffers_pop_in_queue_order() {
    let mut b = ClientBuffers::new();
    b.add_client(0);
    MessageSink::queue(&mut b, 0, &[0x9C; 0x200]).unwrap();
    MessageSink::queue(&mut b, 0, &[0x9D; 4]).unwrap();
    assert_eq!(b.pop(0).unwrap()[0], 0x9C);
    assert_eq!(b.pop(0).unwrap()[0], 0x9D);
    assert!(b.pop(0).is_none());
}
