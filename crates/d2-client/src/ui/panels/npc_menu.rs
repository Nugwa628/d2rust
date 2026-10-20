//! The NPC menu box built from the option table
//! and its captions, and the hire list. The box object is
//! [`super::menu_box`]; the option table and its edits are [`super::npc`].

use d2_proto::client::EntityAction;

use super::menu_box::{MenuBox, MenuError, MenuParams};
use super::npc::STR_IDENTIFY;
use super::PanelOutput;
use crate::ui::layout::NpcMenuRecord;
use crate::ui::messages::{msg_u32s, Metrics};
use crate::ui::panel::ClientIntent;

/// `lowercasecancel`.
pub const STR_LOWER_CANCEL: u16 = 4142;
/// The Resurrect slot's string id (inserted for a dead hireling).
pub const STR_RESURRECT_SLOT: u16 = 0x1507;
/// `hireresurrect2` "Resurrect %s: %d".
pub const STR_HIRE_RESURRECT: u16 = 22696;
/// `NPCHeal`.
pub const STR_HEAL: u16 = 3337;
/// `NPCIdentify2` "Identify Items: ".
pub const STR_IDENTIFY_COST: u16 = 4021;
/// The mercenary name id that stands for string 11021.
pub const MERC_NAME_ALT_ID: u16 = 0x421;
pub const STR_MERC_NAME_ALT: u16 = 11021;
/// NPC classes whose menu build first sends the hire-list request C→S
/// 0x38.
pub const HIRE_REQUEST_NPCS: [u32; 4] = [252, 198, 515, 150];

/// Handlers of the NPC menu box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NpcMenuHandler {
    /// P1 and the cancel item.
    Cancel,
    /// (p2).
    P2,
    /// The option slot's handler.
    Slot(usize),
}

/// What the menu captions read.
#[derive(Clone, Debug, Default)]
pub struct CaptionCtx {
    /// The mercenary's name string id.
    pub merc_name_id: u16,
    /// The hireling's resurrect cost.
    pub resurrect_cost: i32,
    /// Life (stat 6) is below max life.
    pub life_below_max: bool,
    pub heal_cost: i32,
    /// Items to identify.
    pub identify_n: i32,
    /// Quest record 4 bits 0 and 1 are both clear.
    pub identify_quest_bits_clear: bool,
    /// Quest record 41 bits 0 and 1.
    pub quest41_bit0: bool,
    pub quest41_bit1: bool,
    pub difficulty: u8,
}

/// A caption of an option slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Caption {
    /// The slot is not shown.
    Skip,
    Text(Vec<u16>),
}

fn utf16s(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Replaces the conversions `%s`, `%d` (and `%2u`) in order.
fn fmt_ws(fmt: &[u16], args: &[Vec<u16>]) -> Vec<u16> {
    let mut out = Vec::new();
    let mut a = args.iter();
    let mut i = 0;
    while i < fmt.len() {
        let u = fmt[i];
        let next = fmt.get(i + 1).copied();
        if u == u16::from(b'%') && (next == Some(u16::from(b's')) || next == Some(u16::from(b'd')))
        {
            if let Some(v) = a.next() {
                out.extend_from_slice(v);
            }
            i += 2;
        } else {
            out.push(u);
            i += 1;
        }
    }
    out
}

/// The caption of an option slot by its string id and the count change it
/// forces.
pub fn slot_caption(
    string: u16,
    ctx: &CaptionCtx,
    strings: &dyn Fn(u16) -> Vec<u16>,
) -> (Caption, Option<u32>) {
    match string {
        STR_RESURRECT_SLOT => {
            let name = if ctx.merc_name_id == MERC_NAME_ALT_ID {
                strings(STR_MERC_NAME_ALT)
            } else {
                strings(ctx.merc_name_id)
            };
            let cost = utf16s(&ctx.resurrect_cost.to_string());
            (
                Caption::Text(fmt_ws(&strings(STR_HIRE_RESURRECT), &[name, cost])),
                None,
            )
        }
        STR_HEAL => {
            if !ctx.life_below_max {
                return (Caption::Skip, None);
            }
            // The string, then the heal cost (0 is shown as 1) as `%d`.
            let mut t = strings(STR_HEAL);
            t.extend(utf16s(&ctx.heal_cost.max(1).to_string()));
            (Caption::Text(t), None)
        }
        STR_IDENTIFY => {
            if ctx.identify_n == 0 {
                return (Caption::Skip, Some(2));
            }
            let t = if ctx.identify_quest_bits_clear {
                let mut t = strings(STR_IDENTIFY_COST);
                t.extend(utf16s(&(100 * ctx.identify_n).to_string()));
                t
            } else {
                strings(STR_IDENTIFY)
            };
            (Caption::Text(t), Some(3))
        }
        11168 => {
            if !ctx.quest41_bit0 && (ctx.quest41_bit1 || ctx.difficulty == 2) {
                (Caption::Text(strings(11168)), None)
            } else {
                (Caption::Skip, None)
            }
        }
        other => (Caption::Text(strings(other)), None),
    }
}

/// What the NPC menu build needs of the interaction.
pub struct NpcMenuInput<'a> {
    pub npc_class: u32,
    pub npc_guid: u32,
    /// An interaction is active and its NPC exists.
    pub interaction_ok: bool,
    /// The NPC's name.
    pub npc_name: Vec<u16>,
    pub player_guid: Option<u32>,
    pub captions: &'a CaptionCtx,
}

