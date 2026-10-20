//! Building a full save and editing one: the header, the default
//! sections, the stats as a base list in ascending key order, the skill
//! bytes, quest bits, waypoint bits and inventory items (`items.rs`).
//!
//! A new character's stats are the class creation values (act 0) and,
//! for a level above 1, the experience of that level added through the
//! normal experience path (which levels up): what the game would hold
//! for a character that reached the level by experience alone.
//!
//! Not settled yet (refused or written as noted):
//! - "all quests complete": a played completion leaves no single bit
//!   pattern per quest, so `--quests all` is refused. `acts=N` sets only
//!   the bits the act transitions set (bit 0; bit 13 is cleared on load).
//! - the appearance bytes +0x88..+0xA7: the writer fills them with 0xFF
//!   and each equipped item (mode 1) changes some; that mapping is not
//!   known yet. With no equipped item (every file `new` writes) they are
//!   32 × 0xFF, as the game writes them; `set` on a save with an equipped
//!   item keeps the file's bytes and says so.
//!
//! Game-equivalent re-save (`set`, [`resave`]): a save the game writes
//! after loading a file holds every item without flag 0x2000 and the
//! rebuilt appearance bytes; `new` and `set` write the same.

use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Context, Result};
use d2_formats::d2s::ItemEntry;
use d2_formats::d2s::{
    clamp_stat, status, Body, D2s, Golem, Header, Npcs, Quests, SaveTables, Slot, StatEntry, Stats,
    Waypoints, ITEM_FLAG_INSTORE,
};
use d2_proto::item_bits::SaveEntry;
use d2_sim::combat::vitals::{self, VitalsTables, VitalsUnits};
use d2_sim::units::UnitType;

use crate::items::{make_item, ItemSpec, Owner, Pages, ToolGame};
use crate::tables::Tables;

/// Stat ids (rows of the item stat table).
pub mod sid {
    pub const LEVEL: u16 = 12;
    pub const EXPERIENCE: u16 = 13;
    pub const GOLD: u16 = 14;
    pub const GOLDBANK: u16 = 15;
}

/// Stash gold limit: more stash gold than this is set to 0 on load.
pub const STASH_LIMIT: i32 = 2_500_000;

/// Player class from `ama|sor|nec|pal|bar|dru|ass` or `0..6`.
pub fn parse_class(s: &str) -> Result<u8> {
    const NAMES: [&str; 7] = ["ama", "sor", "nec", "pal", "bar", "dru", "ass"];
    if let Some(i) = NAMES.iter().position(|n| s.eq_ignore_ascii_case(n)) {
        return Ok(i as u8);
    }
    match s.parse::<u8>() {
        Ok(c) if c <= 6 => Ok(c),
        _ => bail!("class {s:?}: want ama|sor|nec|pal|bar|dru|ass or 0..6"),
    }
}

/// A difficulty: `normal|nightmare|hell` or `0..2`.
pub fn parse_difficulty(s: &str) -> Result<u8> {
    match s.to_ascii_lowercase().as_str() {
        "normal" | "n" | "0" => Ok(0),
        "nightmare" | "nm" | "1" => Ok(1),
        "hell" | "h" | "2" => Ok(2),
        _ => bail!("difficulty {s:?}: want normal|nightmare|hell or 0..2"),
    }
}

