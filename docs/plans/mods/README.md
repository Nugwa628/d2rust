# Mod plans

Plans for mod features, and later the pipeline that builds them, go here. Each one uses the same
front matter and ranked-module shape as every other plan (`docs/plans/README.md`).

The goal is that a mod feature can go from an approved plan file to merged PRs with as little
hand work as possible:
1. a tool reads the front matter;
2. it picks the next ready module (all `depends_on` done);
3. it starts a session with that module's section as its brief;
4. the merge train lands the PR.

Each mod is its own game: its own binary built on the D2RUST crates, its own files and saves, launched directly.
The D2RUST exe never loads or selects a mod. Mod games share D2RUST's multiplayer architecture.

Rules that already apply to mods (see `CLAUDE.md`):
- Mod games change behaviour only behind `Ruleset::Mod` switches they install, or as data patches applied to the player's own tables at load time.
- Mods are judged against D2RUST's own behaviour.
- Mods may ship their own assets and files. They don't need the original MPQ data.

## Plans

- [Mod seams](seams.md): the override points mod games install (`Ruleset`, providers, hooks, game builder, shared server); in progress.