/// The result of the menu build.
pub enum NpcMenuBuild {
    /// No active interaction or no NPC: the interaction ends and `SetUIState(8,
    /// off, 0)`.
    Ended(Vec<PanelOutput>),
    Built {
        /// C→S 0x38 [3][NPC GUID][player GUID or −1] for classes 252, 198,
        /// 515, 150.
        send: Option<PanelOutput>,
        bx: MenuBox<NpcMenuHandler>,
    },
}

/// The NPC menu (built at interaction start and when a sub-menu goes back).
/// `rec` is the record of the NPC's class (its count can change with the
/// captions).
pub fn build_npc_menu(
    rec: &mut NpcMenuRecord,
    input: &NpcMenuInput<'_>,
    anchor: (i32, i32),
    strings: &dyn Fn(u16) -> Vec<u16>,
    frame: (i32, i32),
    m: &dyn Metrics,
) -> Result<NpcMenuBuild, MenuError> {
    if !input.interaction_ok {
        return Ok(NpcMenuBuild::Ended(vec![PanelOutput::SetUi {
            ui: 8,
            mode: 1,
            jump: false,
        }]));
    }
    let send = HIRE_REQUEST_NPCS.contains(&input.npc_class).then(|| {
        PanelOutput::Intent(ClientIntent::from_message(&EntityAction {
            action: 3,
            npc: input.npc_guid,
            item: input.player_guid.unwrap_or(u32::MAX),
        }))
    });
    let mut b = MenuBox::new(
        anchor,
        MenuParams {
            p1: Some(NpcMenuHandler::Cancel),
            p2: Some(NpcMenuHandler::P2),
            p5: true,
            p9: 1,
            ..Default::default()
        },
    )
    .expect("p1 is set");
    b.set_style(1);
    b.add_item(&input.npc_name, 21, 0, 4, 1, None, false, m)?;
    // Option slots 0 … count − 2; a caption can change the count.
    let mut slot = 0;
    while slot + 1 < rec.count as usize {
        if let Some(o) = rec.options.get(slot).copied().flatten() {
            let (cap, count) = slot_caption(o.string, input.captions, strings);
            if let Some(c) = count {
                rec.count = c;
            }
            if let Caption::Text(t) = cap {
                b.add_item(&t, 15, 0, 0, 1, Some(NpcMenuHandler::Slot(slot)), true, m)?;
            }
        }
        slot += 1;
    }
    b.add_item(
        &strings(STR_LOWER_CANCEL),
        15,
        0,
        0,
        1,
        Some(NpcMenuHandler::Cancel),
        true,
        m,
    )?;
    b.layout(frame.0, frame.1, m)?;
    Ok(NpcMenuBuild::Built { send, bx: b })
}

/// Hire box size and list size.
pub const HIRE_BOX: (i32, i32) = (490, 350);
pub const HIRE_LIST: (i32, i32) = (490, 280);
/// `ItemDesc1s`, `ItemDesc1t`, `Back`.
pub const STR_YOUR_GOLD: u16 = 3364;
pub const STR_NO_MERCS: u16 = 3365;
pub const STR_BACK: u16 = 3400;
/// Hire records: 10, stride 16.
pub const HIRE_RECORDS: usize = 10;
pub const HIRE_STRIDE: usize = 16;