/// `--quests`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuestSpec {
    /// All three records zero.
    None,
    /// Refused: not specified (module doc).
    All,
    /// Bits to set, per difficulty (`None` = all three).
    Items(Vec<QuestItem>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuestItem {
    /// The first `n` act transitions.
    Acts { diff: Option<u8>, n: u8 },
    /// Slot q (0..41), bit b (0..15).
    Bit { diff: Option<u8>, slot: u8, bit: u8 },
}

fn split_diff(s: &str) -> Result<(Option<u8>, &str)> {
    match s.split_once(':') {
        Some((d, rest)) => Ok((Some(parse_difficulty(d)?), rest)),
        None => Ok((None, s)),
    }
}

impl std::str::FromStr for QuestSpec {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "none" => return Ok(QuestSpec::None),
            "all" => return Ok(QuestSpec::All),
            _ => {}
        }
        let mut out = Vec::new();
        for part in s.split(',').filter(|p| !p.is_empty()) {
            let (diff, rest) = split_diff(part)?;
            if let Some(n) = rest.strip_prefix("acts=") {
                let n: u8 = n
                    .parse()
                    .map_err(|_| anyhow!("bad act count in {part:?}"))?;
                out.push(QuestItem::Acts { diff, n });
            } else if let Some((q, b)) = rest.split_once('.') {
                let slot: u8 = q.parse().map_err(|_| anyhow!("bad slot in {part:?}"))?;
                let bit: u8 = b.parse().map_err(|_| anyhow!("bad bit in {part:?}"))?;
                if slot > 41 || bit > 15 {
                    bail!("{part:?}: slot 0..41, bit 0..15 (world/quests.md §1.1, §1.3)");
                }
                out.push(QuestItem::Bit { diff, slot, bit });
            } else {
                bail!("quest item {part:?}: want [diff:]acts=N or [diff:]slot.bit");
            }
        }
        Ok(QuestSpec::Items(out))
    }
}

/// `--waypoints`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WpSpec {
    /// Fresh records: only the Rogue Encampment (index 0).
    None,
    /// Every index a `levels` row carries, every difficulty.
    All,
    /// Indices to set (`None` difficulty = all three).
    List(Vec<(Option<u8>, WpRef)>),
}

/// One item of a `--waypoints` list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WpRef {
    /// A waypoint index.
    Index(u8),
    /// `lv=ID`: the waypoint of level ID (its index in the `levels`
    /// table's waypoint order); a level without one is an error.
    Level(u16),
}

impl std::str::FromStr for WpSpec {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "none" => return Ok(WpSpec::None),
            "all" => return Ok(WpSpec::All),
            _ => {}
        }
        let mut out = Vec::new();
        for part in s.split(',').filter(|p| !p.is_empty()) {
            let (diff, rest) = split_diff(part)?;
            if let Some(lv) = rest.strip_prefix("lv=") {
                let id: u16 = lv.parse().map_err(|_| anyhow!("waypoint level {part:?}"))?;
                out.push((diff, WpRef::Level(id)));
                continue;
            }
            let n: u8 = rest
                .parse()
                .map_err(|_| anyhow!("waypoint index {part:?}"))?;
            if Waypoints::bit(n).is_none() {
                bail!("waypoint index {n}: must be < 112 (world/waypoints.md §1 rule 3)");
            }
            out.push((diff, WpRef::Index(n)));
        }
        Ok(WpSpec::List(out))
    }
}

/// The edit flags `new` and `set` share.
#[derive(Clone, Debug, Default)]
pub struct Edits {
    pub name: Option<String>,
    pub class: Option<u8>,
    pub level: Option<u32>,
    pub expansion: bool,
    /// `Some(true)` `--hardcore`, `Some(false)` `--softcore`.
    pub hardcore: Option<bool>,
    pub stats: Vec<(u16, i32)>,
    pub skills: Vec<(usize, u8)>,
    pub all_skills: Option<u8>,
    /// Left / right mouse skill ids (header slots 0 and 1).
    pub left_skill: Option<i32>,
    pub right_skill: Option<i32>,
    /// 1-based owner item of the mouse skills (0 = none), `--left-skill S@N`.
    pub left_item: u16,
    pub right_item: u16,
    pub gold: Option<i32>,
    pub quests: Option<QuestSpec>,
    pub waypoints: Option<WpSpec>,
    pub unlocked: Option<u8>,
    pub items: Vec<ItemSpec>,
    /// Create / save time (+0x2C, +0x30).
    pub time: Option<u32>,
    /// Game seed the items' seeds derive from.
    pub seed: Option<u32>,
    pub map_seed: Option<u32>,
    /// Town act (0–4) and difficulty of the town byte (+0xA8).
    pub act: Option<u8>,
    pub difficulty: Option<u8>,
    /// Write a town byte of a difficulty the character has not unlocked (the
    /// game refuses such a file at load: internal 13/14 → result 17/18).
    pub allow_locked: bool,
    /// Hireling block (+0xAF): row `Id`, name index, seed,
    /// experience.
    pub merc: Option<(u16, u16, u32, u32)>,
    /// `--hotkey N=SKILL[,ITEM]`: slot 0..15 hotkeys, 16..19 the mouse slots;
    /// ITEM is the 1-based item index (0: none).
    pub hotkeys: Vec<(usize, i32, u16)>,
    /// `--golem ITEM`: the Iron Golem item of the `kf` section.
    pub golem: Option<ItemSpec>,
}

