//! Table loading into the client model (`client_*`: level, object and
//! monster rows and labels, skill rows, unit rows, monster anims).

use super::*;
use rows::*;

/// The `objects.txt` records after their load fix-up
/// `FrameCnt0`–`7` in 1/256 frames), for the sim tables that read
/// them as `>> 8` (waypoint init 17: ENDANIM at
/// frame + `FrameCnt1`). Without the fixed table: the raw rows, shifted.
pub(super) fn fixed_objects(d: &LiveData) -> Vec<Objects> {
    let fixed: Option<Vec<Objects>> = d
        .tables
        .fixed
        .table("objects")
        .and_then(|t| decode_all(t).ok());
    fixed.unwrap_or_else(|| {
        let mut rows = d.waypoints.objects.clone();
        for o in &mut rows {
            o.framecnt1 = o.framecnt1.wrapping_shl(8);
        }
        rows
    })
}

/// The `objects.txt` rows of the client object update: the live table **after
/// its load fix-up** (`FrameCnt0`–`7` in 1/256 frames,
/// as the mode end `End(m)` reads them). The raw `.bin` rows hold whole frames: a
/// one-frame mode would otherwise clamp below zero and lose its arrival frame.
pub fn client_object_rows(data: &GameData) -> Vec<rows::ObjClientRow> {
    let GameData::Live(d) = data;
    let fixed: Option<Vec<Objects>> = d
        .tables
        .fixed
        .table("objects")
        .and_then(|t| decode_all(t).ok());
    match fixed {
        Some(rows) => rows::ObjClientRow::rows(&rows),
        // No fixed table: the fix-up on the raw rows.
        None => rows::ObjClientRow::rows(&d.waypoints.objects)
            .into_iter()
            .map(rows::ObjClientRow::frame_counts_fixed)
            .collect(),
    }
}

/// The `objects.txt` label fields of each class, for the mouse-over
/// label (`world_view::object_label`).
pub fn client_object_labels(data: &GameData) -> Vec<rows::ObjectLabelRow> {
    data.tables()
        .objects
        .iter()
        .map(|o| {
            let n = &o.name;
            let end = n.iter().position(|&b| b == 0).unwrap_or(n.len());
            rows::ObjectLabelRow {
                name: String::from_utf8_lossy(&n[..end]).into_owned(),
                subclass: o.subclass,
                name_offset: o.nameoffset as i32,
                is_door: o.isdoor != 0,
                lockable: o.lockable != 0,
                open_warp: o.openwarp != 0,
            }
        })
        .collect()
}

/// The hovered-NPC label facts per monster class (`world_view::monster_label`):
/// `monstats` `NameStr` and `interact`, `monstats2` `pixHeight` through
/// `MonStatsEx`.
pub fn client_monster_labels(data: &GameData) -> Vec<rows::MonsterLabelRow> {
    let GameData::Live(d) = data;
    let stats = d
        .tables
        .rows::<d2_data::tables::Monstats>()
        .unwrap_or_default();
    let ex = d
        .tables
        .rows::<d2_data::tables::Monstats2>()
        .unwrap_or_default();
    stats
        .iter()
        .map(|m| rows::MonsterLabelRow {
            name_str: m.namestr,
            interact: m.interact,
            killable: m.killable,
            pix_height: ex.get(usize::from(m.monstatsex)).map_or(0, |e| e.pixheight),
            desc_str: m.descstr,
            demon: m.demon,
            undead: m.lundead || m.hundead,
        })
        .collect()
}

/// The `Levels.txt` fields the client reads (`Pal`, `Act`, `BlankScreen`;
/// `SoundEnv`), one row per level id, from the
/// game's `levels` table, with the `leveldefs` ambient.
pub fn client_level_rows(data: &GameData) -> Vec<LevelRow> {
    // `leveldefs` by level id; a missing
    // table or row reads no colour (the environment applies).
    let GameData::Live(d) = data;
    let defs = d
        .tables
        .rows::<d2_data::tables::Leveldefs>()
        .unwrap_or_default();
    data.tables()
        .levels
        .iter()
        .enumerate()
        .map(|(n, l)| LevelRow {
            ambient: defs
                .get(n)
                .map_or_else(Default::default, |d| rows::Ambient {
                    i: d.intensity,
                    r: d.red,
                    g: d.green,
                    b: d.blue,
                }),
            critters: rows::Critters {
                cmon: [l.cmon1, l.cmon2, l.cmon3, l.cmon4].map(|c| c as i16),
                cpct: [l.cpct1, l.cpct2, l.cpct3, l.cpct4].map(|c| c as i16),
                camt: [l.camt4, 0, 0, 0],
            },
            pal: l.pal,
            act: l.act,
            blank_screen: l.blankscreen != 0,
            sound_env: l.soundenv,
            draw_edges: l.drawedges != 0,
            rain: l.rain != 0,
            mud: l.mud != 0,
        })
        .collect()
}

