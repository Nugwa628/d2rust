# T3 Code: what it is and how this project uses it

T3 Code is the app every session of this project runs in (LC, MC, lanes).
This file is the team's reference: what the app does, how its parts fit, and
how we use it. **[docs]** marks facts from the project's own documentation
(github.com/pingdotgg/t3code, `docs/user/`); **[seen]** marks facts verified
on our machines (T3 Code 0.0.45 Alpha, 2026-10-14). It is early software
("Expect bugs" [docs]): re-check a fact against the docs before building on
it, and fix this file when it is wrong.

## 1. What it is

- An open-source (MIT) control surface for coding agents you already have:
  Claude Code, Codex, Cursor, Grok Build, OpenCode, Google Antigravity
  [docs]. It runs no model of its own; each provider uses its own login and
  plan. We use Claude Code (Max plan) [seen].
- Clients: Electron desktop app, web app (app.t3.codes or the local
  server), iOS/Android apps [docs].
- Install: `winget install T3Tools.T3Code` (desktop) or the `t3` CLI
  (`irm https://t3.codes/install.ps1 | iex`); `t3 update`, `t3 --help` [docs].

## 2. Core model

| Concept | Meaning |
|---|---|
| **Client** | Shows the work (desktop, web, phone). Runs nothing. |
| **Environment** | One T3 Code server plus the machine it runs on. Owns the files, projects, threads, provider logins, settings, terminals, git, and state (SQLite in `~/.t3/userdata/`). The environment does the work. |
| **Project** | A directory on an environment, with its settings and an optional checked-in `t3.json`. |
| **Thread** | One durable conversation for one task, kept by the server after the client closes. |
| **Turn** | One message-to-finish cycle. T3 checkpoints the workspace around each turn (hidden git refs, not backups); a message-level revert restores files and conversation together. |
| **Provider instance** | One configured account of a provider (e.g. two Claude accounts). |

