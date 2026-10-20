//! The synthetic rows. Every value here is made up for this fixture:
//! class, item, monster and level names, codes, numbers. Only the column
//! names (from the schema) and the counts and value ranges the load checks
//! demand are fixed by the game's rules.
//!
//! The world: seven classes, two acts of four levels, a handful of items
//! (two blades, a robe, a vest, a potion, gold), two monsters and a town
//! NPC, two skills with a missile and a state, a few stats.

use crate::animdata::Anim;
use crate::synth::{StringSet, TableSet};

/// The seven made-up class names, in class-index order (charstats rows).
pub const CLASSES: [&str; 7] = [
    "Archer", "Mage", "Summoner", "Knight", "Brute", "Shifter", "Rogue",
];
/// Their playerclass codes.
pub const CLASS_CODES: [&str; 7] = ["cl0", "cl1", "cl2", "cl3", "cl4", "cl5", "cl6"];
/// Made-up stats, in itemstatcost row order.
pub const STATS: [&str; 16] = [
    "strength",
    "energy",
    "dexterity",
    "vitality",
    "statpts",
    "newskills",
    "hitpoints",
    "maxhp",
    "mana",
    "maxmana",
    "stamina",
    "maxstamina",
    "level",
    "experience",
    "gold",
    "goldbank",
];
/// Level names, row order (row 0 is the empty level).
pub const LEVELS: [&str; 8] = [
    "Null",
    "Synth Town",
    "Synth Field",
    "Synth Cave",
    "Synth Keep",
    "Dune Town",
    "Dune Waste",
    "Dune Tomb",
];
/// Experience needed to reach level `l` (≥ 1): `50 · (l − 1) · l`.
pub fn exp_for_level(l: u32) -> u32 {
    50 * (l - 1) * l
}
/// Highest level, per class.
pub const MAX_LEVEL: u32 = 99;
/// Superunique rows: one per hcIdx 0..=65.
pub const SUPERUNIQUES: usize = 66;
/// The DT1 files the lvltypes rows name (`File 1` of rows 0 and 1, `File
/// 2` of both), as table strings; [`crate::drlg`] writes them.
pub const FLOOR_DT1: [&str; 2] = ["Synth1\\Floor.dt1", "Synth2\\Floor.dt1"];
pub const WALL_DT1: &str = "Synth\\Wall.dt1";
/// The DS1 files of the four lvlprest rows (Defs 0–3: town, keep, cave,
/// field) and the lvlsub row, as table strings.
pub const PRESET_DS1: [&str; 4] = [
    "Synth\\Town.ds1",
    "Synth\\Keep.ds1",
    "Synth\\Cave.ds1",
    "Synth\\Field.ds1",
];
pub const SUB_DS1: &str = "Synth\\Sub.ds1";
/// `SizeX` / `SizeY` of every lvlprest row: the stored DS1 size.
pub const PRESET_SIZE: u32 = 8;

fn n(v: impl ToString) -> String {
    v.to_string()
}

/// Fills `t` and returns the strings the rows' `strkey` cells name and
/// the AnimData records.
pub fn fill(t: &mut TableSet) -> (StringSet, Vec<Anim>) {
    lookups(t);
    stats(t);
    skills(t);
    classes(t);
    items(t);
    monsters(t);
    world(t);
    misc(t);
    (strings(), anims())
}

fn lookups(t: &mut TableSet) {
    for code in ["nil ", "bas ", "ext "] {
        t.row("compcode", &[("code", code)]);
    }
    for code in CLASS_CODES {
        t.row("playerclass", &[("code", code)]);
    }
    // Row 0 holds the empty code (empty keys): an empty `itemNloc` links to 0.
    for code in [
        "", "head", "neck", "tors", "rarm", "larm", "rrin", "lrin", "belt", "feet", "glov",
    ] {
        t.row("bodylocs", &[("code", code)]);
    }
    for code in ["armr", "weap", "misc"] {
        t.row("storepage", &[("code", code)]);
    }
    for code in ["none", "fire", "ltng", "cold", "pois", "mag"] {
        t.row("elemtypes", &[("code", code)]);
    }
    for code in ["none", "hth", "blde", "spel"] {
        t.row("hitclass", &[("code", code)]);
    }
    for code in ["whit", "lgry", "dred", "dgrn"] {
        t.row("colors", &[("code", code)]);
    }
    t.row("hiredesc", &[("code", "grd")]);
    for (name, token) in [
        ("Death", "DT"),
        ("Neutral", "NU"),
        ("Walk", "WL"),
        ("GetHit", "GH"),
        ("Attack1", "A1"),
        ("Run", "RN"),
    ] {
        t.row(
            "monmode",
            &[
                ("name", name),
                ("token", token),
                ("code", token),
                ("DT_Dir", "8"),
            ],
        );
    }
    for (name, token) in [
        ("Death", "DT"),
        ("Neutral", "NU"),
        ("Walk", "WL"),
        ("Run", "RN"),
        ("GetHit", "GH"),
        ("TownNeutral", "TN"),
        ("TownWalk", "TW"),
        ("Attack1", "A1"),
    ] {
        t.row(
            "plrmode",
            &[("name", name), ("token", token), ("code", token)],
        );
    }
    for ai in ["Idle", "Wander", "Hunter", "Npc"] {
        t.row("monai", &[("AI", ai)]);
    }
    t.row("monplace", &[("code", "ring")]);
    for code in ["lvl", "blvl", "par1"] {
        t.row("skillcalc", &[("code", code)]);
    }
    for code in ["lvl", "par1"] {
        t.row("misscalc", &[("code", code)]);
    }
    for e in ["none", "hit", "kill"] {
        t.row("events", &[("event", e)]);
    }
    for s in ["none", "synth_hit", "synth_drop", "synth_cast"] {
        t.row("sounds", &[("Sound", s)]);
    }
}