/// The levels' waypoint indexes (`levels` `Waypoint`) for the client's waypoint
/// menu.
pub fn client_waypoint_map(data: &GameData) -> d2_sim::world::waypoints::WaypointMap {
    d2_sim::world::waypoints::WaypointMap::new(&data.tables().levels)
}

/// The `levels` `LevelName` keys by level id (row index), the waypoint
/// rows' text keys (`crate::ui::original::OriginalUi::set_level_names`).
pub fn client_level_names(data: &GameData) -> Vec<String> {
    data.tables()
        .levels
        .iter()
        .map(|l| {
            let n = l
                .levelname
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(l.levelname.len());
            String::from_utf8_lossy(&l.levelname[..n]).into_owned()
        })
        .collect()
}

/// The `levels` `LevelWarp` keys by level id (row index), for the label of
/// an `OpenWarp` object ([`rows::ObjectLabels`]).
pub fn client_level_warps(data: &GameData) -> Vec<String> {
    data.tables()
        .levels
        .iter()
        .map(|l| {
            let n = l
                .levelwarp
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(l.levelwarp.len());
            String::from_utf8_lossy(&l.levelwarp[..n]).into_owned()
        })
        .collect()
}

/// `difficultylevels` `ResistPenalty` per row (difficulty), from the
/// user's `.bin` set: the expansion resist penalty of the character
/// panel, for
/// `crate::ui::original::OriginalUi::set_resist_penalties`. The field
/// is read as a signed value.
pub fn client_resist_penalties(archives: &dyn TableFiles) -> Result<Vec<i32>, BuildError> {
    let set =
        d2_data::bin::load_from(archives, "eng").map_err(|e| BuildError::Tables(e.to_string()))?;
    let table = set
        .table("difficultylevels")
        .ok_or_else(|| BuildError::Tables("difficultylevels not loaded".into()))?;
    let rows: Vec<Difficultylevels> =
        decode_all(table).map_err(|e| BuildError::Tables(e.to_string()))?;
    Ok(rows.iter().map(|r| r.resistpenalty as i32).collect())
}

/// The `skills` fields the client skill list reads (`anim`; `monanim`;
/// `passivestate`; `enhanceable`; `EType`;
/// `skilldesc`; `srvdofunc`; `charclass`; `maxlvl`), one row per skill id,
/// from the user's `skills` table.
pub fn client_skill_rows(archives: &dyn TableFiles) -> Result<Vec<SkillRow>, BuildError> {
    let set =
        d2_data::bin::load_from(archives, "eng").map_err(|e| BuildError::Tables(e.to_string()))?;
    let table = set
        .table("skills")
        .ok_or_else(|| BuildError::Tables("skills not loaded".to_owned()))?;
    let rows: Vec<Skills> = decode_all(table).map_err(|e| BuildError::Tables(e.to_string()))?;
    Ok(rows.iter().map(skill_row).collect())
}

/// Each class's `charstats` `Skill 1`…`Skill 10`,
/// one entry per `charstats` row, from the user's table.
pub fn client_class_skills(archives: &dyn TableFiles) -> Result<Vec<[u16; 10]>, BuildError> {
    let set =
        d2_data::bin::load_from(archives, "eng").map_err(|e| BuildError::Tables(e.to_string()))?;
    let table = set
        .table("charstats")
        .ok_or_else(|| BuildError::Tables("charstats not loaded".to_owned()))?;
    let rows: Vec<Charstats> = decode_all(table).map_err(|e| BuildError::Tables(e.to_string()))?;
    Ok(rows.iter().map(crate::skill_rest::class_skills).collect())
}

