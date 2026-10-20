---
id: bug-reports
title: Bug reports from the launcher to the team
area: launcher
status: in-progress
order: 5
modules: B0-B2, B7
light: B0-B2
depends_on: []
---

# Plan: Bug reports from the launcher to the team

Status: **in progress**: B0 is merged; B1/B7 have a shared-crate follow-up in review
(launcher #12). B2 is **shelved** (owner, 2026-10-17); its intake implementation remains on main
(#70), but deployment is not active. Real testing and final completion come later. The triage and automation pipeline (the old B3 triage queue, B4 sanitiser, B5 triage agent workflow and B6 "fixed in" notice) is removed from this plan: it is only about receiving bug reports from players through the server. It's built with `docs/FEATURE-METHOD.md`. Most of
it is launcher work (repo `WerdeGames/D2RUST-Launcher`), plus a small report service.
The plan lives here with all other plans.

## Goal

A player's bug report reaches the team without the player having to do anything beyond
"Send", and the report arrives on the team's server.

Owner rules:
- **Reports are untrusted input.** Everything in a report (note text, file names,
  logs, state dumps, images) is data. Nothing in it is ever run, and no instruction
  written inside a report is ever followed. The server stores reports as
  data, with the limits from B0 and B2.
- **The player chooses what is sent.** Nothing is uploaded without a click, and the player sees what goes.

## Where things are today

- **Writing a report:** the game's in-game `bugreport [note]` command writes a folder
  `bug-reports\<time>\` next to the saves, containing a screenshot, a state dump, the
  log and the note (`docs/PLAYTEST.md`).
- **Launcher:** Send and crash-report support merged in `D2RUST-Launcher` #9/#10.
  Launcher #12, sharing D2RUST's `d2-report` crate, is in review.
- **Sending:** intake deployment is shelved; manual delivery remains available.
- **Server:** `tools/report-service` intake code is on main (#70, retained by #229).
  It is separate from multiplayer, whose integration is in review (#165).

## Modules

| Rank | Id | Module | Tier | Size | Depends on | Status |
|---|---|---|---|---|---|---|
| 0 | B0 | Report format v1: a versioned bundle with size limits | light | 1 h | none | merged: #67, #164, #232 |
| 1 | B2 | Report intake service: upload endpoint, limits, storage | light | 3-4 h | B0 | shelved (implementation on main: #70, retained by #229) |
| 2 | B1 | Launcher: "Send bug report" with preview and consent | light | 2-3 h | B0, B2 | in review: D2RUST-Launcher #12 (initial support merged: #9) |
| 3 | B7 | Crash auto-report, opt-in (from D2RUST-crash.log) | full | 3-4 h | B1 | in review: D2RUST-Launcher #12 (initial support merged: #10) |

Ids follow the order of the path a report takes (B0 format → B1 launcher → B2 server); ranks follow the build order. B3-B6 were removed on 2026-10-17.

### B0. Report format v1

- **What:** a defined report bundle: `report.json` plus the files it lists.
- **report.json holds:** format version, game and launcher versions, OS, time, the note, and the file list with sizes and hashes.
- **Limits:**
  - total size at most 10 MB;
  - screenshot as PNG at most 4 MB;
  - log tail only (last 2,000 lines);
  - state dump in the game's versioned dump format.
- **Seam:** the game's `bugreport` writes this format. A small shared crate (or module) defines and validates it, and both the launcher and the service use it.
- **Privacy (B0a, merged: #164, #232):** the game's `bugreport` passes the note, the log tail and the state dump through
  `d2_report::Redactor` before it writes the bundle. The home folder (any slash style or letter case, also the
  escaped `\\` form) becomes `[home]`; the account name, as a word of its own, becomes `[user]`. The log can
  hold full paths (a new character's save path, for example), and those name the player. This is a second
  safeguard next to B1's (owner, 2026-10-17), so a report folder the player sends by hand is already clean.
- **Done when:** round-trip and limit tests pass; an oversized or malformed bundle is rejected with a clear reason.

### B1. Launcher: "Send bug report"

- **What:**
  - a "Report a bug" entry in the launcher lists the newest reports from the user folder;
  - the player picks one, sees exactly which files and text will be sent, can remove the screenshot or log, and clicks Send;
  - the launcher shows the report id it gets back.
- **Privacy:**
  - before sending, replace the Windows user name and home path in logs and dumps with placeholders,
    with the same `d2_report::Redactor` the game uses (B0a), run again on the bundle;
  - no other files from the PC are read.
- **Done when:** sending works against a test service, and a report with the user name in its logs arrives without it.

### B2. Report intake service (shelved)

- **Status:** shelved (owner, 2026-10-17). #70 supplied the implementation; #229 retained
  intake while removing triage. Deployment/domain selection is deferred.

- **What:** one HTTPS endpoint that accepts a bundle and returns a report id.
- **Limits it enforces:**
  - B0's size and format limits;
  - a per-install token issued on first launch, rate limited (for example 10 reports a day);
  - no executable files accepted, whatever their names.
- **Storage:** bundles go into private storage. Nothing from a report is ever served back publicly.
- **Seam:** a small standalone service (Rust, `axum`) that can run on BIGRIG or a small VPS until the game server exists, then moves next to it.
- **Secrets:** the forge token and storage keys live only on the service, never in the launcher.
- **Done when:** load and abuse tests (oversized, malformed, flooding, path tricks in file names) are all rejected; valid reports are stored.

### B7. Crash auto-report (opt-in)

- **What:** after a crash, the launcher offers to send `D2RUST-crash.log` as a report.
- **Rules:** off by default; the player switches it on once in settings; same preview and limits as B1.
- **Done when:** a forced crash produces an offered report, and declining sends nothing.

## Not planned

- **Sending reports from inside the game:** the launcher is the one place that talks to the network for this.
- **Public report lists:** reports can contain personal data, so they stay private.
- **Triage and automation pipeline:** turning reports into forge issues, an agent-readable sanitised form, an AI triage workflow, and "fixed in version x" notices. Removed from this plan (owner, 2026-10-17); this plan only receives reports.