fn stats(t: &mut TableSet) {
    for (i, s) in STATS.iter().enumerate() {
        let bits = if *s == "experience" { "32" } else { "10" };
        t.row(
            "itemstatcost",
            &[
                ("stat", s),
                ("send bits", bits),
                ("save bits", bits),
                ("csvbits", bits),
                ("descpriority", &n(i * 3)),
                // Row 0 `stuff` selects the stat shift.
                ("stuff", if i == 0 { "6" } else { "" }),
                ("saved", if i < 6 { "1" } else { "" }),
            ],
        );
    }
    t.row(
        "properties",
        &[("code", "str"), ("func1", "1"), ("stat1", "strength")],
    );
    t.row(
        "properties",
        &[("code", "hp"), ("func1", "1"), ("stat1", "maxhp")],
    );
    t.row(
        "properties",
        &[("code", "mana"), ("func1", "1"), ("stat1", "maxmana")],
    );
    t.row(
        "properties",
        &[("code", "gold"), ("func1", "1"), ("stat1", "gold")],
    );
    t.row(
        "overlay",
        &[("overlay", "glow"), ("Filename", "glow"), ("Frames", "8")],
    );
    t.row("pettype", &[("pet type", "none")]);
    t.row(
        "pettype",
        &[
            ("pet type", "summon"),
            ("group", "1"),
            ("basemax", "3"),
            ("name", "strSummon"),
        ],
    );
    for (state, stat) in [("none", ""), ("burning", "hitpoints"), ("chilled", "")] {
        t.row(
            "states",
            &[
                ("state", state),
                ("stat", stat),
                ("overlay1", if state == "burning" { "glow" } else { "" }),
            ],
        );
    }
    for (m, range, sub) in [
        ("arrow", "40", ""),
        ("firebolt", "30", "flamelet"),
        ("flamelet", "4", ""),
    ] {
        t.row(
            "missiles",
            &[
                ("Missile", m),
                ("Range", range),
                ("Param1", "2"),
                ("SrvCalc1", "par1+lvl"),
                ("ExplosionMissile", sub),
                ("HitSound", "synth_hit"),
                ("TravelSound", "none"),
            ],
        );
    }
}

fn skills(t: &mut TableSet) {
    for (skill, class, missile, mana, desc) in [
        ("Attack", "", "", "0", "attack"),
        ("Shoot", "cl0", "arrow", "1", "shoot"),
        ("Firebolt", "cl1", "firebolt", "3", "firebolt"),
        ("Summon Wisp", "cl2", "", "5", "summon"),
    ] {
        t.row(
            "skills",
            &[
                ("skill", skill),
                ("charclass", class),
                ("skilldesc", desc),
                ("srvmissile", missile),
                ("mana", mana),
                ("lvlmana", "1"),
                ("maxlvl", "20"),
                ("reqlevel", if class.is_empty() { "0" } else { "1" }),
                ("Param1", "10"),
                ("calc1", "lvl*par1"),
                ("MinDam", "1"),
                ("MaxDam", "4"),
                ("InGame", "1"),
                ("InTown", if class.is_empty() { "1" } else { "" }),
                ("range", if missile.is_empty() { "h2h" } else { "rng" }),
            ],
        );
        t.row(
            "skilldesc",
            &[
                ("skilldesc", desc),
                ("skillpage", if class.is_empty() { "0" } else { "1" }),
                ("str name", skill),
                ("ddam calc1", "lvl*2"),
                ("p1dmelem", if missile == "firebolt" { "fire" } else { "" }),
            ],
        );
    }
}

