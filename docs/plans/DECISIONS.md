# Decisions for the owner

Open questions that need the owner. Lanes don't wait on these: they build what doesn't depend on
the answer and add new questions here (through LC). When the owner answers, the answer moves into
the plan it belongs to and the item is removed.

| # | Area | Question | Options / suggestion | Raised |
|---|---|---|---|---|
| 1 | Forge | The forge cuts any HTTPS request at about 2 minutes, so release files over ~20 MB can't be uploaded from BIGRIG (they now go through `WerdeGames/D2RUST-builds` over SSH). | Whoever runs the forge server raises the Caddy timeout (and checks Forgejo's attachment size limit). Who runs it, or give LC SSH access. | 2026-10-16 |
| 9 | Bug reports | Deferred: where the report intake service (B2) runs, and its domain. | B2 is shelved (owner, 2026-10-17); revisit hosting only when resumed. | 2026-10-16 |

## Answered

Answers the owner gave; each one is recorded in the plan it belongs to.

| Area | Answer | Plan | Date |
|---|---|---|---|
| Sessions | Lanes run locally on BIGRIG; the account 2 cloud login is not needed. | — | 2026-10-16 |
| Smoothness S10 | Every system runs at a base 60 Hz, no exceptions. | [smoothness-update](game/smoothness-update.md) | 2026-10-16 |
| Smoothness S7 | Generated in-between frames approved: build the full module. | [smoothness-update](game/smoothness-update.md) | 2026-10-16 |
| Display | No separate screenshot check: the display update gets full testing instead. | [display-v2](game/display-v2.md) | 2026-10-16 |
| Multiplayer | QUIC transport; server-hosted only (no player hosting); online characters live on the server only; 8 players per game now, built so 64 is possible later. | [multiplayer](server/multiplayer.md) | 2026-10-16 |
| Multiplayer | Multiplayer code stays on its own branch (`mp/v0`) and goes into no build until the owner says so. | [multiplayer](server/multiplayer.md) | 2026-10-16 |
| Bug reports | Crash auto-report (B7): the player is asked; opt-in, off by default. | [bug-reports](launcher/bug-reports.md) | 2026-10-16 |
| Bug reports | The game itself redacts the home folder and user name in its bug reports (B0a), as a second safeguard alongside the launcher's (B1). | [bug-reports](launcher/bug-reports.md) | 2026-10-17 |
| Display v2 | The Video page drops the unused Perspective row (it never changed). | [display-v2](game/display-v2.md) | 2026-10-16 |
| Display v2 | Add HD lighting and effects (V12) and locally generated HD sprites (V13) as later modules. | [display-v2](game/display-v2.md) | 2026-10-16 |
| Display v2 | Drop V12 (HD lighting and effects) and V13 (locally generated HD sprites) entirely; Display v2 ends at V11. | [display-v2](game/display-v2.md) | 2026-10-17 |
| Test tools | Captured frames and artifacts stay local on E: (never committed or uploaded); the control channel is off unless `--control`, stdin or a named pipe only; real-data scenarios run in CI only on BIGRIG's runner once per train batch, perf budgets report-only; `--offscreen` check runs use the player's resolution and video settings with dev tools on, plain check runs unchanged. | [test-automation-tools](tools/test-automation-tools.md) | 2026-10-16 |
| Mods | Each mod is its own game: shipped as its own files, launched directly, its own binary built on the D2RUST crates (e.g. D2RUST-MXL). The D2RUST exe never loads or selects a mod. | [mod seams](mods/seams.md) | 2026-10-17 |
| Mods | D2RUST is a ~99% Diablo 2 remaster: key systems may be modernized on purpose. Big creative changes go in mod games, and a mod game may override anything. | [mod seams](mods/seams.md) | 2026-10-17 |
| Mod seams | `Ruleset { Original, Mod(Arc<dyn ModRules>) }`, held by `ActionHooks::rules` and fixed per game, with mod-first providers: kept, as override points a mod binary installs. A missing override point is added when a mod needs it; no closed plugin API. | [mod seams](mods/seams.md) | 2026-10-17 |
| Mod seams | No `.d2rmod` save sidecar: each mod game has its own saves (own folder and format). | [mod seams](mods/seams.md) | 2026-10-17 |
| Mod seams | D2RUST and the mod games share the multiplayer architecture. The protocol's extension envelope (C->S 0x71, S->C 0xB5, `[id][u16 len][payload]`) is how a mod game adds messages; plain D2RUST rejects them. The server and session layer is a shared engine part mod binaries reuse. | [mod seams](mods/seams.md) | 2026-10-17 |
| Rewrite | REWRITE project (owner, 2026-10-17): comments rewrite to improve information and understanding for AI and people, plus code organizing where it helps; no logic changes. Scope of this project only. | — | 2026-10-17 |
