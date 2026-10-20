//! The command line (hand-parsed: no argument crate in the workspace).

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use d2_formats::d2s::{self, status, D2s};

use crate::dump::dump;
use crate::save::{apply, new_save, parse_class, parse_difficulty, resave, Edits};
use crate::tables::Tables;
use crate::{read_options, round_trip, RoundTrip};

pub const USAGE: &str = "\
d2s-tool: D2RUST character saves (.d2s) for local testing. Tables come from
--game-dir DIR or D2_GAME_DIR (needed for everything but new-stub and stub files).

  d2s-tool new --name N --class C [edit flags] -o OUT.d2s
  d2s-tool new-stub --name N --class C [--hardcore] [--expansion] [--time T] -o OUT.d2s
  d2s-tool dump FILE.d2s [--classic|--expansion] [--items]  (--items: fields and bytes per item)
  d2s-tool check FILE.d2s [--classic|--expansion]
  d2s-tool set IN.d2s -o OUT.d2s [edit flags]

class: ama|sor|nec|pal|bar|dru|ass or 0..6

edit flags:
  --level L                    level (new: creation stats + that level's experience; set: stats 12, 13)
  --expansion                  expansion character (status 0x20, jf/kf sections)
  --hardcore | --softcore      status 0x04
  --stat ID=VALUE              base stat as stored (life/mana/stamina 6-11 in 1/256 points); repeatable
  --gold G                     stat 14 (0..=level*10000)
  --skill INDEX=LEVEL          skill byte INDEX of the class list; repeatable
  --all-skills LEVEL           every skill byte
  --left-skill ID[@N] / --right-skill ID[@N]   the mouse skill (skill id, e.g. 36 Fire Bolt); @N: its owner item, the 1-based position in the item list
  --quests none|all|LIST       LIST: comma-separated [diff:]acts=N or [diff:]SLOT.BIT ('all' is Pending)
  --waypoints none|all|LIST    LIST: comma-separated [diff:]INDEX or [diff:]lv=LEVEL
  --difficulty-unlocked normal|nightmare|hell   progression bits (d2s.md §2.2 rule 5.4)
  --allow-locked               with --act/--difficulty: write the town of a difficulty not unlocked
                               (a file the save loader rejects, d2s.md §10)
  --act A --difficulty D       town byte: act 0..4 of difficulty D (default 0, normal)
  --item CODE[#Q][@X,Y][:PAGE] a normal identified item (page 0 inventory, 3 cube, 4 stash;
                               #Q: quantity Q); repeatable. Options after '/':
                               /q=low|normal|superior|magic|set|rare|unique|crafted
                               /idx=ROW (set/unique row) /ilvl=N /sock=N /dur=N /broken /unid /eth
                               /stat=ID_LAYER_VALUE (up to 3: an extra stat on the item's list)
                               /body=1..12 (equipped; the base must fit the
                               location, or the save loader drops it) /belt=0..15
  --golem CODE[/opts]          the Iron Golem item of the kf section (needs --expansion)
  --hotkey N=SKILL[,ITEM]      hotkey slot N 0..15 (16..19: the mouse slots) with a 1-based item index
  --merc ID,NAME,SEED,EXP      hireling block (d2s.md §2.5): hireling.txt Id, name
                               index, seed, experience; expansion: an empty jf list
  --legacy V                   (new) write a legacy save of version V (0x48..0x5B,
                               formats/d2s-legacy.md): no items, no hireling
  --break KIND                 damage the written file (d2s.md §10): checksum|size|version|magic|
                               quests|waypoints|npcs|stats|skills|items|corpse|hireling|golem|class|newflag|dead|short
  --seed S                     game seed the items' seeds derive from (default 1)
  --map-seed S                 header map seed (new: default = the time)
  --time T                     create/save time (new: default now; set: save time)
";

/// Arguments after the subcommand.
struct Args {
    rest: Vec<String>,
    i: usize,
}

impl Args {
    fn next(&mut self) -> Option<String> {
        let a = self.rest.get(self.i).cloned();
        self.i += 1;
        a
    }

    fn value(&mut self, flag: &str) -> Result<String> {
        self.next().ok_or_else(|| anyhow!("{flag} needs a value"))
    }
}

/// `SKILL[@ITEM]`: a mouse skill and the 1-based position of its owner item
/// in the inventory item list, 0 = none.
fn skill_item(v: &str, what: &str) -> Result<(i32, u16)> {
    match v.split_once('@') {
        Some((sk, it)) => Ok((num(sk, what)?, num(it, what)?)),
        None => Ok((num(v, what)?, 0)),
    }
}

fn num<T: std::str::FromStr>(s: &str, what: &str) -> Result<T> {
    let parsed = if let Some(h) = s.strip_prefix("0x") {
        u64::from_str_radix(h, 16)
            .ok()
            .and_then(|v| v.to_string().parse().ok())
    } else {
        s.parse().ok()
    };
    parsed.ok_or_else(|| anyhow!("{what}: bad number {s:?}"))
}

fn pair<A: std::str::FromStr, B: std::str::FromStr>(s: &str, what: &str) -> Result<(A, B)> {
    let (a, b) = s
        .split_once('=')
        .ok_or_else(|| anyhow!("{what}: want A=B, got {s:?}"))?;
    Ok((num(a, what)?, num(b, what)?))
}

/// Common options.
#[derive(Default)]
struct Common {
    game_dir: Option<PathBuf>,
    out: Option<PathBuf>,
    files: Vec<PathBuf>,
    expansion_read: Option<bool>,
    /// `--break KIND`: damage the written file the way [`break_save`] says.
    break_kind: Option<String>,
    /// `--legacy V`: write the legacy save form.
    legacy: Option<u32>,
}

/// Parses the flags of a subcommand into edits and common options.
fn parse(rest: Vec<String>) -> Result<(Edits, Common)> {
    let mut a = Args { rest, i: 0 };
    let mut e = Edits::default();
    let mut c = Common::default();
    while let Some(f) = a.next() {
        match f.as_str() {
            "--game-dir" => c.game_dir = Some(a.value(&f)?.into()),
            "--legacy" => c.legacy = Some(num(&a.value(&f)?, "--legacy")?),
            "--break" => c.break_kind = Some(a.value(&f)?),
            "-o" | "--out" => c.out = Some(a.value(&f)?.into()),
            "--classic" => c.expansion_read = Some(false),
            "--items" => crate::dump::ITEM_DETAIL.store(true, std::sync::atomic::Ordering::Relaxed),
            "--name" => e.name = Some(a.value(&f)?),
            "--class" => e.class = Some(parse_class(&a.value(&f)?)?),
            "--level" => e.level = Some(num(&a.value(&f)?, "--level")?),
            "--expansion" => {
                e.expansion = true;
                c.expansion_read = Some(true);
            }
            "--hardcore" => e.hardcore = Some(true),
            "--softcore" => e.hardcore = Some(false),
            "--stat" => e.stats.push(pair(&a.value(&f)?, "--stat")?),
            "--skill" => e.skills.push(pair(&a.value(&f)?, "--skill")?),
            "--left-skill" => {
                let v = a.value(&f)?;
                let (sk, it) = skill_item(&v, "--left-skill")?;
                e.left_skill = Some(sk);
                e.left_item = it;
            }
            "--right-skill" => {
                let v = a.value(&f)?;
                let (sk, it) = skill_item(&v, "--right-skill")?;
                e.right_skill = Some(sk);
                e.right_item = it;
            }
            "--all-skills" => e.all_skills = Some(num(&a.value(&f)?, "--all-skills")?),
            "--gold" => e.gold = Some(num(&a.value(&f)?, "--gold")?),
            "--quests" => e.quests = Some(a.value(&f)?.parse()?),
            "--waypoints" => e.waypoints = Some(a.value(&f)?.parse()?),
            "--difficulty-unlocked" => e.unlocked = Some(parse_difficulty(&a.value(&f)?)?),
            "--difficulty" => e.difficulty = Some(parse_difficulty(&a.value(&f)?)?),
            "--act" => e.act = Some(num(&a.value(&f)?, "--act")?),
            "--item" => e.items.push(a.value(&f)?.parse()?),
            "--golem" => e.golem = Some(a.value(&f)?.parse()?),
            "--hotkey" => {
                let v = a.value(&f)?;
                let (n, r) = v
                    .split_once('=')
                    .ok_or_else(|| anyhow!("--hotkey N=SKILL[,ITEM]: {v:?}"))?;
                let (sk, it) = r.split_once(',').unwrap_or((r, "0"));
                e.hotkeys.push((
                    num(n, "--hotkey slot")?,
                    num(sk, "--hotkey skill")?,
                    num(it, "--hotkey item")?,
                ));
            }
            "--merc" => {
                let v = a.value(&f)?;
                let p: Vec<&str> = v.split(',').collect();
                if p.len() != 4 {
                    bail!("--merc ID,NAME,SEED,EXP: {v:?}");
                }
                e.merc = Some((
                    num(p[0], "--merc id")?,
                    num(p[1], "--merc name")?,
                    num(p[2], "--merc seed")?,
                    num(p[3], "--merc exp")?,
                ));
            }
            "--allow-locked" => e.allow_locked = true,
            "--seed" => e.seed = Some(num(&a.value(&f)?, "--seed")?),
            "--map-seed" => e.map_seed = Some(num(&a.value(&f)?, "--map-seed")?),
            "--time" => e.time = Some(num(&a.value(&f)?, "--time")?),
            s if s.starts_with('-') => bail!("unknown flag {s}\n\n{USAGE}"),
            _ => c.files.push(f.into()),
        }
    }
    Ok((e, c))
}

fn tables(c: &Common) -> Result<Tables> {
    let dir = match &c.game_dir {
        Some(d) => d.clone(),
        None => std::env::var_os("D2_GAME_DIR")
            .map(PathBuf::from)
            .context("tables needed: pass --game-dir DIR or set D2_GAME_DIR")?,
    };
    Tables::load_dir(&dir)
}

fn write_out(c: &Common, bytes: &[u8]) -> Result<PathBuf> {
    let p = c.out.clone().context("-o OUT.d2s is required")?;
    let broken;
    let bytes = match &c.break_kind {
        Some(kind) if c.legacy.is_some() => {
            broken = break_legacy(bytes, kind)?;
            &broken[..]
        }
        Some(kind) => {
            broken = break_save(bytes, kind)?;
            &broken[..]
        }
        None => bytes,
    };
    std::fs::write(&p, bytes).with_context(|| format!("writing {}", p.display()))?;
    Ok(p)
}

/// Damages a written legacy save (no checksum to
/// fix): the load result each kind reaches is in brackets. `size` [14],
/// `version` [1], `count` [1], `act` [9], `short` [9], `quests` [2],
/// `waypoints` [3], `npcs` [11], `stats` [4], `skills` [5]; `new` sets the
/// new-character bit, `hardcore` the hardcore bit [20 in a
/// softcore game].
fn break_legacy(bytes: &[u8], kind: &str) -> Result<Vec<u8>> {
    let mut b = bytes.to_vec();
    let q = 0x82usize;
    let w = q + 298;
    let n = w + 80;
    let st = n + 52;
    let set = (0..16)
        .filter(|i| b[st + 2 + (i >> 3)] & (1 << (i & 7)) != 0)
        .count();
    let sk = st + 5 + 4 * set;
    match kind {
        "size" => b[0x20] = 0x80,
        "version" => b[4] = 0x46,
        "count" => b[0x1E] = 17,
        "act" => b[0x58] = 0x15,
        "short" => b.truncate(0x81),
        "quests" => b[q] = 0,
        "waypoints" => b[w] = 0,
        "npcs" => b[n] = 0,
        "stats" => b[st] = 0,
        "skills" => b[sk] = 0,
        "new" => b[0x18] |= 1,
        "hardcore" => b[0x18] |= 4,
        k => bail!("--break {k} (legacy): size|version|count|act|short|quests|waypoints|npcs|stats|skills|new|hardcore"),
    }
    Ok(b)
}

/// Damages a written save so the game refuses it at load: the
/// internal code each kind reaches is in brackets. The checksum is
/// recomputed after every kind but `checksum`, so the damage is the first
/// check to fail.
///
/// `checksum` [6], `size` [5], `version` [7], `magic` (result 9 before the
/// master), `quests` [15], `waypoints` [16], `npcs` [17], `stats` [18]
/// (the section marker's first byte zeroed); `skills` [19], `items` [20],
/// `corpse` [21], `hireling` [22], `golem` [23] likewise (a save with no
/// corpse items, hireling or golem).
fn break_save(bytes: &[u8], kind: &str) -> Result<Vec<u8>> {
    let mut b = bytes.to_vec();
    let marker = |b: &mut Vec<u8>, at: usize| -> Result<()> {
        *b.get_mut(at)
            .context("file shorter than the marker offset")? = 0;
        Ok(())
    };
    match kind {
        "checksum" => {
            b[0x0C] ^= 1;
            return Ok(b);
        }
        "size" => {
            let n = u32::from_le_bytes(b[8..12].try_into()?).wrapping_add(1);
            b[8..12].copy_from_slice(&n.to_le_bytes());
        }
        "version" => b[4..8].copy_from_slice(&0x61u32.to_le_bytes()),
        "magic" => b[0] ^= 1,
        "quests" => marker(&mut b, 0x14F)?,
        "waypoints" => marker(&mut b, 0x279)?,
        "npcs" => marker(&mut b, 0x2C9)?,
        "stats" => marker(&mut b, 0x2FD)?,
        // Header-level refusals (the class byte +0x28, the status word
        // +0x24): class > 7 [4], the new-character flag with more data [2],
        // a dead hardcore character [10]; `short` cuts the file inside the
        // header [4].
        "class" => b[0x28] = 8,
        "newflag" => b[0x24] |= 0x01,
        "dead" => b[0x24] |= 0x0C,
        "short" => {
            b.truncate(0x100);
            let n = b.len() as u32;
            b[8..12].copy_from_slice(&n.to_le_bytes());
        }
        // The skills section is 2 + 30 bytes and the item list's `JM` follows
        // it; the trailer of a save without corpse items, hireling or golem
        // is `JM 00 00` `jf` `kf g` (the section order).
        "skills" | "items" => {
            let at = (0x2FD..b.len().saturating_sub(34))
                .find(|&i| &b[i..i + 2] == b"if" && &b[i + 32..i + 34] == b"JM")
                .context("no skills section found")?;
            marker(&mut b, if kind == "skills" { at } else { at + 32 })?
        }
        "corpse" => {
            let at = b.len().checked_sub(9).context("short file")?;
            marker(&mut b, at)?
        }
        "hireling" => {
            let at = b.len().checked_sub(5).context("short file")?;
            marker(&mut b, at)?
        }
        "golem" => {
            let at = b.len().checked_sub(3).context("short file")?;
            marker(&mut b, at)?
        }
        k => bail!("--break {k}: checksum|size|version|magic|quests|waypoints|npcs|stats|skills|items|corpse|hireling|golem|class|newflag|dead|short"),
    }
    b[0x0C..0x10].fill(0);
    let sum = d2s::checksum(&b);
    b[0x0C..0x10].copy_from_slice(&sum.to_le_bytes());
    Ok(b)
}

fn one_file(c: &Common) -> Result<&Path> {
    match c.files.as_slice() {
        [f] => Ok(f),
        _ => bail!("want exactly one input file\n\n{USAGE}"),
    }
}

/// Writes `save`, reads the bytes back and checks the rewrite is the same
/// (every file the tool writes passes its own `check`).
fn write_checked(save: &D2s, t: &Tables) -> Result<Vec<u8>> {
    let bytes = d2s::write(save, t).map_err(|e| anyhow!("write: {e}"))?;
    let opts = read_options(&bytes, None);
    match round_trip(&bytes, &opts, t)? {
        RoundTrip::Same(_) => Ok(bytes),
        RoundTrip::Differs { offset, .. } => {
            bail!("internal: the written save does not rewrite to itself (offset {offset:#x})")
        }
    }
}

/// Runs a command line (without the program name); returns the exit code.
pub fn run(args: &[String], out: &mut dyn Write) -> Result<i32> {
    let Some((cmd, rest)) = args.split_first() else {
        write!(out, "{USAGE}")?;
        return Ok(2);
    };
    let (e, c) = parse(rest.to_vec())?;
    match cmd.as_str() {
        "new" => {
            if !c.files.is_empty() {
                bail!("new takes no input file");
            }
            let t = tables(&c)?;
            let save = new_save(&e, &t)?;
            let bytes = match c.legacy {
                Some(v) => {
                    d2s::legacy::write(&save, v).map_err(|e| anyhow!("legacy write: {e}"))?
                }
                None => write_checked(&save, &t)?,
            };
            let p = write_out(&c, &bytes)?;
            writeln!(out, "wrote {} ({} bytes)", p.display(), bytes.len())?;
        }
        "new-stub" => {
            let name = e.name.as_deref().context("--name is required")?;
            let class = e.class.context("--class is required")?;
            let flags = if e.hardcore == Some(true) {
                status::HARDCORE
            } else {
                0
            } | if e.expansion { status::EXPANSION } else { 0 };
            let time = e.time.unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs() as u32)
            });
            let stub = D2s::new_stub(name.as_bytes(), class, flags, time)
                .ok_or_else(|| anyhow!("name {name:?}: at most 15 bytes, no NUL"))?;
            // A stub has no stats or items: no table is read.
            let bytes = d2s::write(&stub, &NoTables).map_err(|e| anyhow!("write: {e}"))?;
            let p = write_out(&c, &bytes)?;
            writeln!(out, "wrote {} ({} bytes)", p.display(), bytes.len())?;
        }
        "dump" | "check" => {
            let f = one_file(&c)?;
            let bytes = std::fs::read(f).with_context(|| format!("reading {}", f.display()))?;
            let opts = read_options(&bytes, c.expansion_read);
            let stub = bytes.len() == d2s::HEADER_SIZE;
            let t = if stub { None } else { Some(tables(&c)?) };
            let st: &dyn d2s::SaveTables = match &t {
                Some(t) => t,
                None => &NoTables,
            };
            let rt = round_trip(&bytes, &opts, st)?;
            if cmd == "dump" {
                let save = match &rt {
                    RoundTrip::Same(s) | RoundTrip::Differs { save: s, .. } => s,
                };
                write!(out, "{}", dump(save, t.as_ref()))?;
                return Ok(0);
            }
            match rt {
                RoundTrip::Same(_) => {
                    writeln!(
                        out,
                        "{}: OK: {} bytes read in section order (expansion {}) and rewritten byte for byte",
                        f.display(),
                        bytes.len(),
                        opts.expansion
                    )?;
                }
                RoundTrip::Differs {
                    offset,
                    file_len,
                    rewrite_len,
                    ..
                } => {
                    writeln!(
                        out,
                        "{}: DIFFERS: first difference at offset {offset:#x} (file {file_len} bytes, rewrite {rewrite_len})",
                        f.display()
                    )?;
                    return Ok(1);
                }
            }
        }
        "set" => {
            let f = one_file(&c)?;
            let bytes = std::fs::read(f).with_context(|| format!("reading {}", f.display()))?;
            let t = tables(&c)?;
            let opts = read_options(&bytes, None);
            let mut save = d2s::read(&bytes, &opts, &t)
                .map_err(|e| anyhow!("read: {e} (load result {:?})", e.result()))?;
            apply(&mut save, &e, &t)?;
            for n in resave(&mut save, &t)? {
                writeln!(out, "note: {n}")?;
            }
            let bytes = write_checked(&save, &t)?;
            let p = write_out(&c, &bytes)?;
            writeln!(out, "wrote {} ({} bytes)", p.display(), bytes.len())?;
        }
        "help" | "--help" | "-h" => write!(out, "{USAGE}")?,
        other => bail!("unknown command {other:?}\n\n{USAGE}"),
    }
    Ok(0)
}

/// Tables for a stub: never asked (a stub has no sections).
struct NoTables;

impl d2s::SaveTables for NoTables {
    fn stat_save(&self, _: u16) -> Option<d2s::StatSave> {
        None
    }
    fn item_entry_len(&self, _: &[u8]) -> Result<usize, String> {
        Err("no tables".to_owned())
    }
}