fn classes(t: &mut TableSet) {
    for (i, class) in CLASSES.iter().enumerate() {
        let i8 = i as u32;
        t.row(
            "charstats",
            &[
                ("class", class),
                ("str", &n(15 + i8 * 2)),
                ("dex", &n(25 - i8)),
                ("vit", &n(20 + i8)),
                ("int", &n(10 + (i8 * 5) % 17)),
                ("stamina", "80"),
                ("hpadd", "30"),
                ("ManaRegen", "30"),
                ("ToHitFactor", &n(5 * i8)),
                ("WalkVelocity", "6"),
                ("RunVelocity", "9"),
                ("RunDrain", "20"),
                ("LifePerLevel", "8"),
                ("StaminaPerLevel", "4"),
                ("ManaPerLevel", "6"),
                ("LifePerVitality", "12"),
                ("StaminaPerVitality", "4"),
                ("ManaPerMagic", "8"),
                ("BlockFactor", "20"),
                ("basewclass", "hth"),
                ("StatPerLevel", "5"),
                ("StrAllSkills", "strAllSkills"),
                (
                    "StartSkill",
                    ["Shoot", "Firebolt", "Summon Wisp", "Shoot", "", "", ""][i],
                ),
                ("Skill 1", "Attack"),
                ("item1", "sb1"),
                ("item1loc", "rarm"),
                ("item1count", "1"),
                ("item2", "pt1"),
                ("item2count", "2"),
            ],
        );
        t.row("plrtype", &[("name", class), ("token", &format!("C{i}"))]);
    }
    // Row 0: the level cap per class; row L + 1: the experience to reach
    // level L + 1.
    let mut row0: Vec<(&str, String)> = Vec::new();
    for c in [
        "Amazon",
        "Sorceress",
        "Necromancer",
        "Paladin",
        "Barbarian",
        "Druid",
        "Assassin",
    ] {
        row0.push((c, n(MAX_LEVEL)));
    }
    row0.push(("ExpRatio", n(10)));
    let cells: Vec<(&str, &str)> = row0.iter().map(|(c, v)| (*c, v.as_str())).collect();
    t.row("experience", &cells);
    for l in 1..=MAX_LEVEL + 1 {
        let v = exp_for_level(l);
        let v = n(v);
        let ratio = n(1024 - l.min(80) * 8);
        t.row(
            "experience",
            &[
                ("Amazon", &v),
                ("Sorceress", &v),
                ("Necromancer", &v),
                ("Paladin", &v),
                ("Barbarian", &v),
                ("Druid", &v),
                ("Assassin", &v),
                ("ExpRatio", &ratio),
            ],
        );
    }
    t.row(
        "arena",
        &[
            ("Suicide", "-10"),
            ("PlayerKill", "50"),
            ("MonsterKill", "1"),
        ],
    );
    t.row(
        "chartemplate",
        &[
            ("Name", "Fresh Archer"),
            ("class", "cl0"),
            ("level", "1"),
            ("str", "20"),
        ],
    );
    t.row(
        "chartemplate",
        &[
            ("Name", "Seasoned Mage"),
            ("class", "cl1"),
            ("level", "30"),
            ("int", "90"),
        ],
    );
}