/// The positions of the hire box and list: box x = (W − 490)
/// / 2, y = (H − 40) / 2 − 195; list at the same x, y = (H − 40) / 2 −
/// 160.
pub fn hire_geometry(w: i32, h: i32) -> ((i32, i32), (i32, i32)) {
    let x = (w - 490) / 2;
    let y = (h - 40) / 2;
    ((x, y - 195), (x, y - 160))
}

/// The handlers of the hire box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HireHandler {
    /// Back (close both, rebuild the NPC menu).
    Back,
}

/// What clicking "hire" does first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HireOpen {
    /// A hire list is up: close it and its list widget, rebuild the NPC
    /// menu, done.
    CloseAndRebuild,
    /// Build the box and list.
    Build,
}

/// The NPC menu box closes; an open hire list closes instead of
/// opening again.
pub fn hire_open(hire_list_up: bool) -> HireOpen {
    if hire_list_up {
        HireOpen::CloseAndRebuild
    } else {
        HireOpen::Build
    }
}

/// The hire box: `ItemDesc1s` with the player's gold (stat 14 +
/// stat 15), height 21, font 1, color 4, not selectable; `Back` (3400),
/// height 315, font 1, color 0, handler back, selectable. Fixed 490 × 350
/// (p5 = 0), p9 = 1, style 1.
pub fn hire_box(
    w: i32,
    h: i32,
    gold: i32,
    strings: &dyn Fn(u16) -> Vec<u16>,
    m: &dyn Metrics,
) -> Result<MenuBox<HireHandler>, MenuError> {
    let (pos, _) = hire_geometry(w, h);
    let mut b = MenuBox::new(
        pos,
        MenuParams {
            p1: Some(HireHandler::Back),
            p6: HIRE_BOX.0,
            p7: HIRE_BOX.1,
            p9: 1,
            ..Default::default()
        },
    )
    .expect("p1 is set");
    b.set_style(1);
    let title = fmt_ws(&strings(STR_YOUR_GOLD), &[utf16s(&gold.to_string())]);
    b.add_item(&title, 21, 0, 4, 1, None, false, m)?;
    b.add_item(
        &strings(STR_BACK),
        315,
        0,
        0,
        1,
        Some(HireHandler::Back),
        true,
        m,
    )?;
    Ok(b)
}

/// One hire record: name id u16 @0; used when the u32 @8 ≠ 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HireRecord {
    pub name: u16,
    pub used: bool,
}

impl HireRecord {
    pub fn from_bytes(b: &[u8; HIRE_STRIDE]) -> Self {
        Self {
            name: u16::from_le_bytes([b[0], b[1]]),
            used: u32::from_le_bytes([b[8], b[9], b[10], b[11]]) != 0,
        }
    }
}

/// The rows of the list: one per used record (its index), or the
/// single "no mercenaries" row (`None`).
pub fn hire_rows(records: &[HireRecord]) -> Vec<Option<usize>> {
    let used: Vec<Option<usize>> = records
        .iter()
        .take(HIRE_RECORDS)
        .enumerate()
        .filter(|(_, r)| r.used)
        .map(|(i, _)| Some(i))
        .collect();
    if used.is_empty() {
        vec![None]
    } else {
        used
    }
}

/// The stats of a hireling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HireStats {
    /// +4.
    pub level: u32,
    /// +8.
    pub hp: u32,
    /// +0x1C.
    pub ac: u32,
    /// +0x14.
    pub cost: u32,
}

/// One row added through the widget's vtable +0x30 (left, right, 0, 0, 0,
/// 0, row index, 0, 1, 0x0E).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HireRowCall {
    pub left: Vec<u16>,
    pub right: Vec<u16>,
    pub index: usize,
}

/// Runs first on the list widget.
pub const HIRE_WIDGET_SETUP: (u32, u32) = (0x23, 0x1E);
const STR_SPACE: u16 = 3995;
const STR_DASH: u16 = 3996;
const STR_COLON: u16 = 3997;
const FIELD_LABELS: [u16; 4] = [3368, 3366, 3367, 3369];

