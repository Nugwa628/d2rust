# tools/coord — session tools

Tools the cloud sessions use to talk to each other and keep their branches
current. Python 3 stdlib and POSIX sh only.

| Tool | What it does |
|---|---|
| `forge.sh` | the forge API: `read`/`say`/`watch`/`inbox` on `coord` threads, PR merge, remote set-up (`FORGE_REPO`, default `WerdeGames/D2RUST`) |
| `forge_watch.py` | the watcher behind `forge.sh watch`: prints other sessions' posts, catches up after a restart (`--selftest`) |
| `session-start-hook.sh` | SessionStart hook (`.claude/settings.json`): prints the thread rule and the open threads |
| `session-setup.sh` | cloud session set-up (toolchain, nextest, caches) |
| `env-setup.sh` | environment variables for the session tools |
| `sync.sh` | fetch the base branch (default `main`) and merge it into the current branch; stops on conflicts |
| `pre_pr.sh` | the checks to run before every push: tool selftests, fmt, and clippy + tests for the crates named |
| `seq_check.py` | no sequence number that main has not issued (`--selftest`) |
| `new_tools.py` | every new `tools/**/*.py` has a `--selftest` run in pre_pr or CI (`--selftest`) |

## sync.sh

    sh tools/coord/sync.sh [--base BRANCH] [--no-fetch]

Merge, never rebase. Exit 0 merged (or up to date), 1 stopped on conflicts
(resolve, then `git commit`), 2 error (dirty tree, fetch failed, usage).