fn items(t: &mut TableSet) {
    // Item types: `treasureclass` 1 builds 32 automatic TCs each.
    // Row 0 has the empty code, so an empty
    // `equiv2` cell links to it (0) instead of missing (−1), which the
    // equivalence walk would reject.
    t.row("itemtypes", &[("code", "")]);
    for (code, equiv, store, tc, body) in [
        ("weap", "", "weap", "1", "rarm"),
        ("blad", "weap", "weap", "", "rarm"),
        ("armo", "", "armr", "1", "tors"),
        ("tors", "armo", "armr", "", "tors"),
        ("misc", "", "misc", "", ""),
        ("potn", "misc", "misc", "", ""),
        ("gold", "misc", "misc", "", ""),
    ] {
        t.row(
            "itemtypes",
            &[
                ("code", code),
                ("equiv1", equiv),
                ("storepage", store),
                ("treasureclass", tc),
                ("body", if body.is_empty() { "0" } else { "1" }),
                ("bodyloc1", body),
                ("normal", "1"),
                (
                    "magic",
                    if tc.is_empty() && equiv != "misc" {
                        ""
                    } else {
                        "1"
                    },
                ),
                ("rarity", "3"),
                ("maxsock1", "2"),
            ],
        );
    }
    for (code, flippy, ty, level, min, max, speed) in [
        ("sb1", "Short Blade", "blad", "2", "2", "7", "0"),
        ("lb1", "Long Blade", "blad", "9", "5", "14", "10"),
    ] {
        t.row(
            "weapons",
            &[
                ("code", code),
                ("namestr", code),
                ("type", ty),
                ("level", level),
                ("levelreq", level),
                ("mindam", min),
                ("maxdam", max),
                ("speed", speed),
                ("invwidth", "1"),
                ("invheight", "3"),
                ("durability", "30"),
                ("cost", "40"),
                ("gamble cost", "200"),
                ("normcode", code),
                ("wclass", "1hs"),
                ("2handedwclass", "1hs"),
                ("spawnable", "1"),
                ("rarity", "4"),
                ("dropsound", "synth_drop"),
                ("hit class", "blde"),
                ("stackable", "0"),
                ("flippyfile", flippy),
            ],
        );
    }
    for (code, name, level, ac) in [("ar1", "Robe", "1", "4"), ("ar2", "Vest", "7", "12")] {
        t.row(
            "armor",
            &[
                ("code", code),
                ("namestr", code),
                ("type", "tors"),
                ("level", level),
                ("minac", ac),
                ("maxac", ac),
                ("invwidth", "2"),
                ("invheight", "3"),
                ("durability", "20"),
                ("cost", "60"),
                ("gamble cost", "300"),
                ("normcode", code),
                ("spawnable", "1"),
                ("rarity", "4"),
                ("flippyfile", name),
            ],
        );
    }
    for (code, ty, level) in [("pt1", "potn", "1"), ("gld", "gold", "0")] {
        t.row(
            "misc",
            &[
                ("code", code),
                ("namestr", code),
                ("type", ty),
                ("level", level),
                ("invwidth", "1"),
                ("invheight", "1"),
                ("stackable", if code == "gld" { "1" } else { "0" }),
                ("cost", "5"),
                ("spawnable", "1"),
                ("calc1", "lvl*2"),
            ],
        );
    }
    for (table, name, code, min, max) in [
        ("magicsuffix", "of Vigor", "hp", "5", "10"),
        ("magicsuffix", "of Wealth", "gold", "20", "40"),
        ("magicprefix", "Strong", "str", "1", "3"),
        ("automagic", "Lively", "hp", "1", "2"),
    ] {
        t.row(
            table,
            &[
                ("name", name),
                ("spawnable", "1"),
                ("level", "1"),
                ("frequency", "1"),
                ("itype1", "weap"),
                ("itype2", "armo"),
                ("mod1code", code),
                ("mod1min", min),
                ("mod1max", max),
            ],
        );
    }
    t.row("raresuffix", &[("name", "fang"), ("itype1", "weap")]);
    t.row("rareprefix", &[("name", "Grim"), ("itype1", "armo")]);
    t.row(
        "uniqueitems",
        &[
            ("index", "Edge of Dawn"),
            ("enabled", "1"),
            ("code", "sb1"),
            ("rarity", "1"),
            ("lvl", "3"),
            ("prop1", "str"),
            ("min1", "2"),
            ("max1", "4"),
        ],
    );
    t.row(
        "sets",
        &[
            ("index", "Wanderer"),
            ("name", "Wanderer"),
            ("pcode2a", "hp"),
            ("pmin2a", "10"),
            ("pmax2a", "10"),
        ],
    );
    for (index, item) in [("Wanderer's Edge", "sb1"), ("Wanderer's Robe", "ar1")] {
        t.row(
            "setitems",
            &[
                ("index", index),
                ("item", item),
                ("set", "Wanderer"),
                ("lvl", "4"),
                ("prop1", "mana"),
                ("min1", "5"),
                ("max1", "5"),
            ],
        );
    }
    t.row(
        "gems",
        &[
            ("name", "Flawed Shard"),
            ("letter", "S"),
            ("code", "pt1"),
            ("nummods", "1"),
            ("weaponmod1code", "str"),
            ("weaponmod1min", "1"),
            ("weaponmod1max", "1"),
        ],
    );
    t.row("books", &[("name", "pt1")]);
    t.row(
        "qualityitems",
        &[
            ("nummods", "1"),
            ("mod1code", "hp"),
            ("mod1min", "1"),
            ("mod1max", "3"),
            ("armor", "1"),
        ],
    );
    for name in ["Crude", "Cracked"] {
        t.row("lowqualityitems", &[("Name", name)]);
    }
    t.row(
        "runes",
        &[
            ("name", "Runeword1"),
            ("rune name", "Echo"),
            ("complete", "1"),
            ("itype1", "weap"),
            ("rune1", "pt1"),
            ("t1code1", "str"),
            ("t1min1", "3"),
            ("t1max1", "3"),
        ],
    );
    for (version, uber, class) in [
        ("0", "0", "0"),
        ("1", "0", "0"),
        ("1", "1", "0"),
        ("1", "0", "1"),
    ] {
        t.row(
            "itemratio",
            &[
                ("Version", version),
                ("Uber", uber),
                ("Class Specific", class),
                ("Unique", "400"),
                ("UniqueDivisor", "1"),
                ("UniqueMin", "6400"),
                ("Rare", "100"),
                ("RareDivisor", "2"),
                ("RareMin", "3200"),
                ("Set", "300"),
                ("SetDivisor", "2"),
                ("SetMin", "5600"),
                ("Magic", "34"),
                ("MagicDivisor", "3"),
                ("MagicMin", "192"),
                ("HiQuality", "12"),
                ("HiQualityDivisor", "8"),
                ("Normal", "2"),
                ("NormalDivisor", "2"),
            ],
        );
    }
    for code in ["sb1", "ar1", "ar2"] {
        t.row("gamble", &[("code", code)]);
    }
    for name in ["the Grim", "Shade"] {
        t.row("uniquetitle", &[("Name", name)]);
    }
    t.row("uniqueprefix", &[("Name", "Ash")]);
    t.row("uniquesuffix", &[("Name", "bane")]);
    t.row("uniqueappellation", &[("Name", "the Cold")]);
    t.row(
        "treasureclassex",
        &[
            ("treasure class", "Gold Pile"),
            ("picks", "1"),
            ("item1", "gld"),
            ("prob1", "1"),
        ],
    );
    t.row(
        "treasureclassex",
        &[
            ("treasure class", "Synth Act 1"),
            ("level", "3"),
            ("picks", "1"),
            ("nodrop", "10"),
            ("item1", "Gold Pile"),
            ("prob1", "5"),
            ("item2", "weap3"),
            ("prob2", "3"),
            ("item3", "armo6"),
            ("prob3", "3"),
            ("item4", "pt1"),
            ("prob4", "4"),
        ],
    );
    t.row(
        "treasureclassex",
        &[
            ("treasure class", "Synth Boss"),
            ("picks", "3"),
            ("unique", "50"),
            ("item1", "Synth Act 1"),
            ("prob1", "1"),
        ],
    );
    t.row(
        "cubemain",
        &[
            ("enabled", "1"),
            ("numinputs", "2"),
            ("input 1", "pt1,qty=2"),
            ("output", "pt1"),
            ("version", "0"),
        ],
    );
}

