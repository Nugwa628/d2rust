//! Save loading: the save name, [`load_character`], the refused-load
//! character and the character loader of the join (`loaded_record`,
//! the mouse skills).

use super::*;

/// The save header's character name, up to the first NUL; empty for a file
/// shorter than the header. Both current and legacy headers are supported.
pub fn save_name(bytes: &[u8]) -> &[u8] {
    d2s::header_name(bytes)
}

/// The character of `d2-client play --save <file.d2s>`: the file read and
/// checked with the user's tables ([`LiveData::read_save`]), loaded at the join
/// with [`LoadContext`] of the app's game (Normal; the saved map seed does not
/// apply: the app's game runs on a fixed seed).
pub fn load_character(
    data: &GameData,
    path: &std::path::Path,
    difficulty: u8,
) -> Result<Character, BuildError> {
    load_character_with(data, path, difficulty, &D2sCodec)
}

/// [`load_character`] with the game's own character format (a mod
/// game's [`CharacterCodec`]).
pub fn load_character_with(
    data: &GameData,
    path: &std::path::Path,
    difficulty: u8,
    codec: &dyn CharacterCodec,
) -> Result<Character, BuildError> {
    let err = |message: String| BuildError::Save {
        path: path.display().to_string(),
        message,
    };
    let GameData::Live(d) = data;
    let bytes = std::fs::read(path).map_err(|e| err(e.to_string()))?;
    let save = codec.read(d, &bytes, difficulty).map_err(err)?;
    Ok(Character::Save(
        Box::new(save),
        LoadContext {
            difficulty,
            map_seed_applies: false,
        },
    ))
}

/// The refused character of the save at `path`, when the game's load of it fails
/// with a result; `None` when it loads or cannot be read.
pub fn refused_load(data: &GameData, path: &std::path::Path, difficulty: u8) -> Option<Character> {
    let GameData::Live(d) = data;
    let bytes = std::fs::read(path).ok()?;
    let err = d.read_save(&bytes, difficulty).err()?;
    let result = err.result()?;
    let after_player = err.internal().is_some_and(|c| (15..=23).contains(&c))
        || matches!(
            err,
            d2s::D2sError::LegacyResult {
                after_player: true,
                ..
            }
        );
    let skills = err
        .internal()
        .filter(|c| (20..=23).contains(c))
        .and_then(|_| d2_formats::d2s::skills_before_failure(&bytes, &d.save));
    let mut name = [0u8; 16];
    let n = save_name(&bytes);
    name[..n.len()].copy_from_slice(n);
    let status = d2_formats::d2s::header_status(&bytes).unwrap_or(0) as u8;
    Some(Character::Refused(RefusedLoad {
        result: u32::from(result),
        name,
        expansion: status & d2_formats::d2s::status::EXPANSION as u8 != 0,
        difficulty,
        class: d2_formats::d2s::header_class(&bytes),
        after_player,
        skills,
    }))
}

/// The character each joining client plays, from its id and its 0x67
/// request (which carries the character's name).
pub type Characters = Box<dyn FnMut(ClientId, &CreateGame) -> Character>;

/// The [`Characters`] of single player: `character` for the first join,
/// a new character for any later one.
pub fn one_character(character: Character) -> Characters {
    let mut first = Some(character);
    Box::new(move |_, _| first.take().unwrap_or_default())
}