/// The row text of one used record: left = the name (0x421 →
/// 11021) + space dash space, then four fields (`Level`, `HP`, `AC`,
/// `Cost`), each = the label (cut to 20 units) + colon + space + the value
/// as `%2u` + two spaces; right = two spaces, then the skill text when the
/// type has a skill.
pub fn hire_row_text(
    name: u16,
    stats: &HireStats,
    skill_text: Option<Vec<u16>>,
    strings: &dyn Fn(u16) -> Vec<u16>,
) -> (Vec<u16>, Vec<u16>) {
    let mut left = if name == MERC_NAME_ALT_ID {
        strings(STR_MERC_NAME_ALT)
    } else {
        strings(name)
    };
    for s in [STR_SPACE, STR_DASH, STR_SPACE] {
        left.extend(strings(s));
    }
    for (label, v) in FIELD_LABELS
        .iter()
        .zip([stats.level, stats.hp, stats.ac, stats.cost])
    {
        left.extend(strings(*label).into_iter().take(20));
        left.extend(strings(STR_COLON));
        left.extend(strings(STR_SPACE));
        left.extend(utf16s(&format!("{v:2}")));
        left.extend(strings(STR_SPACE));
        left.extend(strings(STR_SPACE));
    }
    let mut right = strings(STR_SPACE);
    right.extend(strings(STR_SPACE));
    if let Some(s) = skill_text {
        right.extend(s);
    }
    (left, right)
}

/// The rows of the list with their text; the builder stops at the first
/// record whose stats fail.
pub fn hire_row_calls(
    records: &[HireRecord],
    stats: &dyn Fn(usize) -> Option<HireStats>,
    skill: &dyn Fn(usize) -> Option<Vec<u16>>,
    strings: &dyn Fn(u16) -> Vec<u16>,
) -> Vec<HireRowCall> {
    let mut out = Vec::new();
    for (row, idx) in hire_rows(records).into_iter().enumerate() {
        match idx {
            None => out.push(HireRowCall {
                left: strings(STR_NO_MERCS),
                right: Vec::new(),
                index: row,
            }),
            Some(i) => {
                let Some(st) = stats(i) else { break };
                let (left, right) = hire_row_text(records[i].name, &st, skill(i), strings);
                out.push(HireRowCall {
                    left,
                    right,
                    index: row,
                });
            }
        }
    }
    out
}

/// What the player's click on a row does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HireChoose {
    /// The click sound.
    pub sound: u32,
    pub action: HireAction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HireAction {
    /// Nothing more (a row that is not a hireling).
    None,
    /// C→S 0x36 [NPC GUID @1][the record's u16 @0 zero-extended @5]; the list
    /// closes, the hire state:= 10 and the waiting note opens.
    Hire { send: PanelOutput },
    /// The list closes, transaction kind:= 5 and the confirm
    /// dialog opens.
    Confirm,
}

/// The facts of a choose.
#[derive(Clone, Copy, Debug)]
pub struct ChooseFacts {
    /// The widget's selection.
    pub row: usize,
    /// The row's item value (< 10 for a hireling row).
    pub item_value: usize,
    pub npc_guid: u32,
    /// The record's u16 @0.
    pub record_name: u16,
    /// = −1: the player has no hireling.
    pub no_hireling: bool,
    pub classic_game: bool,
    /// = 0 and the hireling's id = 0xFFFF.
    pub merc_state_clear: bool,
}