// ------------------------------------------------------------- vitals seam

/// The player's base stats for the vitals rules (no items, no lists: a
/// unit total is the base value).
struct PlayerStats {
    class: i32,
    base: BTreeMap<u16, i32>,
}

impl VitalsUnits for PlayerStats {
    type Unit = ();
    fn unit_type(&self, _: ()) -> UnitType {
        UnitType::Player
    }
    fn class_id(&self, _: ()) -> i32 {
        self.class
    }
    fn base_stat(&self, _: (), s: u16) -> i32 {
        self.base.get(&s).copied().unwrap_or(0)
    }
    fn stat(&self, u: (), s: u16) -> i32 {
        self.base_stat(u, s)
    }
    fn set_base_stat(&mut self, _: (), s: u16, v: i32) {
        self.base.insert(s, v);
    }
    fn add_base_stat(&mut self, _: (), s: u16, v: i32) {
        let e = self.base.entry(s).or_default();
        *e = e.wrapping_add(v);
    }
    fn max_life(&self, u: ()) -> i32 {
        self.base_stat(u, vitals::stat::MAXHP)
    }
    fn max_mana(&self, u: ()) -> i32 {
        self.base_stat(u, vitals::stat::MAXMANA)
    }
    fn max_stamina(&self, u: ()) -> i32 {
        self.base_stat(u, vitals::stat::MAXSTAMINA)
    }
    fn refresh(&mut self, _: ()) {}
    fn level_up_notify(&mut self, _: ()) {}
    fn level_up_event(&mut self, _: ()) {}
}

/// Experience of level `l`: the class's experience threshold of row
/// l − 1.
fn exp_for_level(v: &VitalsTables, class: u8, l: u32) -> Result<u32> {
    let max = v.max_level(i32::from(class));
    if l < 1 || l > max {
        bail!("level {l}: must be 1..={max} (experience table row 0)");
    }
    Ok(if l == 1 {
        0
    } else {
        v.threshold(i32::from(class), l - 1)
    })
}

/// Creation values (act 0) and the level reached by adding its
/// experience (which levels up as in play).
fn creation_stats(v: &VitalsTables, class: u8, level: u32) -> Result<BTreeMap<u16, i32>> {
    let need = exp_for_level(v, class, level)?;
    let mut p = PlayerStats {
        class: i32::from(class),
        base: BTreeMap::new(),
    };
    vitals::init_player_stats(&mut p, v, (), 0);
    if need > 0 {
        vitals::add_experience(&mut p, v, (), need);
    }
    let got = p.base_stat((), sid::LEVEL);
    if got != level as i32 {
        bail!("experience {need} gives level {got}, not {level} (experience table)");
    }
    Ok(p.base)
}

// --------------------------------------------------------------- stats

/// The base stats of a body, by (id, layer). Only the bit-field layout
/// (version > 0x5E) is edited.
pub fn stat_map(b: &Body) -> Result<BTreeMap<(u16, u16), i32>> {
    match &b.stats {
        Stats::Bits(v) => Ok(v.iter().map(|e| ((e.id, e.layer), e.value)).collect()),
        Stats::Mask { .. } => {
            bail!("stats in the 0x5C–0x5E mask layout (d2s.md §7.1 rule 7): not edited")
        }
    }
}

