//! Every step verb a scenario may use: the capability it needs and
//! whether the tool can drive it yet.
//!
//! A scenario is **pending** while one of the capabilities it needs is
//! missing: either the tool has no driver for a verb yet (`built: false`
//! below; the feature's wire messages aren't in the contracts or the code
//! yet), or the server doesn't serve it (its ready line lists `caps`
//! without it). Pending scenarios are reported and never fail a run. When
//! a feature lands, its lane writes the verb's driver in `runner.rs`,
//! flips `built` to true here, and the scenarios that use it run from then
//! on.

/// What a verb does to the step's flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Runs once; it may take until its `within` time.
    Action,
    /// Reads the bot's view; retried until its expectations hold or its
    /// `within` time runs out.
    Query,
}

/// One verb.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Verb {
    /// The word in a scenario.
    pub name: &'static str,
    /// The capability it needs.
    pub cap: &'static str,
    /// Action or query.
    pub kind: Kind,
    /// The tool can drive it.
    pub built: bool,
    /// Its arguments and facts, for `--verbs`.
    pub about: &'static str,
}

const fn verb(
    name: &'static str,
    cap: &'static str,
    kind: Kind,
    built: bool,
    about: &'static str,
) -> Verb {
    Verb {
        name,
        cap,
        kind,
        built,
        about,
    }
}

use Kind::{Action, Query};

/// Every bot verb. Capabilities: `connect` (handshake, resume), `game`
/// (entering a game, seeing the others), `walk`, `accounts`,
/// `characters`, `lobby`, `chat`, `party`, `trade`, `items`, `combat`,
/// `waypoints`, `acts`; the server-only ones are in [`SERVER_CAPS`].
#[rustfmt::skip]
pub const VERBS: &[Verb] = &[
    // Connection.
    verb("connect", "connect", Action, true,
        "[build=S] [protocol=N] [tick=N] [game=ID] [trust=pinned|any|wrong]; facts client, max_players, tick"),
    verb("reconnect", "connect", Action, true,
        "comes back with the last resume token; facts client, same=yes|no"),
    verb("disconnect", "connect", Action, true, "ends the connection with a goodbye"),
    verb("stats", "connect", Query, true,
        "facts connected=yes|no, frames, ticks, delivered, bad (malformed messages), ids (count per message id)"),
    // A game, entered directly (C→S 0x67 then 0x6B), and play in it.
    verb("enter", "game", Action, true,
        "GAME [class=N] [difficulty=N]; ok once the bot's own player is assigned (refused=no-player otherwise); facts guid, x, y"),
    verb("leave", "game", Action, true, "leaves the game (C→S 0x69)"),
    verb("move", "walk", Action, true, "X Y or +DX +DY (relative to its own position): walks there"),
    verb("frames", "game", Action, true, "N: waits until N more server frames have arrived"),
    verb("see", "game", Query, true,
        "facts players (count), x, y (own), bN=yes|no, bN.x, bN.y, bN.moved=yes|no (since first seen) for every other bot"),
    verb("agree", "game", Query, true,
        "on a group: agree=yes when every bot of the group sees the same players at the same places"),
    // Accounts (register + login before anything else, owner 2026-10-19).
    verb("register", "accounts", Action, true, "ACCOUNT PASSWORD; facts ms (answer time), wait_s"),
    verb("login", "accounts", Action, true, "ACCOUNT PASSWORD; facts ms (answer time), wait_s"),
    verb("logout", "accounts", Action, true, "ends the session"),
    // Online characters, stored on the server, owned by an account.
    verb("char-create", "characters", Action, true, "NAME CLASS [hardcore] [classic]"),
    verb("char-list", "characters", Query, true, "facts count, has.NAME=yes|no, NAME.level, NAME.difficulties (mask)"),
    verb("char-select", "characters", Action, true, "NAME; facts difficulties"),
    verb("char-delete", "characters", Action, true, "NAME PASSWORD (the account's, to confirm)"),
    // Lobby.
    verb("games", "lobby", Query, true,
        "facts count, has.NAME=yes|no, NAME.players, NAME.max, NAME.difficulty (normal|nightmare|hell), NAME.locked=yes|no, NAME.joinable (0 or an error code)"),
    verb("create-game", "lobby", Action, true,
        "NAME [password=P] [difficulty=normal|nightmare|hell] [max=N]: creates, attaches and enters it; facts game_id"),
    verb("join-game", "lobby", Action, true,
        "NAME [password=P]: joins by its name in the list, attaches and enters; facts game_id, difficulty"),
    verb("leave-game", "lobby", Action, true, "leaves the game (0x69, game stream ends): back in the lobby"),
    verb("chat", "chat", Action, true, "TEXT"),
    verb("heard", "chat", Query, true, "facts last (the last line heard), from"),
    // Play together.
    verb("party-invite", "party", Action, false, "bN"),
    verb("party-accept", "party", Action, false, "bN"),
    verb("party", "party", Query, false, "facts members, bN=yes|no"),
    verb("trade", "trade", Action, false, "bN: asks bN to trade"),
    verb("trade-accept", "trade", Action, false, "accepts the open trade request"),
    verb("trade-offer", "trade", Action, false, "ITEM: puts an item in the trade window"),
    verb("trade-confirm", "trade", Action, false, "confirms the trade"),
    verb("inventory", "items", Query, true,
        "facts count, has.ITEM=yes|no (ITEM an item code; items it holds: inventory, belt, cursor, worn)"),
    verb("drop", "items", Action, true, "ITEM: drops it on the ground (to the cursor, then down); ok once it lies there"),
    verb("pickup", "items", Action, true, "ITEM: walks to the nearest one on the ground and picks it up; ok once it holds it"),
    verb("ground", "items", Query, true, "facts count, has.ITEM=yes|no (items it sees on the ground)"),
    verb("attack", "combat", Action, true,
        "MONSTER (as a dev spawn named it): every bot of the step attacks it until it sees it die; ok once it attacked at least once; facts attacks"),
    verb("monster", "combat", Query, true,
        "MONSTER (as a dev spawn named it): facts seen=yes|no, life (0..128), dead=yes|no"),
    verb("waypoint", "waypoints", Action, true,
        "AREA (a name such as cold-plains, or a level id): travels by the nearest waypoint; ok once there"),
    verb("act", "acts", Action, true, "N: travels by waypoint to act N's town; ok once there"),
    verb("area", "game", Query, true, "facts area (a name such as cold-plains, or the level id), level, act"),
];