fn monsters(t: &mut TableSet) {
    for ty in ["beast", "undead", "human"] {
        t.row("montype", &[("type", ty), ("strsing", ty), ("strplur", ty)]);
    }
    for id in ["beast1", "ghoul1", "keeper"] {
        t.row(
            "monstats2",
            &[
                ("Id", id),
                ("SizeX", "2"),
                ("SizeY", "2"),
                ("MeleeRng", "1"),
                ("BaseW", "hth"),
                ("HDv", "nil"),
                ("TRv", "nil,bas"),
                ("HD", "1"),
                ("TR", "1"),
                ("TotalPieces", "2"),
                ("mNU", "1"),
                ("mWL", "1"),
                ("mDT", "1"),
                ("mGH", "1"),
                ("mA1", "1"),
                ("dNU", "8"),
                ("dWL", "8"),
                ("dDT", "8"),
                ("dGH", "8"),
                ("dA1", "8"),
                ("isSel", "1"),
                ("isAtt", "1"),
                ("light", "1"),
            ],
        );
    }
    t.row(
        "monprop",
        &[
            ("Id", "beast1"),
            ("prop1", "hp"),
            ("chance1", "50"),
            ("min1", "2"),
            ("max1", "4"),
        ],
    );
    for id in ["beast", "ghoul"] {
        t.row(
            "monsounds",
            &[
                ("Id", id),
                ("Attack1", "synth_hit"),
                ("HitSound", "synth_hit"),
                ("CvtMo1", "NU"),
            ],
        );
    }
    t.row(
        "monseq",
        &[
            ("sequence", "ghoulrise"),
            ("mode", "A1"),
            ("frame", "0"),
            ("dir", "0"),
            ("event", "1"),
        ],
    );
    for (id, ty, ai, sound, level, hp, exp, npc) in [
        ("beast1", "beast", "Hunter", "beast", "2", "6", "40", ""),
        ("ghoul1", "undead", "Wander", "ghoul", "5", "14", "90", ""),
        ("keeper", "human", "Npc", "", "1", "200", "0", "1"),
    ] {
        t.row(
            "monstats",
            &[
                ("Id", id),
                ("BaseId", id),
                ("NameStr", id),
                ("Code", &id[..2]),
                ("MonSound", sound),
                ("MonStatsEx", id),
                ("MonType", ty),
                ("MonProp", if id == "beast1" { "beast1" } else { "" }),
                ("AI", ai),
                ("Rarity", if npc.is_empty() { "2" } else { "" }),
                ("MinGrp", "1"),
                ("MaxGrp", "3"),
                ("Velocity", "5"),
                ("Run", "8"),
                ("enabled", "1"),
                ("isMelee", "1"),
                ("killable", if npc.is_empty() { "1" } else { "" }),
                ("npc", npc),
                ("interact", npc),
                ("inTown", npc),
                ("lUndead", if ty == "undead" { "1" } else { "" }),
                (
                    "TreasureClass1",
                    if npc.is_empty() { "Synth Act 1" } else { "" },
                ),
                (
                    "TreasureClass2",
                    if npc.is_empty() { "Synth Boss" } else { "" },
                ),
                ("threat", "4"),
                ("aidel", "5"),
                ("aip1", "40"),
                ("Level", level),
                ("Level(N)", &n(level.parse::<u32>().unwrap() + 30)),
                ("Level(H)", &n(level.parse::<u32>().unwrap() + 60)),
                ("MinHP", hp),
                ("MaxHP", &n(hp.parse::<u32>().unwrap() * 2)),
                ("AC", "5"),
                ("Exp", exp),
                ("A1TH", "30"),
                ("A1MinD", "1"),
                ("A1MaxD", "3"),
                ("ResFi", if ty == "undead" { "25" } else { "" }),
                ("Skill1", if id == "ghoul1" { "Attack" } else { "" }),
                ("Sk1mode", if id == "ghoul1" { "A1" } else { "" }),
                ("Sk1lvl", if id == "ghoul1" { "1" } else { "" }),
                ("SplEndDeath", ""),
            ],
        );
    }
    t.row("monumod", &[("uniquemod", "none"), ("enabled", "0")]);
    for (m, champ) in [("hpmultiply", "1"), ("fast", "1"), ("cold", "0")] {
        t.row(
            "monumod",
            &[
                ("uniquemod", m),
                ("enabled", "1"),
                ("champion", champ),
                ("cpick", "1"),
                ("upick", "1"),
                ("exclude1", if m == "cold" { "undead" } else { "" }),
            ],
        );
    }
    // One superunique per hcIdx 0..=65.
    for i in 0..SUPERUNIQUES {
        let name = format!("Warden {i}");
        let class = if i % 2 == 0 { "beast1" } else { "ghoul1" };
        let hc = n(i);
        t.row(
            "superuniques",
            &[
                ("Superunique", &name),
                ("Name", &name),
                ("Class", class),
                ("hcIdx", &hc),
                ("Mod1", "1"),
                ("MinGrp", "2"),
                ("MaxGrp", "4"),
                ("TC", "Synth Boss"),
            ],
        );
    }
    t.row("monpreset", &[("Act", "1"), ("Place", "beast1")]);
    t.row("monpreset", &[("Act", "1"), ("Place", "Warden 0")]);
    t.row("monpreset", &[("Act", "2"), ("Place", "ring")]);
    t.row(
        "hireling",
        &[
            ("version", "100"),
            ("id", "1"),
            ("class", "2"),
            ("act", "1"),
            ("difficulty", "1"),
            ("seller", "2"),
            ("gold", "150"),
            ("level", "3"),
            ("namefirst", "merc00"),
            ("namelast", "merc03"),
            ("exp/lvl", "100"),
            ("hp", "40"),
            ("hp/lvl", "8"),
            ("hiredesc", "grd"),
            ("skill1", "Shoot"),
            ("mode1", "4"),
            ("chance1", "50"),
            ("level1", "1"),
        ],
    );
    t.row(
        "npc",
        &[
            ("npc", "keeper"),
            ("sell mult", "1024"),
            ("buy mult", "256"),
            ("rep mult", "1024"),
            ("max buy", "5000"),
            ("max buy (N)", "10000"),
            ("max buy (H)", "20000"),
        ],
    );
    t.row(
        "monequip",
        &[
            ("monster", "ghoul1"),
            ("level", "1"),
            ("oninit", "1"),
            ("item1", "sb1"),
            ("loc1", "rarm"),
        ],
    );
    for level in 0..10u32 {
        t.row(
            "monlvl",
            &[
                ("AC", &n(10 + level * 5)),
                ("TH", &n(20 + level * 6)),
                ("HP", &n(100 + level * 20)),
                ("DM", &n(100 + level * 10)),
                ("XP", &n(100 + level * 30)),
                ("L-AC", &n(20 + level * 5)),
                ("L-HP", &n(200 + level * 25)),
            ],
        );
    }
    t.row(
        "monitempercent",
        &[
            ("HeartPercent", "10"),
            ("BodyPartPercent", "10"),
            ("TreasureClassPercent", "70"),
            ("ComponentPercent", "10"),
        ],
    );
}