/// Each class's `charstats` `WalkVelocity` and `RunVelocity`,
/// one entry per row: the path speeds a skill start sets.
pub fn client_class_velocities(archives: &dyn TableFiles) -> Result<Vec<[u8; 2]>, BuildError> {
    let set =
        d2_data::bin::load_from(archives, "eng").map_err(|e| BuildError::Tables(e.to_string()))?;
    let table = set
        .table("charstats")
        .ok_or_else(|| BuildError::Tables("charstats not loaded".to_owned()))?;
    let rows: Vec<Charstats> = decode_all(table).map_err(|e| BuildError::Tables(e.to_string()))?;
    Ok(rows
        .iter()
        .map(|c| [c.walkvelocity, c.runvelocity])
        .collect())
}

/// Each class's charstats life per vitality, stamina per vitality and
/// mana per energy, one entry per `charstats` row.
pub fn client_class_vitals(archives: &dyn TableFiles) -> Result<Vec<[u8; 3]>, BuildError> {
    let set =
        d2_data::bin::load_from(archives, "eng").map_err(|e| BuildError::Tables(e.to_string()))?;
    let table = set
        .table("charstats")
        .ok_or_else(|| BuildError::Tables("charstats not loaded".to_owned()))?;
    let rows: Vec<Charstats> = decode_all(table).map_err(|e| BuildError::Tables(e.to_string()))?;
    Ok(rows
        .iter()
        .map(|c| [c.lifepervitality, c.staminapervitality, c.manapermagic])
        .collect())
}

/// Each stat's op-target table entries(T) as (source stat, op), one entry per `itemstatcost`
/// row, from the user's table with the load fix-up (`d2_data::fixup::records::stat_ops`)
/// applied to a copy.
pub fn client_stat_ops(archives: &dyn TableFiles) -> Result<Vec<Vec<(u16, u8)>>, BuildError> {
    let set =
        d2_data::bin::load_from(archives, "eng").map_err(|e| BuildError::Tables(e.to_string()))?;
    let mut table = set
        .table("itemstatcost")
        .ok_or_else(|| BuildError::Tables("itemstatcost not loaded".to_owned()))?
        .clone();
    d2_data::fixup::records::stat_ops(&mut table);
    let stats = d2_sim::stats::StatTable::from_fixed(&table)
        .map_err(|e| BuildError::Tables(e.to_string()))?;
    Ok((0..stats.len())
        .map(|s| {
            stats.get(s as u16).map_or_else(Vec::new, |i| {
                i.entries.iter().map(|e| (e.source, e.op)).collect()
            })
        })
        .collect())
}

/// The `monstats` / `monstats2` columns of the client monster set-up: `m` the
/// typed row, `raw` its record (the `Sk`i`mode` bytes
/// columns), `m2` the `monstats2` record (`isSel` record field bit 3, `shadow` +5
/// bit 6, `isAtt` +5 bit 1).
fn monster_setup(m: &Monstats, raw: &[u8], m2: &[u8]) -> MonsterSetup {
    let byte = |b: &[u8], o: usize| b.get(o).copied().unwrap_or(0);
    let skill = [
        (m.skill1, m.sk1lvl),
        (m.skill2, m.sk2lvl),
        (m.skill3, m.sk3lvl),
        (m.skill4, m.sk4lvl),
        (m.skill5, m.sk5lvl),
        (m.skill6, m.sk6lvl),
        (m.skill7, m.sk7lvl),
        (m.skill8, m.sk8lvl),
    ];
    let mut skills = [(0i16, 0u8, 0u8); 8];
    for (i, ((s, l), out)) in skill.into_iter().zip(skills.iter_mut()).enumerate() {
        *out = (s as i16, l, byte(raw, 0x180 + i));
    }
    MonsterSetup {
        level: [m.level, m.level_n, m.level_h],
        res: [
            [m.resdm, m.resdm_n, m.resdm_h],
            [m.resma, m.resma_n, m.resma_h],
            [m.resfi, m.resfi_n, m.resfi_h],
            [m.resli, m.resli_n, m.resli_h],
            [m.resco, m.resco_n, m.resco_h],
            [m.respo, m.respo_n, m.respo_h],
        ],
        velocity: m.velocity,
        align: m.align,
        skills,
        is_sel: byte(m2, 4) & 0x08 != 0,
        shadow: byte(m2, 5) & 0x40 != 0,
        is_att: byte(m2, 5) & 0x02 != 0,
    }
}