/// The character load of the session flow's join: the player of
/// the request's class, as the save loader leaves it (no room, at (0, 0),
/// mode 1), then the [`Character`]'s values; the player's quest record
/// (a new one) and name go to the wired host's rest.
/// What did not apply is logged in [`LocalSeams::log`].
///
/// `characters` names the character of each joining client: single
/// player has one ([`one_character`]); a game with several clients picks
/// each client's own.
pub fn loader(mut characters: Characters) -> CharacterLoader<WorldSim<LocalSeams>, World> {
    Box::new(move |s: &mut Sim, client: ClientId, r: &CreateGame| {
        let character = characters(client, r);
        // A refused load gives the result before any player exists
        // (`session_flow.rs` join: 0xB4 and the client removed).
        if let Character::Refused(refused) = &character {
            if !refused.after_player {
                return Err(refused.result);
            }
        }
        // A save's own class allocates the player: the request's class byte is
        // 0 for an existing character (`create_request_for`).
        let class = match &character {
            Character::Save(save, _) => save.header.class,
            Character::Refused(r) => r.class,
            _ => r.class,
        };
        let req = AllocRequest {
            ty: UnitType::Player,
            class: u32::from(class),
            room: None,
            add: true,
            fixed_guid: None,
            mode: 1,
            allied: true,
        };
        let Some(player) = s.events.action.with(&mut s.game, |g, v| {
            // The load draws the player's unit
            // seed right after the allocation, before
            // the save's or the start items and the act's DRLG.
            let p = v.allocate(g, &req, 0, 0)?;
            v.init_player_seed(p);
            // A player is good (2), its state-105
            // list there before its first 0xAA. The game sets it inside the
            // allocation; it
            // draws nothing, so running it after the seed step is
            // equivalent.
            v.set_alignment(g, p, 2);
            Some(p)
        }) else {
            s.events
                .action
                .hooks()
                .x
                .log
                .push(format!("join: allocating player class {class} failed"));
            return Err(LOAD_FAILED);
        };
        if let Some(u) = s.events.action.sys.units.get_mut(player) {
            u.mode = 1;
        }
        let (entry, quests) = match &character {
            Character::New | Character::Named(_) => {
                // B034: a new character knows only the first town's waypoint
                // (record bit 0); the others are activated at their objects.
                // The earlier staging also granted Cold Plains. The stub path
                // (start stats, `StartSkill`), so the join sends 0x5F and the
                // two 0x23. Then the start items on the wired host. Their
                // queued 0x9C / 0x9D (`items.sent`) are the join's item
                // messages, sent after the stat messages.
                let (entry, report, items) = load_new_character_with_items(s, player, r.char_name);
                super::save_gaps::seed_new_flags(
                    s,
                    player,
                    GAME_SETUP.expansion,
                    character.difficulty(),
                );
                let own: Vec<Vec<u8>> = items
                    .sent
                    .iter()
                    .filter(|(u, _)| *u == player)
                    .map(|(_, b)| b.clone())
                    .collect();
                if !own.is_empty() {
                    s.events
                        .action
                        .sys
                        .hooks
                        .session
                        .join_items
                        .insert(player, own);
                }
                // The synthetic game has an inventory model but no vitals
                // tables: its start items are not made (q-a4-quest-items).
                let has_inventory =
                    s.world.inventory.is_some() && s.events.action.hooks().vitals.is_some();
                let log = &mut s.events.action.hooks().x.log;
                log.extend(
                    report
                        .unapplied
                        .iter()
                        .map(|u| format!("join: new character: {u:?}")),
                );
                // Without an inventory model (synthetic data) the report's
                // "start items" step already names why nothing was made.
                if has_inventory {
                    log.extend(
                        items
                            .faults
                            .iter()
                            .map(|f| format!("join: new character: start items: {f}")),
                    );
                }
                (entry, PlayerQuests::default())
            }
            // A section reader failed with the player allocated: the join
            // announces the player, then refuses (`LOAD_AFTER_PLAYER`).
            Character::Refused(r) => {
                if let Some(skills) = &r.skills {
                    crate::adapters::session::apply_skills_before_failure(s, player, skills);
                }
                // The client record holds the player for the announcement.
                if let Some(id) = s.sim_client(client) {
                    if let Some(e) = s.game.lists.client_mut(id) {
                        e.player = Some(player);
                    }
                }
                let skills = if r.skills.is_some() {
                    crate::adapters::session_flow::LOAD_AFTER_SKILLS
                } else {
                    0
                };
                return Err(r.result | crate::adapters::session_flow::LOAD_AFTER_PLAYER | skills);
            }
            Character::Save(save, ctx) => match load_save(s, player, save, ctx) {
                Ok((mut entry, report)) => {
                    // A stub (new-character flag, load; a legacy save's
                    // new-character bit):
                    // the start items as for a new character, queued as the
                    // join's 0x9C / 0x9D.
                    if report.new_character {
                        let items = s.world.start_items(&mut s.game, &mut s.events, player);
                        super::save_gaps::seed_new_flags(
                            s,
                            player,
                            GAME_SETUP.expansion,
                            character.difficulty(),
                        );
                        let own: Vec<Vec<u8>> = items
                            .sent
                            .iter()
                            .filter(|(u, _)| *u == player)
                            .map(|(_, b)| b.clone())
                            .collect();
                        if !own.is_empty() {
                            s.events
                                .action
                                .sys
                                .hooks
                                .session
                                .join_items
                                .insert(player, own);
                        }
                    }
                    // The skill section's assigns turn the passive states on
                    // with their stat lists (the save's skills section,
                    // before the items).
                    s.events.action.passive_refresh_all(&mut s.game, player);
                    // q-save-full: the save's items, made on the wired host.
                    let items_ok = super::save_full::join_items(s, player, save);
                    // The post-load selections and the
                    // join's 0x23 pair read player data as the items' weapon
                    // bookkeeping left it, not the
                    // header.
                    let saved = s
                        .world
                        .saved_mouse_skills(&mut s.game, &mut s.events, player)
                        .filter(|_| save.body.is_some());
                    if let Some(saved) = saved {
                        (entry.load_selects, entry.load_item_selects) =
                            super::save_gaps::saved_selects(saved);
                    }
                    let corpses_ok = super::save_full::join_corpses(s, player, save);
                    // The saved hireling (load order: the player's
                    // items, the corpses, then the hireling): its roomless
                    // allocation draws its unit seed before game entry
                    // populates the rooms, and the monster init of that
                    // allocation (its component and stat rolls) needs the lent
                    // world.
                    let (game, world) = (&mut s.game, &mut s.world);
                    s.events.lend_world(|a| world.hireling_calls(game, a));
                    super::save_gaps::join_gaps(s, client, player, save);
                    // The hot keys, their item
                    // indices resolved over the loaded inventory list.
                    entry.hotkeys = super::save_gaps::loaded_hotkeys(
                        &save.header.hotkeys,
                        &s.world.item_guids(player),
                    );
                    // A full save's record
                    // (the join's 0x5F and 0x23 pair). A stub's is the loader's.
                    if entry.record.is_none() {
                        let portals = s.events.action.sys.hooks.drlg.data.portal_levels();
                        let guids = s.world.item_guids(player);
                        let mut rec = match saved {
                            Some(saved) => super::save_gaps::saved_record(
                                saved,
                                &guids,
                                initial_portal_flags(&portals),
                            ),
                            None => super::save_gaps::loaded_record(
                                &save.header.mouse[..2],
                                &guids,
                                initial_portal_flags(&portals),
                            ),
                        };
                        // A legacy save writes the mouse skills only (player
                        // data );
                        // the item words , so the join's two
                        // 0x23 carry item 0 (recorded: gen-fmt-legacy-v59).
                        if save.header.version < d2s::VERSION_MIN {
                            for h in &mut rec.hands {
                                h.item = 0;
                            }
                        }
                        entry.record = Some(rec);
                    }
                    let log = &mut s.events.action.hooks().x.log;
                    log.extend(
                        report
                            .unapplied
                            .iter()
                            .filter(|u| !(items_ok && u.step == "items"))
                            .filter(|u| !(corpses_ok && u.step == "corpse"))
                            // Applied below, with the quest record.
                            .filter(|u| u.step != "npc fields")
                            .map(|u| format!("join: save load: {u:?}")),
                    );
                    // Load quests row: each 96-byte record copied into the
                    // player's quest record with normalisation. A stub (no
                    // body) keeps the new record (load).
                    let mut quests = PlayerQuests::default();
                    if let Some(body) = &save.body {
                        for (d, rec) in body.quests.records.iter().enumerate() {
                            match QuestFlags::copy_in(rec, true) {
                                Ok(f) => quests.flags[d] = f,
                                Err(e) => log.push(format!("join: save load: quests {d}: {e}")),
                            }
                        }
                        // The save reader: fields A
                        // and B back into the player's NPC record.
                        quests.first_talk = body.npcs.a;
                        for (d, &b) in body.npcs.b.iter().enumerate() {
                            quests.set_intro_bits(d, b);
                        }
                    }
                    (entry, quests)
                }
                Err(e) => {
                    s.events
                        .action
                        .hooks()
                        .x
                        .log
                        .push(format!("join: save load failed: {e}"));
                    return Err(e.result());
                }
            },
        };
        let name = r.char_name;
        let n = name.iter().position(|&b| b == 0).unwrap_or(name.len());
        let rest = &mut s.world.rest;
        rest.quests.insert(player, quests);
        rest.names.insert(player, name[..n].to_vec());
        // The quest entry (single player takes, mode 0; a new
        // character's stub load calls mode 1 first): its messages (0x5E, 0x28,
        // 0x29, 0x89) are part of the join, sent after the loader's
        // other messages.
        let new_character = matches!(character, Character::New | Character::Named(_))
            || matches!(&character, Character::Save(save, _) if save.body.is_none());
        quest_entry(s, player, new_character);
        // The point parser reads the staged position (`point_state`); the
        // tick moves it to the path's ([`WorldHost::unit_positions`]).
        s.set_unit(
            player,
            crate::adapters::UnitFacts {
                act: 0,
                pos: crate::seams::Pos { x: 0, y: 0 },
                owner: None,
            },
        );
        s.set_player(
            player,
            PlayerFields {
                gate: PlayerGate {
                    mode: 1,
                    uninterruptable: false,
                },
                data: Some(PlayerData { last_accept: 0 }),
            },
        );
        Ok(Loaded { player, entry })
    })
}