[docs, seen] Our environments: `DESKTOP-TSHGSN6` (the owner's laptop) and
`BIGRIG` (the team's always-on PC). A client can switch its machine's
**Local environment** off (Settings → Connections) and then only drive
remote environments [docs, seen].

## 3. Workspaces: current checkout vs worktree

- **Current checkout**: the thread works in the project directory itself,
  shared with every other thread there.
- **New worktree**: the thread gets its own git worktree and branch (shared
  object store), so parallel threads never edit the same files [docs].
  Location: `~/.t3/worktrees` by default, per machine under Settings →
  Storage → Worktree location [docs].
- `t3.json` in the repo can set repository defaults such as the workspace
  for new threads and `worktreeSubmodules`. Settings resolve in order:
  project override → environment setting → `t3.json` → built-in default
  [docs]. (We have no `t3.json` yet.)

**For us:** lanes always run as worktree threads (FINISH-LINE §11). LC and
MC each use their own checkout.

## 4. Permission modes

Supervised, Auto-accept edits, Auto, Full access [docs]. Approvals show as
inline cards that any connected client can answer. Plan mode makes the
agent plan before it implements. Our sessions run Full access (the owner's
choice; `runtime_mode = full-access` [seen]).

## 5. Remote access (how LC/MC are reached from anywhere)

| Route | What it is |
|---|---|
| **T3 Connect** | Sign in to the same account on each device and enable it per environment. A hosted relay handles setup, credentials and mobile notifications; a managed Cloudflare tunnel per environment carries the traffic, and client traffic does not pass through the relay [docs]. A host offline for minutes loses its tunnel and gets a new one when it comes back [docs]. |
| **Pairing** | `t3 pair` makes a one-time link or QR for LAN, private network or tailnet. Treat it like a password. |
| **Tailscale HTTPS** | Settings → Connections, or `t3 serve --tailscale-serve`. |
| **SSH (desktop only)** | The desktop app starts a server on a Linux or Apple-Silicon host. Not for Windows hosts. |

[docs] Load balancing places only **new** threads on another eligible
machine; existing threads stay where they started.

**Troubleshooting "not connected"** [docs]: on a CLI host,
`t3 service status` and its log (`t3 connect status` shows saved config
only); check clocks match on both devices (clock drift breaks link proofs);
update both ends; `auth_invalid` / `invalid_bearer` → `t3 connect login`.
[seen] 2026-10-14: the laptop's relay check-ins stopped at 16:39 after a
restart, and the fix was to turn T3 Connect off and on again (sign-out and
sign-in also works).

## 6. Keeping an environment running (important for BIGRIG)

- `t3 service install` runs the server as a background service, **on Linux
  and macOS only. "Windows background services are not supported."** [docs]
- BIGRIG is Windows: its T3 server lives only while the desktop app runs.
  BIGRIG stays signed in 24/7 with auto-sign-in (owner, #745), so keep the
  T3 Code desktop app open on it and set to start at login. If it closes, MC
  and LC stop until it is reopened.

## 7. Providers and accounts

- Claude: `claude auth login` on the environment; each machine has its own
  login. A new environment asks for it [seen: BIGRIG did].
- **Several Claude accounts** [docs]: one provider instance per account, each
  with its own config dir:
  `CLAUDE_CONFIG_DIR=~/.claude_2 claude auth login`, then Settings →
  Providers → add an instance with binary `claude` and **CLAUDE_CONFIG_DIR
  path** `~/.claude_2`. Threads switch only between instances that share a
  config dir. When a plan's limit is hit mid-turn, the thread shows the limit
  and the reset time and holds the turn [docs].
- [seen] Client settings include `loadBalancingEnabled` and
  `loadBalancingWeights` (off by default); the docs describe placement across
  machines (§5), not across accounts.

## 8. Source control and PRs

- Supported forges: GitHub, GitLab, **Forgejo**, Gitea, Bitbucket, Azure
  DevOps, GitCafe [docs]. **Forgejo uses the `fj` CLI** (with `tea` as a
  fallback), so `fj` must be logged in to git.werdegames.com on each
  environment that should see PR state (`fj auth login`). With one `fj`
  server configured, `owner/repo` works; otherwise use full URLs.
- Pull requests page: review, reviewers, checkout, merge; threads can be
  linked to PRs; a thread can **settle** by itself when its linked PR merges
  [docs]. "Viewed" marks for Forgejo are kept on the T3 server [docs].
  Auto-merge in T3 is listed for GitHub, GitLab and Azure only; we use
  Forgejo's own (`tools/coord/forge.sh automerge`, M27).
- Diffs per turn, per branch and of the working tree; a commit guard asks
  before committing to the default branch [docs].
- **Rule for agents:** link every PR you open to your thread
  (`link_pull_request`, §9), so T3 shows its state and settles the thread.

## 9. MCP: agents talking to T3

- **Inside a thread** [seen]: T3 gives the agent tools under `t3-code`:
  `link_pull_request`, `list_thread_pull_requests`, `unlink_pull_request`,
  browser preview (`preview_open`, `preview_navigate`, `preview_click`,
  `preview_snapshot`, recording, …) and device tools.
- **Outside agents** [docs]: each environment serves MCP at
  `<environment address>/mcp`. An agent connected there can read projects
  and threads, list providers and models, and, with Supervised or Full
  access, **start, message and stop threads**. Setup for Claude Code:
  `claude mcp add --transport http t3 https://<environment-address>/mcp`,
  then `claude mcp login t3`; approve in Settings → Connections; sign-ins
  last 30 days and can be revoked there.
  **For us:** MC could start lane threads on BIGRIG and message them
  without the owner starting each by hand. Not set up or tested yet.

## 10. Other features

- Terminals run on the environment, not the client; a file browser and
  editor with line comments sent to the agent; a browser preview panel; an
  Agents panel for subagents [docs].
- Usage page: local token and model usage with API-equivalent cost estimates
  (not subscription billing) [docs]. In a thread, `/usage` and `/context`
  show plan limits and context [seen].
- Composer: up to 120,000 characters plus images; `/` commands, `$` skills;
  Ctrl+Enter sends to a background thread [docs]. The command palette
  searches threads across connected environments [docs].
- [seen] Settings worth knowing: `contextWindowMeterEnabled` (a live
  context meter, off by default), `notificationMode`, `planModeEnabled`,
  `followUpBehavior` (queue), `sidebarWorkingShelfEnabled`.

## 11. How this project uses T3 Code

- **MC** (main coordinator) and **LC** (owner's helper) run as threads on
  the **BIGRIG** environment. The laptop is a client (Local environment can
  be off). See `CLAUDE.md` "Where work runs".
- Sessions talk through `coord` threads on the forge, not through T3
  (`tools/coord/forge.sh`, FINISH-LINE §5). T3's outside-agent MCP (§9) is a
  candidate for MC to start and steer lane threads itself.
- Lanes: worktree threads on BIGRIG (§3), each with its own coord thread.
- Every agent links the PRs it opens to its thread (§8).

Open items:
1. Keep the T3 desktop app open and set to start at login on BIGRIG (§6).
2. Log `fj` in on BIGRIG so T3 shows Forgejo PR state (§8).
3. Try the outside-agent MCP for MC to start lane threads (§9).
4. Decide on a `t3.json` for D2RUST-MAIN (new threads in worktrees by
   default, submodules setting) (§3).

Sources: github.com/pingdotgg/t3code (README; `docs/user/`
source-control.md, providers-claude.md, remote-access.md,
background-service.md, project-settings.md, outside-agents.md);
flaviocopes.com/t3-code (architecture overview); our T3 install
(`~/.t3/userdata`, logs, settings), 2026-10-14.