/// Stores the base list in ascending key order, keeping only what the
/// writer writes (`CSvBits` ≠ 0, value ≠ 0).
/// A value the writer would clamp is refused (it would not read back).
pub fn store_stats(b: &mut Body, m: &BTreeMap<(u16, u16), i32>, t: &dyn SaveTables) -> Result<()> {
    let mut out = Vec::new();
    for (&(id, layer), &value) in m {
        let Some(col) = t.stat_save(id).filter(|c| c.bits != 0) else {
            continue;
        };
        if value == 0 {
            continue;
        }
        let n = u32::from(col.bits);
        let raw = clamp_stat(value, col);
        let back = if n < 32 && col.signed {
            let sh = 32 - n;
            ((raw << sh) as i32) >> sh
        } else {
            raw as i32
        };
        if back != value {
            bail!("stat {id} = {value} does not fit CSvBits {n} (written as {back}, d2s.md §7.1 rule 2.3)");
        }
        if col.param == 0 && layer != 0 {
            bail!("stat {id} has CSvParam 0: layer {layer} is not saved");
        }
        out.push(StatEntry { id, layer, value });
    }
    b.stats = Stats::Bits(out);
    Ok(())
}

// --------------------------------------------------------------- build

fn now() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as u32)
}

/// A full (non-stub) save from the edit flags (`d2s-tool new`).
pub fn new_save(e: &Edits, t: &Tables) -> Result<D2s> {
    let name = e.name.as_deref().context("--name is required")?;
    let class = e.class.context("--class is required")?;
    let level = e.level.unwrap_or(1);
    let time = e.time.unwrap_or_else(now);
    // Appearance bytes: no equipped item → 32 × 0xFF (the default).
    let mut h = Header {
        create_time: time,
        save_time: time,
        map_seed: e.map_seed.unwrap_or(time),
        ..Header::default()
    };
    h.set_name(name.as_bytes())
        .ok_or_else(|| anyhow!("name {name:?}: at most 15 bytes, no NUL"))?;
    h.class = class;
    // Header +0x2A: the largest class skill count; the writer writes
    // count[class] bytes and the reader reads +0x2A, so a
    // class whose count differs would not frame.
    let max = t.max_skill_count();
    let count = t.class_skills(class).len();
    if max == 0 || max > 255 || count != max {
        bail!(
            "class {class} has {count} skills, the largest class has {max}: \
             the skills section would not frame (d2s.md §7.2 rules 1–2)"
        );
    }
    h.skill_count = max as u8;
    // Status 0x20 for every expansion character, always for class 5 / 6.
    let expansion = e.expansion || matches!(class, 5 | 6);
    h.status = if expansion { status::EXPANSION } else { 0 }
        | if e.hardcore == Some(true) {
            status::HARDCORE
        } else {
            0
        };
    let mut body = Body {
        quests: Quests::default(),
        waypoints: Waypoints::default(),
        npcs: Npcs::default(),
        stats: Stats::Bits(Vec::new()),
        skills: vec![0; max],
        items: Vec::new(),
        corpses: Vec::new(),
        hireling_items: expansion.then_some(None),
        golem: expansion.then(Golem::default),
        trailing: Vec::new(),
    };
    let base: BTreeMap<(u16, u16), i32> = match &t.vitals {
        Some(v) => creation_stats(v, class, level)?
            .into_iter()
            .map(|(id, v)| ((id, 0), v))
            .collect(),
        None => {
            // No experience table: the level is set and experience must
            // come from `--stat 13=`.
            if level > 1 && !e.stats.iter().any(|&(id, _)| id == sid::EXPERIENCE) {
                bail!(
                    "no charstats / experience table: level {level} needs --stat 13=<experience>"
                );
            }
            BTreeMap::from([((sid::LEVEL, 0), level as i32)])
        }
    };
    // Stored as is here; `apply` checks the list after the overrides.
    body.stats = Stats::Bits(
        base.iter()
            .map(|(&(id, layer), &value)| StatEntry { id, layer, value })
            .collect(),
    );
    let mut save = D2s {
        header: h,
        body: Some(body),
    };
    let e2 = Edits {
        level: None,
        act: Some(e.act.unwrap_or(0)),
        difficulty: Some(e.difficulty.unwrap_or(0)),
        ..e.clone()
    };
    apply(&mut save, &e2, t)?;
    Ok(save)
}