/// The skills tables and formula buffers of the client's passive refresh, from
/// the user's tables.
pub fn client_skill_tables(archives: &dyn TableFiles) -> Result<SkillTables, BuildError> {
    let set =
        d2_data::bin::load_from(archives, "eng").map_err(|e| BuildError::Tables(e.to_string()))?;
    SkillTables::from_bin(&set, d2_sim::skills::LEVEL_CAP_114D)
        .map_err(|e| BuildError::Tables(e.to_string()))
}

/// The unit-message rows of the client (each `monstats` row's `MonStatsEx` link
/// into `monstats2`; whose record bytes
/// counts; the `itemstatcost` send columns; the lighting's
/// `objects.txt` and `shrines.txt` codes), from the user's tables.
pub fn client_unit_rows(archives: &dyn TableFiles) -> Result<UnitRows, BuildError> {
    let set =
        d2_data::bin::load_from(archives, "eng").map_err(|e| BuildError::Tables(e.to_string()))?;
    let table = |name: &str| {
        set.table(name)
            .ok_or_else(|| BuildError::Tables(format!("{name} not loaded")))
    };
    let err = |e: d2_data::tables::WrongTable| BuildError::Tables(e.to_string());
    let monstats_table = table("monstats")?;
    let monstats: Vec<Monstats> = decode_all(monstats_table).map_err(err)?;
    let monstats2 = table("monstats2")?;
    let monstats2_rows: Vec<d2_data::tables::Monstats2> = decode_all(monstats2).map_err(err)?;
    // The item `lightradius` of each
    // `compcode` code (the lookup runs over the item
    // tables; the first row with the code wins); `lit`, `med`, `hvy` and
    // the empty code are skipped (0).
    // `.bin` codes are zero padded, the compared codes space padded.
    let pad = |c: [u8; 4]| c.map(|b| if b == 0 { b' ' } else { b });
    let weapons: Vec<d2_data::tables::Weapons> = decode_all(table("weapons")?).map_err(err)?;
    let armor: Vec<d2_data::tables::Armor> = decode_all(table("armor")?).map_err(err)?;
    let misc: Vec<d2_data::tables::Misc> = decode_all(table("misc")?).map_err(err)?;
    let mut item_light = std::collections::BTreeMap::new();
    let items = (weapons.iter().map(|r| (r.code, r.lightradius)))
        .chain(armor.iter().map(|r| (r.code, r.lightradius)))
        .chain(misc.iter().map(|r| (r.code, r.lightradius)));
    for (code, radius) in items {
        item_light.entry(pad(code)).or_insert(radius);
    }
    let compcode: Vec<d2_data::tables::Compcode> = decode_all(table("compcode")?).map_err(err)?;
    let code_light = |idx: u8| -> u8 {
        match compcode.get(usize::from(idx)).map(|r| pad(r.code)) {
            None | Some([b' ', b' ', b' ', b' ']) => 0,
            Some(k) if matches!(&k, b"lit " | b"med " | b"hvy ") => 0,
            Some(k) => item_light.get(&k).copied().unwrap_or(0),
        }
    };
    let monsters = monstats
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let link = m.monstatsex as i16;
            if link < 0 || link as usize >= monstats2.count {
                return None;
            }
            let m2 = monstats2.record(link as usize);
            let mut c = MonsterClass::from_record(m2, m.npc, m.interact)?;
            c.setup = Some(monster_setup(m, monstats_table.record(i), m2));
            for (comp, row) in c.component_light.iter_mut().enumerate() {
                for (v, r) in row.iter_mut().enumerate() {
                    *r = m2.get(0x26 + 12 * comp + v).map_or(0, |&b| code_light(b));
                }
            }
            c.no_aura = m.noaura;
            c.undead = m.lundead || m.hundead;
            c.demon = m.demon;
            c.min_grp = m.mingrp;
            c.max_grp = m.maxgrp;
            c.in_town = m.intown;
            c.flying = m.flying;
            c.open_doors = m.opendoors;
            c.boss = m.boss;
            c.switchai = m.switchai;
            c.base_id = m.baseid;
            c.code = m.code;
            c.spl_end_death = m.splenddeath == 1;
            c.spl_client_end = m.splclientend != 0;
            if let Some(x) = monstats2_rows.get(link as usize) {
                c.light = x.light;
                c.light_rgb = (x.light_r, x.light_g, x.light_b);
                c.no_unique_shift = x.nouniqueshift;
                c.soft = x.soft;
                c.small = x.small;
                c.large = x.large;
                c.bleed = x.bleed;
                c.critter = x.critter;
            }
            Some(c)
        })
        .collect();
    let difficulty: Vec<Difficultylevels> = decode_all(table("difficultylevels")?).map_err(err)?;
    let mut monster_skill_bonus = [0i32; 3];
    for (b, d) in monster_skill_bonus.iter_mut().zip(&difficulty) {
        *b = d.monsterskillbonus as i32;
    }
    let isc: Vec<Itemstatcost> = decode_all(table("itemstatcost")?).map_err(err)?;
    let stats = isc
        .iter()
        .map(|r| StatSend {
            bits: r.send_bits,
            param_bits: r.send_param_bits,
            signed: r.signed,
            update_anim_rate: r.updateanimrate,
        })
        .collect();
    let objects: Vec<Objects> = decode_all(table("objects")?).map_err(err)?;
    let objects = objects
        .iter()
        .map(|o| ObjectRow {
            subclass: o.subclass,
            shrine_function: o.shrinefunction,
            env_effect: o.enveffect != 0,
            lit: [
                o.lit0, o.lit1, o.lit2, o.lit3, o.lit4, o.lit5, o.lit6, o.lit7,
            ],
            selectable: [
                o.selectable0 != 0,
                o.selectable1 != 0,
                o.selectable2 != 0,
                o.selectable3 != 0,
                o.selectable4 != 0,
                o.selectable5 != 0,
                o.selectable6 != 0,
                o.selectable7 != 0,
            ],
            rgb: (o.red, o.green, o.blue),
        })
        .collect();
    let shrines: Vec<Shrines> = decode_all(table("shrines")?).map_err(err)?;
    // The stat lists' retention flags and colour columns.
    // The client create's columns.
    let missiles: Vec<d2_data::tables::Missiles> = decode_all(table("missiles")?).map_err(err)?;
    let missiles = missiles
        .iter()
        .map(|m| rows::ClientMissileRow {
            vel: i32::from(m.vel),
            vel_lev: i32::from(m.vellev),
            max_vel: i32::from(m.maxvel),
            accel: m.accel as i16,
            range: m.range as i16,
            lev_range: m.levrange as i16,
            sub_loop: m.subloop,
            sub_start: m.substart,
            sub_stop: m.substop,
            activate: i32::from(m.activate),
            init_steps: m.initsteps,
            anim_len: m.animlen,
            anim_speed: m.animspeed,
            light: m.light,
            rgb: (m.red, m.green, m.blue),
            can_slow: m.canslow,
            pierce: m.pierce,
            last_collide: m.lastcollide,
            clt_do_func: m.pcltdofunc,
            loop_anim: m.loopanim != 0,
            flicker: m.flicker,
            collide_type: m.collidetype,
            always_explode: m.alwaysexplode != 0,
            explosion_missile: m.explosionmissile as i16,
            clt_hit_func: m.pclthitfunc as i16,
            clt_sub: [
                m.cltsubmissile1 as i16,
                m.cltsubmissile2 as i16,
                m.cltsubmissile3 as i16,
            ],
            clt_param: [
                m.cltparam1 as i32,
                m.cltparam2 as i32,
                m.cltparam3 as i32,
                m.cltparam4 as i32,
                m.cltparam5 as i32,
            ],
            clt_calc1: m.cltcalc1,
            s_hit_calc1: m.shitcalc1,
            town: m.town,
            clt_src_town: m.cltsrctown,
            size: m.size,
            rand_start: m.randstart as i32,
            prog_sound: m.progsound as i16,
            prog_overlay: m.progoverlay as i16,
            param: [m.param1 as i32, m.param2 as i32],
            client_col: m.clientcol != 0,
            collide_kill: m.collidekill != 0,
            collide_friend: m.collidefriend != 0,
            next_hit: m.nexthit != 0,
            next_delay: m.nextdelay,
            can_destroy: m.candestroy,
            clt_hit_sub: [
                m.clthitsubmissile1 as i16,
                m.clthitsubmissile2 as i16,
                m.clthitsubmissile3 as i16,
                m.clthitsubmissile4 as i16,
            ],
            c_hit_par: [m.chitpar1 as i32, m.chitpar2 as i32, m.chitpar3 as i32],
            hit_sub1_server: m.hitsubmissile1 as i16,
            sub1_server: m.submissile1 as i16,
            sub2_server: m.submissile2 as i16,
            travel_sound: m.travelsound as i16,
            hit_sound: m.hitsound as i16,
            no_multishot: m.nomultishot,
        })
        .collect();
    let overlays: Vec<d2_data::tables::Overlay> = decode_all(table("overlay")?).map_err(err)?;
    let overlays = overlays
        .iter()
        .map(|o| rows::OverlayLightRow {
            version: o.version as i16,
            frames: o.frames,
            anim_rate: o.animrate,
            init_radius: o.initradius,
            radius: o.radius,
            rgb: (o.red, o.green, o.blue),
            one_of_n: o.f_1ofn,
        })
        .collect();
    let states: Vec<d2_data::tables::States> = decode_all(table("states")?).map_err(err)?;
    let states = states
        .iter()
        .map(|s| StateRow {
            dead_bit_only: s.notondead,
            keep_list: s.noclear,
            colorpri: s.colorpri,
            colorshift: s.colorshift,
            light_rgb: (s.light_r, s.light_g, s.light_b),
            meleeonly: s.meleeonly,
            restrict: s.restrict,
            links: rows::StateOverlayLinks {
                cast: s.castoverlay as i16,
                overlay1: s.overlay1 as i16,
                overlay2: s.overlay2 as i16,
                remove: s.removerlay as i16,
            },
            aura: s.aura,
            nooverlays: s.nooverlays,
            transform: s.transform,
            disguise: s.disguise,
            setfunc: s.setfunc,
            remfunc: s.remfunc,
            gfxtype: s.gfxtype,
            gfxclass: s.gfxclass,
            udead: s.udead,
            hide: s.hide,
            shatter: s.shatter,
            pgsv: s.pgsv,
            pgsv_overlay: s.pgsvoverlay as i16,
            stat: s.stat as i16,
        })
        .collect();
    // The monster sequences: the slots of the fixed-up
    // `monstats` and the `monseq` lists.
    let monseq: Vec<d2_data::tables::Monseq> = decode_all(table("monseq")?).map_err(err)?;
    let monster_sequences = Arc::new(d2_sim::skills::sequences::MonsterSequences::from_tables(
        monstats_table,
        &monseq,
    ));
    // The client hireling set-up reads the
    // `hireling` rows.
    let hirelings = Arc::new(
        d2_sim::world::hirelings::HirelingRows::from_table(table("hireling")?)
            .map_err(|e| BuildError::Tables(format!("hireling: {e}")))?,
    );
    Ok(UnitRows {
        monster_sequences,
        hirelings,
        monsters,
        monster_skill_bonus,
        stats,
        objects,
        shrines: shrines.iter().map(|s| s.code).collect(),
        states,
        missiles,
        overlays,
    })
}