/// Click sound 2; only for row < 10 whose item value
/// is < 10. A player with no hireling hires at once in a classic game or
/// when the mercenary state is clear; otherwise the confirm dialog opens.
pub fn hire_choose(f: &ChooseFacts) -> HireChoose {
    if f.row >= HIRE_RECORDS || f.item_value >= HIRE_RECORDS {
        return HireChoose {
            sound: 2,
            action: HireAction::None,
        };
    }
    let direct = f.no_hireling && (f.classic_game || f.merc_state_clear);
    let action = if direct {
        HireAction::Hire {
            send: PanelOutput::Intent(msg_u32s(0x36, &[f.npc_guid, u32::from(f.record_name)])),
        }
    } else {
        HireAction::Confirm
    };
    HireChoose { sound: 2, action }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::layout::{MenuOption, OptionKind};
    use crate::ui::messages::testutil::{w, Fixed};

    fn strings(id: u16) -> Vec<u16> {
        match id {
            3364 => w("Gold: %d  Hire?"),
            3365 => w("None"),
            3995 => w(" "),
            3996 => w("-"),
            3997 => w(":"),
            3368 => w("Lvl"),
            3366 => w("Life"),
            3367 => w("Def"),
            3369 => w("Cost"),
            11021 => w("Rogue"),
            22696 => w("Resurrect %s: %d"),
            _ => w(&format!("s{id}")),
        }
    }

    fn rec(npc: u32, options: &[(u16, OptionKind)], count: u32) -> NpcMenuRecord {
        let mut o = [None; 5];
        for (i, &(string, kind)) in options.iter().enumerate() {
            o[i] = Some(MenuOption { string, kind });
        }
        NpcMenuRecord {
            record: 0,
            npc,
            count,
            options: o,
            flag: 0,
        }
    }

    fn input<'a>(class: u32, ctx: &'a CaptionCtx) -> NpcMenuInput<'a> {
        NpcMenuInput {
            npc_class: class,
            npc_guid: 0x11,
            interaction_ok: true,
            npc_name: w("Akara"),
            player_guid: Some(0x22),
            captions: ctx,
        }
    }

    fn texts(b: &MenuBox<NpcMenuHandler>) -> Vec<String> {
        b.items
            .iter()
            .map(|i| String::from_utf16(&i.text).unwrap())
            .collect()
    }

    #[test]
    fn npc_menu_items_and_hire_request() {
        let ctx = CaptionCtx::default();
        let mut r = rec(
            148,
            &[(3381, OptionKind::Talk), (3396, OptionKind::Trade)],
            3,
        );
        let m = Fixed;
        let NpcMenuBuild::Built { send, bx } = build_npc_menu(
            &mut r,
            &input(148, &ctx),
            (300, 200),
            &strings,
            (800, 600),
            &m,
        )
        .unwrap() else {
            panic!()
        };
        // The name (21, color 4), the slots (15), `lowercasecancel`.
        assert_eq!(texts(&bx), ["Akara", "s3381", "s3396", "s4142"]);
        assert!(send.is_none());
        let it = &bx.items;
        assert_eq!(
            (it[0].height, it[0].color, it[0].font, it[0].selectable),
            (21, 4, 1, false)
        );
        for i in &it[1..] {
            assert_eq!((i.height, i.color, i.font, i.selectable), (15, 0, 1, true));
        }
        assert_eq!(it[1].handler, Some(NpcMenuHandler::Slot(0)));
        assert_eq!(it[2].handler, Some(NpcMenuHandler::Slot(1)));
        assert_eq!(it[3].handler, Some(NpcMenuHandler::Cancel));
        assert_eq!((bx.style, bx.params.p5, bx.params.p9), (1, true, 1));
        assert_eq!(bx.params.p1, Some(NpcMenuHandler::Cancel));
        assert_eq!(bx.params.p2, Some(NpcMenuHandler::P2));
        // Classes 252, 198, 515, 150 first send C→S 0x38 [3][NPC][player].
        for class in [252, 198, 515, 150] {
            let mut r = rec(class, &[(3381, OptionKind::Talk)], 2);
            let NpcMenuBuild::Built { send, .. } = build_npc_menu(
                &mut r,
                &input(class, &ctx),
                (300, 200),
                &strings,
                (800, 600),
                &m,
            )
            .unwrap() else {
                panic!()
            };
            assert_eq!(
                send,
                Some(PanelOutput::Intent(ClientIntent(vec![
                    0x38, 3, 0, 0, 0, 0x11, 0, 0, 0, 0x22, 0, 0, 0
                ])))
            );
        }
        // No player: −1.
        let mut inp = input(150, &ctx);
        inp.player_guid = None;
        let mut r = rec(150, &[(3381, OptionKind::Talk)], 2);
        let NpcMenuBuild::Built { send, .. } =
            build_npc_menu(&mut r, &inp, (300, 200), &strings, (800, 600), &m).unwrap()
        else {
            panic!()
        };
        assert_eq!(
            send,
            Some(PanelOutput::Intent(ClientIntent(vec![
                0x38, 3, 0, 0, 0, 0x11, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF
            ])))
        );
        // Without an active interaction or NPC: the interaction ends and
        // state 8 closes.
        let mut inp = input(148, &ctx);
        inp.interaction_ok = false;
        let mut r = rec(148, &[(3381, OptionKind::Talk)], 2);
        let b = build_npc_menu(&mut r, &inp, (0, 0), &strings, (800, 600), &m).unwrap();
        assert!(
            matches!(b, NpcMenuBuild::Ended(o) if o == vec![PanelOutput::SetUi { ui: 8, mode: 1, jump: false }])
        );
    }

    #[test]
    fn slot_captions() {
        let mut ctx = CaptionCtx {
            merc_name_id: 100,
            resurrect_cost: 5000,
            life_below_max: false,
            heal_cost: 0,
            identify_n: 0,
            identify_quest_bits_clear: true,
            ..Default::default()
        };
        let cap = |s: u16, c: &CaptionCtx| slot_caption(s, c, &strings);
        // 0x1507: "Resurrect %s: %d" with the name and the cost.
        assert_eq!(
            cap(0x1507, &ctx),
            (Caption::Text(w("Resurrect s100: 5000")), None)
        );
        // Name id 0x421 → string 11021.
        ctx.merc_name_id = 0x421;
        assert_eq!(
            cap(0x1507, &ctx),
            (Caption::Text(w("Resurrect Rogue: 5000")), None)
        );
        // 3337: only while life is below max; the cost 0 shown as 1.
        assert_eq!(cap(3337, &ctx), (Caption::Skip, None));
        ctx.life_below_max = true;
        assert_eq!(cap(3337, &ctx), (Caption::Text(w("s33371")), None));
        ctx.heal_cost = 45;
        assert_eq!(cap(3337, &ctx), (Caption::Text(w("s333745")), None));
        // 4020: n = 0 → count:= 2 and skipped; else count:= 3 with NPCIdentify2 +
        // 100 · n while quest 4 bits 0 and 1 are clear.
        assert_eq!(cap(4020, &ctx), (Caption::Skip, Some(2)));
        ctx.identify_n = 3;
        assert_eq!(cap(4020, &ctx), (Caption::Text(w("s4021300")), Some(3)));
        ctx.identify_quest_bits_clear = false;
        assert_eq!(cap(4020, &ctx), (Caption::Text(w("s4020")), Some(3)));
        // 11168: quest 41 bit 0 clear and (bit 1 set or difficulty 2).
        let mut c = CaptionCtx::default();
        assert_eq!(cap(11168, &c), (Caption::Skip, None));
        c.quest41_bit1 = true;
        assert_eq!(cap(11168, &c), (Caption::Text(w("s11168")), None));
        c.quest41_bit0 = true;
        assert_eq!(cap(11168, &c), (Caption::Skip, None));
        let c = CaptionCtx {
            difficulty: 2,
            ..Default::default()
        };
        assert_eq!(cap(11168, &c), (Caption::Text(w("s11168")), None));
        // Any other id: the string as is.
        assert_eq!(cap(3396, &ctx), (Caption::Text(w("s3396")), None));
        // The count change reaches the record: Cain with nothing to
        // identify shows no slot 1.
        let ctx = CaptionCtx::default();
        let mut r = rec(
            244,
            &[
                (3381, OptionKind::Talk),
                (4020, OptionKind::Identify),
                (3396, OptionKind::Trade),
            ],
            4,
        );
        let NpcMenuBuild::Built { bx, .. } = build_npc_menu(
            &mut r,
            &input(244, &ctx),
            (300, 200),
            &strings,
            (800, 600),
            &Fixed,
        )
        .unwrap() else {
            panic!()
        };
        assert_eq!(r.count, 2);
        assert_eq!(texts(&bx), ["Akara", "s3381", "s4142"]);
    }

    // Test vector "hire list at 640 × 480".
    #[test]
    fn hire_list_geometry_box_and_rows() {
        assert_eq!(hire_geometry(640, 480), ((75, 25), (75, 60)));
        assert_eq!(hire_geometry(800, 600), ((155, 85), (155, 120)));
        assert_eq!((HIRE_BOX, HIRE_LIST), ((490, 350), (490, 280)));
        // An open hire list closes and the NPC menu is rebuilt.
        assert_eq!(hire_open(true), HireOpen::CloseAndRebuild);
        assert_eq!(hire_open(false), HireOpen::Build);
        // The box.
        let b = hire_box(640, 480, 1234, &strings, &Fixed).unwrap();
        assert_eq!((b.anchor, b.size, b.style), ((75, 25), (490, 350), 1));
        assert_eq!((b.params.p5, b.params.p9), (false, 1));
        assert_eq!(b.items.len(), 2);
        assert_eq!(b.items[0].text, w("Gold: 1234  Hire?"));
        assert_eq!(
            (
                b.items[0].height,
                b.items[0].font,
                b.items[0].color,
                b.items[0].selectable
            ),
            (21, 1, 4, false)
        );
        assert_eq!(b.items[1].text, w("s3400"));
        assert_eq!(
            (
                b.items[1].height,
                b.items[1].font,
                b.items[1].color,
                b.items[1].selectable
            ),
            (315, 1, 0, true)
        );
        assert_eq!(b.items[1].handler, Some(HireHandler::Back));
        // 10 records of 16 bytes; used when the u32 @8 ≠ 0.
        let mut raw = [[0u8; HIRE_STRIDE]; HIRE_RECORDS];
        for (i, r) in raw.iter_mut().enumerate() {
            r[0] = i as u8 + 1;
            if i == 2 || i == 5 {
                r[8] = 1;
            }
        }
        let recs: Vec<HireRecord> = raw.iter().map(HireRecord::from_bytes).collect();
        assert_eq!(
            recs[2],
            HireRecord {
                name: 3,
                used: true
            }
        );
        assert_eq!(hire_rows(&recs), vec![Some(2), Some(5)]);
        // None used: the single "no mercenaries" row.
        let none = vec![
            HireRecord {
                name: 1,
                used: false
            };
            10
        ];
        assert_eq!(hire_rows(&none), vec![None]);
        let calls = hire_row_calls(&none, &|_| None, &|_| None, &strings);
        assert_eq!(
            calls,
            vec![HireRowCall {
                left: w("None"),
                right: vec![],
                index: 0
            }]
        );
    }

    #[test]
    fn hire_choose_paths() {
        let f = ChooseFacts {
            row: 1,
            item_value: 1,
            npc_guid: 0x10,
            record_name: 0x0123,
            no_hireling: true,
            classic_game: true,
            merc_state_clear: false,
        };
        // No hireling in a classic game: C→S 0x36 [NPC GUID][name u16].
        assert_eq!(
            hire_choose(&f),
            HireChoose {
                sound: 2,
                action: HireAction::Hire {
                    send: PanelOutput::Intent(ClientIntent(vec![
                        0x36, 0x10, 0, 0, 0, 0x23, 0x01, 0, 0
                    ]))
                }
            }
        );
        // Expansion: only with the mercenary state clear.
        let g = ChooseFacts {
            classic_game: false,
            ..f
        };
        assert_eq!(hire_choose(&g).action, HireAction::Confirm);
        let g = ChooseFacts {
            classic_game: false,
            merc_state_clear: true,
            ..f
        };
        assert!(matches!(hire_choose(&g).action, HireAction::Hire { .. }));
        // A player with a hireling: the confirm dialog (kind 5).
        let g = ChooseFacts {
            no_hireling: false,
            ..f
        };
        assert_eq!(hire_choose(&g).action, HireAction::Confirm);
        // Row 10 or a value ≥ 10: only the click sound.
        for g in [
            ChooseFacts { row: 10, ..f },
            ChooseFacts {
                item_value: 10,
                ..f
            },
        ] {
            assert_eq!(
                hire_choose(&g),
                HireChoose {
                    sound: 2,
                    action: HireAction::None
                }
            );
        }
    }

    #[test]
    fn hire_row_texts() {
        let st = HireStats {
            level: 9,
            hp: 120,
            ac: 45,
            cost: 12345,
        };
        let (l, r) = hire_row_text(0x421, &st, Some(w("Cold Arrow")), &strings);
        // Name (0x421 → 11021), space dash space, then label colon space
        // %2u two spaces for each field.
        assert_eq!(
            String::from_utf16(&l).unwrap(),
            "Rogue - Lvl:  9  Life: 120  Def: 45  Cost: 12345  "
        );
        assert_eq!(String::from_utf16(&r).unwrap(), "  Cold Arrow");
        // No skill: only the two spaces.
        let (_, r) = hire_row_text(7, &st, None, &strings);
        assert_eq!(r, w("  "));
        // The label is cut to 20 units.
        let long = |id: u16| {
            if id == 3368 {
                vec![65u16; 30]
            } else {
                strings(id)
            }
        };
        let (l, _) = hire_row_text(7, &st, None, &long);
        assert!(String::from_utf16(&l)
            .unwrap()
            .contains(&format!("{}:", "A".repeat(20))));
        assert!(!String::from_utf16(&l).unwrap().contains(&"A".repeat(21)));
        // Rows are added in used-record order; the builder stops at the
        // first record whose stats fail.
        let recs = vec![
            HireRecord {
                name: 1,
                used: true,
            },
            HireRecord {
                name: 2,
                used: true,
            },
            HireRecord {
                name: 3,
                used: true,
            },
        ];
        let rows = hire_row_calls(&recs, &|i| (i != 1).then_some(st), &|_| None, &strings);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].index, 0);
        assert_eq!(HIRE_WIDGET_SETUP, (0x23, 0x1E));
    }

    // The menu's edge cases, each reproduced.
    #[test]
    fn menus_edge_cases_reproduced() {
        use crate::ui::panels::menu_box::{MenuBox, MenuParams, PENTSPIN_MOD};
        use crate::ui::panels::shop::{Pending, SendFacts, ShopEffect, ShopTx, TxKind, MK_SHIFT};
        use crate::ui::panels::waypoint::{
            set_tab, WaypointPanel, TAB_DRAW_QUEST_RECORD, TAB_QUEST_RECORD,
        };
        // Waypoint tab 4 is drawn on quest record 26 but selected on 28:
        // with 26 set and 28 clear a click on the drawn tab 4 opens tab 3
        // or lower.
        assert_eq!(TAB_DRAW_QUEST_RECORD[4], 26);
        assert_eq!(TAB_QUEST_RECORD[4], 28);
        let q = |r: u32| matches!(r, 7 | 15 | 23 | 26);
        assert_eq!(set_tab(4, &q), 3);
        // The waypoint self-close keeps drawing the menu in the frame it
        // closes.
        let mut p = WaypointPanel::new();
        p.open(7);
        assert!(p.self_close_checked(true, false).draw_continues);
        // C→S 0x35 for one item carries the durability, not the price.
        let pf = |_t: u8| 999;
        let mut t = ShopTx::default();
        t.pending = Some(Pending {
            kind: TxKind::Repair,
            item_guid: 9,
            item_class: 0,
            price: 999,
            t: 3,
            repair_all: false,
        });
        t.npc_guid = 5;
        let e = t.send_with(
            MK_SHIFT,
            1,
            &SendFacts {
                item_found: true,
                item_mode: 0,
                durability: 41,
                item_flag_1a5: false,
                item_is_cursor: false,
            },
        );
        assert!(e.contains(&ShopEffect::Send(crate::ui::messages::msg_u32s(
            0x35,
            &[5, 9, 0, 41]
        ))));
        let _ = pf;
        // The pentspin counter uses `% 7` on an 8-frame file: frame 7 is
        // never drawn.
        assert_eq!(PENTSPIN_MOD, 7);
        let m = Fixed;
        let mut b = MenuBox::new(
            (100, 100),
            MenuParams {
                p1: Some(1u8),
                p5: true,
                ..Default::default()
            },
        )
        .unwrap();
        b.add_item(&w("x"), 15, 0, 0, 1, Some(1), true, &m).unwrap();
        b.layout(640, 480, &m).unwrap();
        let mut frames = std::collections::BTreeSet::new();
        let mut c = 0;
        for _ in 0..16 {
            for d in b.draw(&mut c, &m) {
                if let crate::ui::panels::menu_box::MenuDraw::Pentspin { frame, .. } = d {
                    frames.insert(frame);
                }
            }
        }
        assert_eq!(
            frames.into_iter().collect::<Vec<_>>(),
            (0..7).collect::<Vec<_>>()
        );
    }
}
