//! Socketing from the inventory panel: whether the cursor item can go into the
//! item under the mouse. The fillers' properties in the tool tip are summed
//! into the item's property block (`ui::item_tip_world`).
//!
//! The decision uses only what the model holds: the header flags of the
//! two items (0x800 socketed, 0x10 identified, 0x100), the socket count
//! of the target's record, the fillers the model lists under the target
//! (mode 6, owner = the target, 0x9D action 0x13) and the filler type.
//! The server decides the outcome.
//!
//! D2RUST's own, unverified: the filler type test.

use crate::bridge::items::{self, mode, ItemView};
use crate::bridge::world::ClientWorld;
use crate::ui::inv_grid::{can_socket, SocketFacts};
use crate::ui::item_tip::ItemTips;
use d2_proto::client::SocketItem;
use d2_proto::item_bits::hflag;

/// What the socket decision reads from the tables and streams.
pub trait SocketInfo {
    fn is_socket_filler(&self, code: [u8; 4]) -> bool;
    /// The target's socket count (its record's stat 194).
    fn sockets(&self, stream: &[u8]) -> Option<u8>;
}

impl SocketInfo for ItemTips {
    fn is_socket_filler(&self, code: [u8; 4]) -> bool {
        ItemTips::is_socket_filler(self, code)
    }
    fn sockets(&self, stream: &[u8]) -> Option<u8> {
        self.bits(stream)?.sockets
    }
}

/// The fillers listed under `target`, in key order.
pub fn fillers(world: &ClientWorld, target: &ItemView) -> Vec<ItemView> {
    items::items(world)
        .into_iter()
        .filter(|i| i.mode == mode::SOCKETED && i.owner == Some(target.key))
        .collect()
}

/// The inputs of the socket check for `cursor` over `target`.
pub fn socket_facts(
    info: &dyn SocketInfo,
    world: &ClientWorld,
    cursor: &ItemView,
    target: &ItemView,
) -> SocketFacts {
    SocketFacts {
        filler: cursor.code.is_some_and(|c| info.is_socket_filler(c)),
        target_socketed: target.flags & hflag::SOCKETED != 0,
        target_identified: target.flags & hflag::IDENTIFIED != 0,
        target_flag_100: target.flags & 0x100 != 0,
        filled: fillers(world, target).len() as u32,
        sockets: items::stream(world, target.key)
            .and_then(|s| info.sockets(s))
            .map_or(0, u32::from),
        mode: target.mode,
    }
}

/// C→S 0x28 for `cursor` over `target` when the facts allow it.
pub fn socket_intent(
    info: &dyn SocketInfo,
    world: &ClientWorld,
    cursor: &ItemView,
    target: &ItemView,
) -> Option<SocketItem> {
    can_socket(&socket_facts(info, world, cursor, target)).then_some(SocketItem {
        socketable: cursor.key.guid,
        target: target.key.guid,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::world::{ClientUnit, ItemData, ItemRecord, KindData, PlayerData, UnitKey};

    const PLAYER: UnitKey = UnitKey::new(0, 1);

    /// Gems are `gsw `; every target has 2 sockets.
    struct Info;
    impl SocketInfo for Info {
        fn is_socket_filler(&self, code: [u8; 4]) -> bool {
            &code == b"gsw "
        }
        fn sockets(&self, _: &[u8]) -> Option<u8> {
            Some(2)
        }
    }

    /// An item stream head: flags, version,
    /// mode, location, code.
    fn stream(flags: u32, m: u8, code: &[u8; 4]) -> Vec<u8> {
        let bits: Vec<(u32, u32)> = vec![
            (flags, 32),
            (0x65, 10),
            (u32::from(m), 3),
            (0, 4),
            (0, 4),
            (0, 4),
            (1, 3),
            (u32::from_le_bytes(*code), 32),
        ];
        let (mut out, mut acc, mut n) = (Vec::new(), 0u64, 0u32);
        for (v, w) in bits {
            acc |= u64::from(v) << n;
            n += w;
            while n >= 8 {
                out.push(acc as u8);
                acc >>= 8;
                n -= 8;
            }
        }
        if n > 0 {
            out.push(acc as u8);
        }
        out
    }

    fn add(
        w: &mut ClientWorld,
        guid: u32,
        owner: Option<UnitKey>,
        flags: u32,
        m: u8,
        code: &[u8; 4],
    ) {
        let k = UnitKey::new(items::ITEM, guid);
        let mut u = ClientUnit::new(k);
        u.kind = KindData::Item(ItemData {
            last: Some(ItemRecord {
                id: if owner.is_some() { 0x9D } else { 0x9C },
                action: 0x04,
                category: 0,
                owner,
                seq: 0,
                stream: stream(flags, m, code),
            }),
            ..ItemData::default()
        });
        w.units.insert(k, u);
    }

    /// A socketed identified sword (guid 10) holding `filled` gems and a
    /// gem (guid 20) on the cursor.
    fn world(filled: u32, sword_flags: u32) -> ClientWorld {
        let mut w = ClientWorld::default();
        let mut p = ClientUnit::new(PLAYER);
        p.kind = KindData::Player(PlayerData {
            cursor_item: Some(20),
            ..PlayerData::default()
        });
        w.units.insert(PLAYER, p);
        w.local_player = Some(PLAYER);
        add(&mut w, 10, None, sword_flags, 0, b"ssd ");
        add(&mut w, 20, None, 0x10, 4, b"gsw ");
        for i in 0..filled {
            let sword = UnitKey::new(items::ITEM, 10);
            add(&mut w, 30 + i, Some(sword), 0x10, 6, b"gsw ");
        }
        w
    }

    fn intent(w: &ClientWorld) -> Option<SocketItem> {
        let c = items::cursor_item(w)?;
        let t = items::item(w, UnitKey::new(items::ITEM, 10))?;
        socket_intent(&Info, w, &c, &t)
    }

    #[test]
    fn a_gem_over_a_socketed_item_with_a_free_socket_sends_0x28() {
        let w = world(1, 0x810);
        assert_eq!(
            intent(&w),
            Some(SocketItem {
                socketable: 20,
                target: 10
            })
        );
    }

    #[test]
    fn a_full_plain_or_unidentified_target_sends_nothing() {
        assert_eq!(intent(&world(2, 0x810)), None, "no free socket");
        assert_eq!(intent(&world(0, 0x10)), None, "not socketed");
        assert_eq!(intent(&world(0, 0x800)), None, "not identified");
    }

    #[test]
    fn a_non_filler_on_the_cursor_sends_nothing() {
        let mut w = world(0, 0x810);
        add(&mut w, 20, None, 0x10, 4, b"key ");
        assert_eq!(intent(&w), None);
    }
}
