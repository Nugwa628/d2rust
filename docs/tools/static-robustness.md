# Static and robustness checks

`data-check [GAME_DIR]` reads the highest-priority excel text and animation metadata,
checks schema linker references and duplicate keys, numeric skill/level/preset ids,
monster mode COFs and AnimData entries, item inventory/drop graphics, missile sprites,
sound files and level tile files, and preset-to-level references. It only checks asset
membership; graphics and sounds are never decoded. Exit 0 means no findings; exit 1
means findings or a load error. Every finding is an **Unverified** candidate, with its
source table, line, column and actual value. Nothing repairs tables or game logic.
Uniqueness follows the production patch schema: sequence rows may repeat a key.
Monster knockback uses GH; non-composite death uses hth. Sequence mode draws its
constituent modes and has no standalone SQ asset requirement.
Unused content is legal: an unreferenced row alone is not an error. An orphan means
a named reference with no matching key. A duplicate name key is info for the rows
after the first: the linker keeps the first index, so those later rows are
unreachable (the shipped tables have one, `monstats` `cr_lancer8`). Numeric
skill/level/preset id duplicates stay findings because the id itself is the key.

`data-check NATIVE_ROOT --mod NAME` checks the patched effective tables and assets
for that mod and its declared dependencies in dependency order. Other enabled mods
are excluded. Dependencies, metadata and patches use the production native loader.
The selected mod need not be in `mods/order.toml`. Invalid names and cycles fail.
Native conversion/patch compilation is included in total time and can exceed the
static-check budget; the tool reports total and table-check time separately.

Build through BIGRIG's build queue:

```sh
sh C:/Users/USER/.t3/projects/lc/scratch/build-slot.sh cargo build -p data-tool --bin data-check
./target/debug/data-check
```

The properties in `prop_rule_ranges`, `prop_drop_weights`, `prop_rule_totals` and
`items::tests::prop_legal_affixes` derive bounds from production code and synthetic
tables. Damage multiplication and additive totals are restricted to domains without
signed wrapping. Crafted affix requirements cap at 98 before base/socket/skill/stat
requirements. Drop tests compile actual treasure rows and exhaust their roll range,
including expansion-only items. `prop_walk` compares A* against an independent BFS
when the reachable component fits its 200-node budget and checks walked cells;
its corridor cases now check every segment as well. Increase `PROPTEST_CASES` for
longer runs. Property failures are candidates to preserve and investigate, never
permission to change game rules.

The five new cargo-fuzz targets live in `fuzz/fuzz_targets/`:

- `client_messages`: protocol sizing/classification, local queues and server dispatch
  into the existing synthetic wired cube/trade hosts and real handlers.
- `server_messages`: message sizing, buffer splitting and the client-side S2C parser.
- `save_loader`: production save loader in classic/expansion modes, raw bytes and
  checksum-repaired mutations, using synthetic save stat tables.
- `mod_pack`: native manifest/mod metadata/order and patch layer/stack parsers.
- `scenario`: the production scenario parser and canonical writer round trip,
  including input/check grammars; enable `scenario-parser` for this target.

The committed `fuzz/seeds/` files are synthetic. `fuzz/corpus/` remains ignored for
local game-derived corpora; never copy those into seeds. Regenerate the three valid
save seeds with `cargo run --bin mkrobustseeds` from
`fuzz/` (also through the queue). The generator verifies each seed with the loader.

On a host supporting nightly libFuzzer/sanitizers, from `fuzz/`, run each target:

```sh
cargo +nightly fuzz run client_messages seeds/client_messages -- -max_total_time=60 -max_len=65536
cargo +nightly fuzz run server_messages seeds/server_messages -- -max_total_time=60 -max_len=65536
cargo +nightly fuzz run save_loader seeds/save_loader -- -max_total_time=60 -max_len=65536
cargo +nightly fuzz run mod_pack seeds/mod_pack -- -max_total_time=60 -max_len=65536
cargo +nightly fuzz run scenario --features scenario-parser seeds/scenario -- -max_total_time=60 -max_len=65536
```

Use the build queue for fuzz builds/runs on BIGRIG and keep local artifacts under
`E:/work/artifacts/cx-q9-static/`. Set libFuzzer `-artifact_prefix` there. Minimize a
crash with `cargo +nightly fuzz tmin TARGET INPUT`, preserve the minimized input and
record the failing command as **Unverified**. Do not fix it in this module. A clean
60-second run is a smoke check, not proof that all input/state combinations are safe.
If this Windows toolchain cannot run libFuzzer, build-check all targets with
`cargo check --manifest-path fuzz/Cargo.toml --bins --features scenario-parser` and
record the runtime gap explicitly. Q10 owns longer farm runs.

PR CI only checks target compilation (`cargo check --locked --bins --features
scenario-parser`); it does not install cargo-fuzz or rebuild instrumented Bevy.
The local sixty-second runs and longer Q10 farm runs provide runtime coverage.