/// The `skills` fields of the list operations (`client/msg-skills.md`
/// Inputs) of one record.
pub fn skill_row(s: &Skills) -> SkillRow {
    SkillRow {
        anim: s.anim,
        monanim: s.monanim,
        seqnum: s.seqnum,
        passivestate: s.passivestate,
        maxlvl: s.maxlvl,
        charclass: s.charclass as i8,
        srvdofunc: s.srvdofunc as i16,
        enhanceable: s.enhanceable,
        skilldesc: s.skilldesc,
        etype: s.etype,
        seqinput: s.seqinput,
        selectproc: s.selectproc,
        range: s.range,
        flags: skill_flags(s),
        itypes: [
            [s.itypea1 as i16, s.itypea2 as i16, s.itypea3 as i16],
            [s.itypeb1 as i16, s.itypeb2 as i16, s.itypeb3 as i16],
        ],
        etypes: [
            [s.etypea1 as i16, s.etypea2 as i16],
            [s.etypeb1 as i16, s.etypeb2 as i16],
        ],
        ingame: s.ingame,
        aura: s.aura,
        mana: s.mana,
        lvlmana: s.lvlmana,
        manashift: s.manashift,
        cltstfunc: s.cltstfunc,
        param1: s.param1 as i32,
        param2: s.param2 as i32,
        param3: s.param3 as i32,
        param4: s.param4 as i32,
        restrict: s.restrict,
        shape_states: [s.state1 as i16, s.state2 as i16, s.state3 as i16],
        cltdofunc: s.cltdofunc as i16,
        cltmissile: s.cltmissile as i16,
        cltmissile_abc: [
            s.cltmissilea as i16,
            s.cltmissileb as i16,
            s.cltmissilec as i16,
        ],
        castoverlay: s.castoverlay as i16,
        dosound: s.dosound as i16,
        dosound_a: s.dosound_a as i16,
        dosound_b: s.dosound_b as i16,
        tgtoverlay: s.tgtoverlay as i16,
        tgtsound: s.tgtsound as i16,
        cltcalc1: s.cltcalc1,
        progressive: s.progressive,
        lob: s.lob,
        aurastate: s.aurastate as i16,
        aurastat1: s.aurastat1 as i16,
        delay: (s.delay != u32::MAX).then_some(s.delay),
        startmana: s.startmana,
    }
}