fn world(t: &mut TableSet) {
    for (i, name) in LEVELS.iter().enumerate() {
        let act = if i >= 5 { "1" } else { "0" };
        let town = i == 1 || i == 5;
        let (drlg, ltype) = match i {
            0 => ("0", "0"),
            3 | 7 => ("1", if i == 3 { "1" } else { "2" }),
            _ => ("2", if i < 5 { "1" } else { "2" }),
        };
        let id = n(i);
        let size = if drlg == "1" { "0" } else { "8" };
        t.row(
            "levels",
            &[
                ("Id", &id),
                ("Act", act),
                ("LevelName", name),
                ("LevelWarp", name),
                ("EntryFile", name),
                (
                    "Waypoint",
                    // Index 0 the act 0 town, 1 the act 1 town, 2 the keep
                    // (the second act 0 waypoint level, so a travel can
                    // leave the town's level).
                    match i {
                        1 => "0",
                        5 => "1",
                        4 => "2",
                        _ => "255",
                    },
                ),
                ("MonLvl1", if town { "" } else { "2" }),
                ("MonDen", if town { "" } else { "500" }),
                ("NumMon", if town { "" } else { "2" }),
                ("mon1", if town { "" } else { "beast1" }),
                ("mon2", if town { "" } else { "ghoul1" }),
                ("nmon1", if town { "" } else { "beast1" }),
                ("umon1", if town { "" } else { "ghoul1" }),
                ("IsInside", if drlg == "1" { "1" } else { "0" }),
                ("DrlgType", drlg),
                ("LevelType", ltype),
                ("SizeX", size),
                ("SizeY", size),
                // The keep (a preset level) is placed away from the town's
                // rooms at (0, 0): a preset level's position is its
                // `OffsetX/Y`, and a room that
                // still contains the cell keeps the unit on a teleport,
                // so overlapping levels
                // would leave a warped player in the town's room.
                ("OffsetX", if i == 4 { "100" } else { "" }),
                ("OffsetY", if i == 4 { "100" } else { "" }),
                ("Vis0", if i == 1 { "2" } else { "0" }),
                ("Warp0", if i == 1 { "0" } else { "-1" }),
                ("Layer", &id),
            ],
        );
    }
    for (file, act) in [(FLOOR_DT1[0], "0"), (FLOOR_DT1[1], "1")] {
        t.row(
            "lvltypes",
            &[("File 1", file), ("File 2", WALL_DT1), ("Act", act)],
        );
    }
    t.row("lvltypes", &[("Act", "0")]);
    let size = n(PRESET_SIZE);
    for (def, level, file) in [
        ("0", "1", PRESET_DS1[0]),
        ("1", "4", PRESET_DS1[1]),
        ("2", "3", PRESET_DS1[2]),
        // The field (level 2, a preset level the town's Vis0 links) has
        // a def so the town's neighbour can be built.
        ("3", "2", PRESET_DS1[3]),
    ] {
        t.row(
            "lvlprest",
            &[
                ("Def", def),
                ("LevelId", level),
                ("Populate", "1"),
                ("SizeX", &size),
                ("SizeY", &size),
                ("Files", "1"),
                ("File1", file),
                ("Dt1Mask", "1"),
            ],
        );
    }
    t.row(
        "lvlwarp",
        &[
            ("Id", "0"),
            ("SelectX", "1"),
            ("SelectY", "1"),
            ("SelectDX", "2"),
            ("SelectDY", "2"),
            ("ExitWalkX", "1"),
            ("ExitWalkY", "3"),
            ("Direction", "b"),
        ],
    );
    t.row(
        "lvlmaze",
        &[
            ("Level", "3"),
            ("Rooms", "6"),
            ("Rooms(N)", "8"),
            ("Rooms(H)", "10"),
            ("SizeX", "8"),
            ("SizeY", "8"),
            ("Merge", "100"),
        ],
    );
    t.row(
        "lvlsub",
        &[
            ("Type", "0"),
            ("File", SUB_DS1),
            ("BordType", "1"),
            ("GridSize", "2"),
            ("Dt1Mask", "1"),
            ("Prob0", "50"),
            ("Trials0", "1"),
            ("Max0", "2"),
        ],
    );
    for (level, tile, cel) in [
        ("1 Town", "fl", "0"),
        ("1 Town", "wl", "1"),
        ("1 Cave", "fl", "2"),
    ] {
        t.row(
            "automap",
            &[
                ("LevelName", level),
                ("TileName", tile),
                ("Cel1", cel),
                ("Cel2", "-1"),
            ],
        );
    }
    for (name, token, op) in [
        ("Waypoint", "W1", "23"),
        ("Chest", "C1", "4"),
        ("Door", "D1", "8"),
    ] {
        t.row(
            "objects",
            &[
                ("Name", name),
                ("Token", token),
                ("SizeX", "2"),
                ("SizeY", "2"),
                ("FrameCnt0", "1"),
                ("FrameCnt1", "15"),
                ("Selectable0", "1"),
                ("OperateRange", "4"),
                ("OperateFn", op),
                ("IsDoor", if name == "Door" { "1" } else { "0" }),
            ],
        );
        t.row("objtype", &[("name", name), ("token", token)]);
    }
    for (name, token) in [("Neutral", "NU"), ("Operating", "OP"), ("Opened", "ON")] {
        t.row("objmode", &[("name", name), ("token", token)]);
    }
    t.row(
        "objgroup",
        &[("ID0", "1"), ("DENSITY0", "4"), ("PROB0", "100")],
    );
    t.row(
        "shrines",
        &[
            ("Code", "1"),
            ("Arg0", "10"),
            ("Duration in frames", "1000"),
            ("rarity", "1"),
            ("view name", "Quiet Shrine"),
            ("niftyphrase", "calm"),
        ],
    );
}

