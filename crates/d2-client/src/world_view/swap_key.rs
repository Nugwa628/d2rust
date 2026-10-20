//! The weapon-swap key (W, `Cfgswapweapons`): the action no panel took
//! leaves as C→S 0x60 SwapWeapons. The server trades the hands with the
//! swap set and answers with the item updates and S→C 0x97; this module
//! only asks (step 7). D2RUST's own, unverified: the
//! command's own handler is not specified further.

use crate::bridge::link::ServerLink;
use crate::bridge::{Bridge, BridgeError};
use crate::controls::Action;
use crate::ui::{ActionId, UiEvent};

/// The UI states that block the swap command (command 44: NPC shop,
/// trade, stash).
pub const SWAP_BLOCKING: [u8; 3] = [0x0C, 0x17, 0x19];

/// Sends C→S 0x60 for each swap-weapons action in `unhandled`; returns
/// how many left.
pub fn send_swaps<L: ServerLink>(
    unhandled: &[UiEvent],
    bridge: &mut Bridge<L>,
) -> Result<usize, BridgeError> {
    let swap = UiEvent::Action(ActionId(Action::SwapWeapons.index() as u16));
    let mut sent = 0;
    for _ in unhandled.iter().filter(|e| **e == swap) {
        bridge.send(&d2_proto::client::SwapWeapons)?;
        sent += 1;
    }
    Ok(sent)
}

/// The speech commands (commands 27–33, 55): C→S 0x3F
/// with value 0x19 + k for Say 0–6, 0x20 for Say 7X, one
/// per command; returns how many were sent.
pub fn send_says<L: ServerLink>(
    unhandled: &[UiEvent],
    bridge: &mut Bridge<L>,
) -> Result<usize, BridgeError> {
    use Action as A;
    const SAYS: [(Action, u16); 8] = [
        (A::Say0, 0x19),
        (A::Say1, 0x1A),
        (A::Say2, 0x1B),
        (A::Say3, 0x1C),
        (A::Say4, 0x1D),
        (A::Say5, 0x1E),
        (A::Say6, 0x1F),
        (A::Say7X, 0x20),
    ];
    let mut sent = 0;
    for e in unhandled {
        for &(a, sound) in &SAYS {
            if *e == UiEvent::Action(ActionId(a.index() as u16)) {
                bridge.send(&d2_proto::client::PlayAudio { sound })?;
                sent += 1;
            }
        }
    }
    Ok(sent)
}

/// Whether the swap command runs: none of
/// [`SWAP_BLOCKING`] is open.
pub fn swap_allowed(is_open: &dyn Fn(u8) -> bool) -> bool {
    !SWAP_BLOCKING.iter().any(|&s| is_open(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_swap_with_the_shop_trade_or_stash_open() {
        assert!(swap_allowed(&|_| false));
        for s in [0x0C, 0x17, 0x19] {
            assert!(!swap_allowed(&|u| u == s), "ui {s:#x}");
        }
        // The inventory or the cube do not block it.
        assert!(swap_allowed(&|u| u == 1 || u == 0x1A));
    }
}