/// Applies the edit flags to a full save (`d2s-tool set`, and the second
/// half of `new`).
pub fn apply(save: &mut D2s, e: &Edits, t: &Tables) -> Result<()> {
    let h = &mut save.header;
    let b = save
        .body
        .as_mut()
        .context("the file is the 335-byte new-character stub (d2s.md §2.6): nothing to edit")?;
    if let Some(name) = &e.name {
        h.set_name(name.as_bytes())
            .ok_or_else(|| anyhow!("name {name:?}: at most 15 bytes, no NUL"))?;
    }
    if let Some(c) = e.class {
        if c != h.class {
            bail!("changing the class of a save is not supported (skills and items depend on it)");
        }
    }
    if e.expansion && h.status & status::EXPANSION == 0 {
        // An expansion character has `jf` and `kf`.
        h.status |= status::EXPANSION;
        b.hireling_items.get_or_insert(None);
        b.golem.get_or_insert_with(Golem::default);
    }
    if let Some((id, name_index, seed, experience)) = e.merc {
        if seed == 0 && name_index == 0 && experience == 0 {
            bail!("--merc: seed, name index and experience all 0 read as no hireling (d2s.md §2.5 rule 2)");
        }
        h.hireling = d2_formats::d2s::Hireling {
            flags: 0,
            seed,
            name_index,
            id,
            experience,
            rest: [0; 16],
        };
        // An expansion save lists the hireling's items (none).
        if h.status & status::EXPANSION != 0 {
            b.hireling_items = Some(Some(Vec::new()));
        }
    }
    match e.hardcore {
        Some(true) => h.status |= status::HARDCORE,
        Some(false) => h.status &= !status::HARDCORE,
        None => {}
    }
    let expansion = h.status & status::EXPANSION != 0;

    // Stats.
    let mut m = stat_map(b)?;
    if let Some(l) = e.level {
        let v = t
            .vitals
            .as_ref()
            .context("--level needs the experience table")?;
        m.insert((sid::LEVEL, 0), l as i32);
        m.insert((sid::EXPERIENCE, 0), exp_for_level(v, h.class, l)? as i32);
    }
    for &(id, v) in &e.stats {
        match t.stat_save(id) {
            Some(c) if c.bits != 0 => {}
            Some(_) => bail!("stat {id} has CSvBits 0: never written (d2s.md §7.1 rule 4)"),
            None => bail!("stat {id}: no itemstatcost row"),
        }
        m.insert((id, 0), v);
    }
    if let Some(g) = e.gold {
        m.insert((sid::GOLD, 0), g);
    }
    // Out-of-range gold is set to 0 on load; refuse it.
    let level = m.get(&(sid::LEVEL, 0)).copied().unwrap_or(0);
    let gold = m.get(&(sid::GOLD, 0)).copied().unwrap_or(0);
    if gold < 0 || i64::from(gold) > i64::from(level) * 10_000 {
        bail!("gold {gold} is outside 0..=level × 10,000 ({}): the loader sets it to 0 (d2s.md §9 rule 4)", i64::from(level) * 10_000);
    }
    let bank = m.get(&(sid::GOLDBANK, 0)).copied().unwrap_or(0);
    if !(0..=STASH_LIMIT).contains(&bank) {
        bail!("stash gold {bank} is outside 0..=2,500,000: the loader sets it to 0 (d2s.md §9 rule 4)");
    }
    store_stats(b, &m, t)?;
    // Header +0x2B: stat 12, low byte.
    h.level = level as u8;

    // Skills: byte k = the base level of the class list's k-th skill.
    let n = b.skills.len();
    if let Some(l) = e.all_skills {
        b.skills.fill(l);
    }
    for &(i, l) in &e.skills {
        if i >= n {
            bail!("skill index {i}: the class list has {n} entries");
        }
        b.skills[i] = l;
    }

    // Mouse skills: slot 0 left, slot 1 right.
    if let Some(sk) = e.left_skill {
        h.mouse[0] = Slot::encode(sk, true, e.left_item)
            .ok_or_else(|| anyhow!("--left-skill {sk}: too large"))?;
    }
    if let Some(sk) = e.right_skill {
        h.mouse[1] = Slot::encode(sk, false, e.right_item)
            .ok_or_else(|| anyhow!("--right-skill {sk}: too large"))?;
    }

    // Quests.
    if let Some(q) = &e.quests {
        apply_quests(&mut b.quests, q, expansion)?;
    }
    // Waypoints.
    if let Some(w) = &e.waypoints {
        apply_waypoints(&mut b.waypoints, w, t)?;
    }
    // Progression (status bits 8–12 and their thresholds).
    if let Some(d) = e.unlocked {
        let p: u16 = match (d, expansion) {
            (0, _) => 0,
            (1, true) => 5,
            (1, false) => 4,
            (_, true) => 10,
            (_, false) => 8,
        };
        h.status = (h.status & !0x1F00) | (p << 8);
    }
    // Town byte (header +0xA8): [difficulty] = act | 0x80, the others 0.
    if e.act.is_some() || e.difficulty.is_some() {
        let act = e.act.unwrap_or(0);
        let d = e.difficulty.unwrap_or(0);
        if act > 4 || d > 2 {
            bail!("town act 0..4, difficulty 0..2");
        }
        let p = status::progression(h.status);
        let need = match (d, expansion) {
            (0, _) => 0,
            (1, true) => 5,
            (1, false) => 4,
            (_, true) => 10,
            (_, false) => 8,
        };
        if p < need && !e.allow_locked {
            bail!("difficulty {d} is not unlocked (progression {p} < {need}, d2s.md §2.2 rule 5.4): add --difficulty-unlocked");
        }
        h.towns = [0; 3];
        h.towns[usize::from(d)] = act | 0x80;
    }
    if let Some(time) = e.time {
        h.save_time = time;
    }

    for &(n, skill, item) in &e.hotkeys {
        let slot = Slot::encode(skill, false, item).context("--hotkey: skill id above 0x7FFF")?;
        match n {
            0..=15 => h.hotkeys[n] = slot,
            16..=19 => h.mouse[n - 16] = slot,
            _ => bail!("--hotkey slot {n}: 0..=15 hotkeys, 16..=19 mouse slots"),
        }
    }

    // Items.
    if !e.items.is_empty() || e.golem.is_some() {
        let mut name = [0u8; 16];
        name.copy_from_slice(&h.name);
        let owner = Owner {
            class: h.class,
            name,
            level,
            hardcore: h.status & status::HARDCORE != 0,
            expansion,
        };
        let mut pages = Pages::new();
        for it in &b.items {
            pages.take_existing(t, &owner, it)?;
        }
        let mut game = ToolGame::new(e.seed.unwrap_or(1), expansion);
        for s in &e.items {
            let entry = make_item(t, &mut game, &owner, &mut pages, s)?;
            b.items.push(entry);
        }
        if let Some(s) = &e.golem {
            anyhow::ensure!(expansion, "--golem needs --expansion (the kf section)");
            let entry = make_item(t, &mut game, &owner, &mut Pages::new(), s)?;
            b.golem = Some(Golem {
                flag: 1,
                item: Some(entry),
            });
        }
    }
    Ok(())
}