pub fn skill_flags(s: &d2_data::tables::Skills) -> u32 {
    use skill_flag::*;
    [
        (s.passive, PASSIVE),
        (s.intown, IN_TOWN),
        (s.targetableonly, TARGETABLE_ONLY),
        (s.searchenemyxy, SEARCH_ENEMY_XY),
        (s.searchenemynear, SEARCH_ENEMY_NEAR),
        (s.searchopenxy, SEARCH_OPEN_XY),
        (s.targetcorpse, TARGET_CORPSE),
        (s.targetpet, TARGET_PET),
        (s.targetally, TARGET_ALLY),
        (s.targetitem, TARGET_ITEM),
        (s.attacknomana, ATTACK_NO_MANA),
        (s.noammo, NO_AMMO),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .fold(0, |m, (_, f)| m | f)
}

pub mod skill_flag {

    /// The skill is passive and cannot be used by clicking.
    pub const PASSIVE: u32 = 1 << 4;
    /// `noammo`: the item type test's ammo rule does not apply to this skill.
    pub const NO_AMMO: u32 = 1 << 16;
    /// The skill may be used in town.
    pub const IN_TOWN: u32 = 1 << 8;
    /// The skill works on targetable units only.
    pub const TARGETABLE_ONLY: u32 = 1 << 20;
    /// The skill searches for an enemy at the clicked point.
    pub const SEARCH_ENEMY_XY: u32 = 1 << 21;
    /// The skill searches for an enemy near the clicked point.
    pub const SEARCH_ENEMY_NEAR: u32 = 1 << 22;
    /// The skill searches for an open spot near the clicked point.
    pub const SEARCH_OPEN_XY: u32 = 1 << 23;
    /// The skill may target a corpse.
    pub const TARGET_CORPSE: u32 = 1 << 24;
    /// The skill may target the player's pets.
    pub const TARGET_PET: u32 = 1 << 25;
    /// The skill may target allies.
    pub const TARGET_ALLY: u32 = 1 << 26;
    /// The skill may target an item on the ground.
    pub const TARGET_ITEM: u32 = 1 << 27;
    /// When the player lacks mana, a click with this skill falls back to a plain Attack.
    pub const ATTACK_NO_MANA: u32 = 1 << 28;
}

/// Composite names needed when loading the monster animation and corpse direction rows.
pub trait MonsterAppearance: crate::local_seams::UnitAppearance {
    fn monster_cof(&self, class: u32, mode: u32) -> Option<super::composite_names::CofName>;
}
impl MonsterAppearance for super::appearance::AnimationNames {
    fn monster_cof(&self, class: u32, mode: u32) -> Option<super::composite_names::CofName> {
        self.cof(d2_sim::units::UnitType::Monster, class, mode)
    }
}
/// Populate animation rows directly from a headless file source.
pub fn client_monster_anims(archives: &GameFiles, rows: &mut UnitRows) -> Result<(), BuildError> {
    client_monster_anims_with(archives, rows, || {
        super::appearance::AnimationNames::load(archives).map_err(BuildError::Tables)
    })
}
/// The animation columns of the client monster rows ([`MonsterClass::anims`],
/// `walk_speed`, `run_speed`): `AnimData.d2` by the class's composite name per
/// mode (the shared composite naming rules) and the
/// fixed-up `monstats` speeds, from the user's files.
pub fn client_monster_anims_with<A: MonsterAppearance>(
    archives: &GameFiles,
    rows: &mut UnitRows,
    load_appearance: impl FnOnce() -> Result<A, BuildError>,
) -> Result<(), BuildError> {
    let anim = match archives.read_native(
        &CanonicalPath::new(d2_formats::animdata::PATH)
            .map_err(|e| BuildError::Tables(format!("{}: {e}", d2_formats::animdata::PATH)))?,
    ) {
        Some(Ok(NativeAsset::AnimData(a))) => a,
        Some(Ok(_)) => return Err(BuildError::Tables("AnimData.d2: wrong kind".into())),
        Some(Err(e)) => return Err(BuildError::Tables(format!("AnimData.d2: {e}"))),
        None => return Err(BuildError::Tables("AnimData.d2: in no archive".into())),
    };
    let bins = d2_data::bin::load_from(archives, d2_data::bin::DEFAULT_LANGUAGE)
        .map_err(|e| BuildError::Tables(e.to_string()))?;
    let tables = GameTables::from_loaded(bins, anim.clone())?;
    let monstats = tables
        .fixed
        .table("monstats")
        .ok_or_else(|| BuildError::Tables("monstats not loaded".into()))?;
    let looks = load_appearance()?;
    for (class, row) in rows.monsters.iter_mut().enumerate() {
        let Some(c) = row.as_mut() else {
            continue;
        };
        if class < monstats.count {
            let r = monstats.record(class);
            c.walk_speed = u16::from_le_bytes([r[0x36], r[0x37]]);
            c.run_speed = u16::from_le_bytes([r[0x38], r[0x39]]);
            // The mode missile columns: A1, A2, S1–S4, SC, SQ.
            for (mode, off) in [
                (4, 0x3A),
                (5, 0x3C),
                (8, 0x3E),
                (9, 0x40),
                (10, 0x42),
                (11, 0x44),
                (7, 0x46),
                (14, 0x48),
            ] {
                let v = u16::from_le_bytes([r[off], r[off + 1]]);
                c.mode_missiles[mode] = (v != 0xFFFF).then_some(v);
            }
        }
        for mode in 0..c.anims.len() {
            let key = looks.anim_key(
                d2_sim::units::UnitType::Monster,
                class as u32,
                mode as u32,
                0,
            );
            let rec = key.and_then(|k| anim.record(&k).ok());
            c.anims[mode] = rec.map(|rec| (rec.frames, rec.speed));
            c.anim_events[mode] = rec.map(|rec| rec.events);
        }
        // The DT / DD COFs' direction count `D` for the dead monster's
        // snap write-back.
        for mode in [0u32, 12] {
            let read = |name: &super::composite_names::CofName| {
                crate::file_source::read_cof(archives, &name.full()).and_then(Result::ok)
            };
            let cof = looks.monster_cof(class as u32, mode).and_then(|name| {
                read(&name).or_else(|| {
                    read(&super::composite_names::CofName {
                        weapon_class: super::composite_names::HTH,
                        ..name
                    })
                })
            });
            c.cof_dirs[mode as usize] = cof.map_or(0, |cof| cof.directions);
        }
    }
    Ok(())
}

/// The charstats walk / run speeds of `character`'s class for the play
/// preview's walk prediction (decision D2); `None` for a class past the rows.
pub fn walk_velocities(
    data: &GameData,
    character: &Character,
) -> Result<Option<(u8, u8)>, BuildError> {
    let GameData::Live(d) = data;
    let class = match character {
        Character::New => PLAYER_CLASS as u8,
        Character::Named(c) => c.class,
        Character::Save(save, _) => save.header.class,
        Character::Refused(_) => 0,
    };
    let rows: Vec<d2_data::tables::Charstats> = d
        .tables
        .rows()
        .map_err(|e| BuildError::Tables(e.to_string()))?;
    Ok(rows
        .get(usize::from(class))
        .map(|r| (r.walkvelocity, r.runvelocity)))
}