fn misc(t: &mut TableSet) {
    for (i, token) in [
        "HD", "TR", "LG", "RA", "LA", "RH", "LH", "SH", "S1", "S2", "S3", "S4", "S5", "S6", "S7",
        "S8",
    ]
    .iter()
    .enumerate()
    {
        t.row(
            "composit",
            &[("name", &format!("Piece{i}")), ("token", token)],
        );
    }
    for (name, token) in [("Light", "LIT"), ("Medium", "MED"), ("Heavy", "HVY")] {
        t.row("armtype", &[("name", name), ("token", token)]);
    }
    // 32 inventory layouts: 16 pages, two resolutions.
    for i in 0..32u32 {
        let x = 320 + (i % 2) * 80;
        t.row(
            "inventory",
            &[
                ("invLeft", &n(x)),
                ("invRight", &n(x + 320)),
                ("invTop", "0"),
                ("invBottom", "432"),
                ("gridX", "10"),
                ("gridY", if i % 3 == 0 { "8" } else { "4" }),
                ("gridLeft", &n(x + 16)),
                ("gridRight", &n(x + 306)),
                ("gridTop", "254"),
                ("gridBottom", "368"),
                ("gridBoxWidth", "29"),
                ("gridBoxHeight", "29"),
            ],
        );
    }
    // 14 belts: 7 belt types × two resolutions.
    for i in 0..14u32 {
        t.row(
            "belts",
            &[
                ("numboxes", &n(4 * (1 + (i % 7) / 2))),
                ("box1left", "178"),
                ("box1right", "208"),
                ("box1top", "445"),
                ("box1bottom", "475"),
            ],
        );
    }
    for (penalty, mult) in [("0", "0"), ("-40", "5"), ("-100", "10")] {
        t.row(
            "difficultylevels",
            &[
                ("ResistPenalty", penalty),
                ("DeathExpPenalty", mult),
                ("UberCodeOddsNormal", "3"),
                ("UberCodeOddsGood", "5"),
                ("LifeStealDivisor", "8"),
                ("ManaStealDivisor", "8"),
                ("MonsterSkillBonus", mult),
                ("MonsterFreezeDivisor", "1"),
                ("MonsterColdDivisor", "1"),
                ("AiCurseDivisor", "1"),
                ("UniqueDamageBonus", "100"),
                ("ChampionDamageBonus", "100"),
                ("StaticFieldMin", "0"),
                ("GambleRare", "800"),
                ("GambleSet", "200"),
                ("GambleUnique", "100"),
                ("GambleUber", "500"),
                ("GambleUltra", "250"),
            ],
        );
    }
}