/// The act transitions, in order: (slot, bit) sets of acts 1–4 (bit 13,
/// cleared on load, is left out).
const ACT_SETS: [&[(usize, u8)]; 4] = [
    &[(7, 0)],
    &[(10, 0), (15, 0)],
    &[(18, 0), (23, 0)],
    &[(28, 0)],
];

fn set_bit(rec: &mut [u8; 96], slot: usize, bit: u8) {
    let n = slot * 16 + usize::from(bit);
    rec[n >> 3] |= 1 << (n & 7);
}

fn apply_quests(q: &mut Quests, spec: &QuestSpec, expansion: bool) -> Result<()> {
    match spec {
        QuestSpec::None => q.records = [[0; 96]; 3],
        QuestSpec::All => bail!(
            "--quests all is Pending: a played completion leaves no single bit pattern per quest \
             (world/quests.md §1.8 rule 3, Open question 14); use acts=N (the §8.1 act \
             transitions) or explicit slot.bit items (bit 0 is what every completion reader \
             accepts, §1.8 rule 4)"
        ),
        QuestSpec::Items(items) => {
            for it in items {
                let (diff, sets): (Option<u8>, Vec<(usize, u8)>) = match *it {
                    QuestItem::Acts { diff, n } => {
                        let max = if expansion { 4 } else { 3 };
                        if n > max {
                            bail!("acts={n}: at most {max} act transitions (world/quests.md §8.1; Act IV's needs an expansion game)");
                        }
                        (diff, ACT_SETS[..usize::from(n)].concat())
                    }
                    QuestItem::Bit { diff, slot, bit } => (diff, vec![(usize::from(slot), bit)]),
                };
                for d in 0..3u8 {
                    if diff.is_some_and(|x| x != d) {
                        continue;
                    }
                    for &(slot, bit) in &sets {
                        set_bit(&mut q.records[usize::from(d)], slot, bit);
                    }
                }
            }
        }
    }
    Ok(())
}

