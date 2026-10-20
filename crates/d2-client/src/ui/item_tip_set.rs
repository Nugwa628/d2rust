//! The tool tip of an identified set item: only the bonus lists the client has
//! received are shown (the client never computes set bonuses). The item's
//! partial lists 165 + k come with its stream, the owner's state 165–170 lists
//! with S→C 0xA8 / 0xAA, and which pieces are worn or owned from the host
//! ([`super::item_tip_build::SetCtx`]). Unverified until the `text-0002` set
//! capture cases run.

use super::item_tip::color;
use super::item_tip_build::{pfx, Build, TipText};
use super::item_tip_desc::{sid, DescNames};
use super::item_tip_props::{self as props, StatList};

/// First set state (165–170).
const SET_STATE: u8 = 165;

/// Text of one received list (state ≠ 0: no runeword list, no
/// fillers), multi-line.
fn described(build: &Build, l: &StatList, undead: bool) -> Vec<u16> {
    let args = props::PropArgs {
        undead,
        multi: true,
        label: &[],
    };
    let item = props::PropItem::default();
    props::property_text(build.tips(), &build.view(), &item, l, &args)
}

/// The item's received list of set state 165 + `k`.
fn item_state_list(build: &Build, k: usize) -> Option<StatList> {
    let list = build.bits().lists.get(1 + k)?.as_ref()?;
    Some(build.tips().stat_list(list))
}

/// The set item tip.
pub(super) fn set_tip(build: &Build) -> TipText {
    let t = build.tips();
    let b = build.bits();
    let ctx = build.ctx();
    let s = |id: u16| t.string(id);
    let nl = s(sid::NL);
    let row = b.quality_fields.file_index.map(|i| i as usize);
    let rec = row.and_then(|r| t.lookup.setitems.get(r));
    let set_id = rec.map(|r| r.set);
    let owner = ctx.unit.or(ctx.player);
    let mut text = Vec::new();
    // r1: member list.
    let mut members = Vec::new();
    if let Some(set) = set_id {
        for (i, m) in t.lookup.setitems.iter().enumerate() {
            if m.set != set {
                continue;
            }
            let name = t.key_text(&t.set, i).unwrap_or_default();
            let tpl = s(super::item_tip_build::tid::NAME_SET);
            let mut line = fill0(&tpl, &name);
            line.extend(&nl);
            let k = if ctx.set.owned.contains(&(i as u32)) {
                color::GREEN
            } else {
                color::RED
            };
            members.extend(pfx(line, k));
        }
    }
    text.extend(pfx(members, color::GREEN));
    // r2: set name.
    let set_name = set_id
        .and_then(|i| usize::try_from(i).ok())
        .and_then(|i| t.set_names.get(i))
        .map(|&id| [s(id), nl.clone()].concat())
        .unwrap_or_default();
    text.extend(pfx(set_name, color::GOLD));
    // r3: the owner's active set-wide bonuses, on an equipped piece.
    if b.mode == 1 {
        if let (Some(u), Some(set)) = (owner, set_id) {
            for state in SET_STATE..=SET_STATE + 5 {
                let Some(l) = u.state_list(state) else {
                    continue;
                };
                if l.get(71, 0) != i32::from(set) {
                    continue;
                }
                let d = described(build, &l, false);
                if !d.is_empty() {
                    text.extend(&nl);
                    text.extend(pfx(d, color::GOLD));
                }
            }
        }
    }
    // r4.
    text.extend(&nl);
    // r5: the item's partial lists the worn pieces switch on.
    let mut partial = Vec::new();
    match rec.map(|r| r.add_func) {
        Some(1) => {
            let mask = ctx.set.worn_without;
            let n = usize::from(rec.map_or(0, |r| r.slot));
            for i in 0..=5usize {
                if i == n || mask & (1 << i) == 0 {
                    continue;
                }
                let k = if i > n { i - 1 } else { i };
                if let Some(l) = item_state_list(build, k) {
                    partial.extend(described(build, &l, false));
                }
            }
        }
        Some(2) => {
            let worn = ctx.set.worn_with.count_ones() as usize;
            for k in 0..worn.saturating_sub(1) {
                if let Some(l) = item_state_list(build, k) {
                    partial.extend(described(build, &l, false));
                }
            }
        }
        _ => {}
    }
    text.extend(pfx(partial, color::GREEN));
    // r6: the item's own blue lines.
    text.extend(pfx(
        [build.eth_sockets(), build.own_props()].concat(),
        color::BLUE,
    ));
    // r7: the base block; the class line is red only for a player of
    // another class.
    let class = t
        .lookup
        .find_code(b.code)
        .and_then(|i| t.lookup.itype_of(i))
        .map_or(7, |ty| ty.class);
    let other = owner.is_some_and(|u| u.unit_type() == 0 && u.class() != u32::from(class));
    text.extend(build.set_base_block(other));
    // r8.
    let text = build.set_store_lines(text);
    TipText {
        text,
        color: color::WHITE,
    }
}

/// 10089 `%0` filled with `name`.
fn fill0(tpl: &[u16], name: &[u16]) -> Vec<u16> {
    let pat = [u16::from(b'%'), u16::from(b'0')];
    match tpl.windows(2).position(|w| w == pat) {
        Some(i) => [&tpl[..i], name, &tpl[i + 2..]].concat(),
        None => tpl.to_vec(),
    }
}