/// Every `strkey` key the rows name; base table elements 0, 1, …
fn strings() -> StringSet {
    let mut base: Vec<(String, String)> = vec![("x".into(), "".into())];
    let mut add = |k: &str, v: &str| base.push((k.to_owned(), v.to_owned()));
    for (code, name) in [
        ("sb1", "Short Blade"),
        ("lb1", "Long Blade"),
        ("ar1", "Robe"),
        ("ar2", "Vest"),
        ("pt1", "Potion"),
        ("gld", "Gold"),
    ] {
        add(code, name);
    }
    for (k, v) in [
        ("merc00", "Ada"),
        ("merc01", "Bram"),
        ("merc02", "Cole"),
        ("merc03", "Dara"),
    ] {
        add(k, v);
    }
    for m in ["beast1", "ghoul1", "keeper", "beast", "undead", "human"] {
        add(m, m);
    }
    for l in LEVELS {
        add(l, l);
    }
    for s in [
        "Attack",
        "Shoot",
        "Firebolt",
        "Summon Wisp",
        "strAllSkills",
        "strSummon",
    ] {
        add(s, s);
    }
    for a in [
        "of Vigor",
        "of Wealth",
        "Strong",
        "Lively",
        "fang",
        "Grim",
        "Crude",
        "Cracked",
        "Echo",
        "Wanderer",
        "Edge of Dawn",
        "Wanderer's Edge",
        "Wanderer's Robe",
        "the Grim",
        "Shade",
        "Ash",
        "bane",
        "the Cold",
    ] {
        add(a, a);
    }
    let patch = vec![("patchnote".to_owned(), "Synthetic patch".to_owned())];
    let mut expansion = Vec::new();
    for i in 0..SUPERUNIQUES {
        let k = format!("Warden {i}");
        expansion.push((k.clone(), k));
    }
    StringSet {
        base,
        patch,
        expansion,
    }
}

/// A few animation records; monsters not listed use the default record.
fn anims() -> Vec<Anim> {
    let mut out = Vec::new();
    for (code, frames) in [("BE", 12), ("GH", 16)] {
        for (mode, speed) in [("NU", 256), ("WL", 256), ("A1", 128)] {
            out.push(Anim {
                name: format!("{code}{mode}HTH"),
                frames,
                speed,
                events: if mode == "A1" {
                    vec![(frames as usize / 2, 1)]
                } else {
                    Vec::new()
                },
            });
        }
    }
    out
}