fn apply_waypoints(w: &mut Waypoints, spec: &WpSpec, t: &Tables) -> Result<()> {
    let set = |w: &mut Waypoints, d: usize, n: u8| {
        if let Some((byte, mask)) = Waypoints::bit(n) {
            w.records[d][byte] |= mask;
        }
    };
    match spec {
        WpSpec::None => w.records = Waypoints::default().records,
        WpSpec::All => {
            for d in 0..3 {
                for &n in &t.waypoint_indices {
                    set(w, d, n);
                }
            }
        }
        WpSpec::List(l) => {
            for &(diff, r) in l {
                let n = match r {
                    WpRef::Index(n) => n,
                    WpRef::Level(id) => match t.level_waypoint.get(usize::from(id)) {
                        Some(&n) if n != 255 && Waypoints::bit(n).is_some() => n,
                        Some(_) => bail!("level {id} has no waypoint (levels Waypoint 255)"),
                        None => bail!("level {id}: no such levels row"),
                    },
                };
                for d in 0..3u8 {
                    if diff.is_none_or(|x| x == d) {
                        set(w, usize::from(d), n);
                    }
                }
            }
        }
    }
    Ok(())
}

/// The record offsets of an entry's item and its children, depth first
/// (each child's entry follows its parent's own padded stream).
fn record_offsets(e: &SaveEntry, at: usize, out: &mut Vec<usize>) {
    out.push(at);
    let own = e.len - e.children.iter().map(|c| c.len).sum::<usize>();
    let mut o = at + own;
    for c in &e.children {
        record_offsets(c, o, out);
        o += c.len;
    }
}

/// Item flag 0x2000 cleared in every record of `entry`, children
/// included. Returns whether the entry has an equipped
/// (mode 1) top-level item.
fn clear_instore(entry: &mut ItemEntry, t: &Tables) -> Result<bool> {
    let e = t
        .decode_entry(&entry.bytes)
        .map_err(|err| anyhow!("item entry: {err}"))?;
    let mut offs = Vec::new();
    record_offsets(&e, 0, &mut offs);
    for at in offs {
        let f = entry
            .record_flags(at)
            .with_context(|| format!("item record at +{at} has no JM marker"))?;
        entry
            .set_record_flags(at, f & !ITEM_FLAG_INSTORE)
            .expect("checked above");
    }
    Ok(e.item.mode == 1)
}

/// What the game's next save of a loaded file holds where it differs from
/// the file without any change in play: every item record (player list,
/// corpse, hireling and golem item, children) without flag 0x2000, and
/// the appearance bytes rebuilt: 32 × 0xFF when no player item is
/// equipped. With an equipped item the bytes are kept (their mapping is
/// not known yet) and a note says so.
/// Returns the notes.
pub fn resave(save: &mut D2s, t: &Tables) -> Result<Vec<String>> {
    let mut notes = Vec::new();
    let Some(b) = save.body.as_mut() else {
        return Ok(notes);
    };
    let mut equipped = false;
    for it in &mut b.items {
        equipped |= clear_instore(it, t)?;
    }
    for c in &mut b.corpses {
        for it in &mut c.items {
            clear_instore(it, t)?;
        }
    }
    if let Some(Some(list)) = &mut b.hireling_items {
        for it in list {
            clear_instore(it, t)?;
        }
    }
    if let Some(Golem { item: Some(it), .. }) = &mut b.golem {
        clear_instore(it, t)?;
    }
    if equipped {
        notes.push(
            "appearance bytes +0x88..+0xA7 kept: an item is equipped and the per-item mapping \
             is not specified (d2s.md §2.8 rule 3, Open question 17)"
                .into(),
        );
    } else {
        save.header.reset_appearance();
    }
    Ok(notes)
}