/// Capabilities only the server can say it has: a scenario that needs
/// one runs only on a server whose ready line lists it. `persist`
/// (accounts and characters kept across a restart), `admin` (offline
/// admin commands: `server admin ARGS`), `dev` (dev commands for a
/// character on a test server: `server dev CHARACTER COMMAND`), `mods`.
pub const SERVER_CAPS: &[&str] = &["persist", "admin", "dev", "mods"];

/// Capabilities the tool can drive: every built verb's, plus the server
/// steps the runner does itself.
pub fn driven() -> std::collections::BTreeSet<&'static str> {
    let mut s: std::collections::BTreeSet<&str> =
        VERBS.iter().filter(|v| v.built).map(|v| v.cap).collect();
    s.extend(["persist", "admin", "dev"]);
    s
}

/// The capability a server step needs, if any.
pub fn server_step_cap(verb: &str) -> Option<&'static str> {
    match verb {
        "admin" => Some("admin"),
        "dev" | "progression" => Some("dev"),
        _ => None,
    }
}

/// The verb named `name`.
pub fn find(name: &str) -> Option<&'static Verb> {
    VERBS.iter().find(|v| v.name == name)
}

/// Extra capabilities a verb's arguments ask for (`connect game=X`, a
/// hello for another game, needs `mods`: the handshake has a game id from
/// wire version 4 on).
pub fn arg_caps(verb: &str, args: &[String]) -> Vec<&'static str> {
    match verb {
        "connect" | "reconnect" if args.iter().any(|a| a.starts_with("game=")) => vec!["mods"],
        _ => Vec::new(),
    }
}
