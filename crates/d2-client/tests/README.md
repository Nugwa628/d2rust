# Client integration tests

The integration files in this folder are included by six explicit Cargo targets:

- `app`: `app_*.rs`.
- `front_end`: `front_*.rs` and `smoke_frontend.rs`.
- `properties`: `prop_*.rs`.
- `ui`: `ui_*.rs` and `game_*.rs`, plus the registration guard.
- `world`: the remaining top-level integration files.
- `mutants_client`: the existing mutation regression modules.

Keep files in place and register a new top-level file in the matching target's
`main.rs`:

```rust
#[path = "../app_new_feature.rs"]
mod app_new_feature;
```

The `ui` registration test rejects missing or duplicate registrations and prints
an actionable target path. Helpers belong in subdirectories with `mod.rs`; they
are not standalone tests. Each target declares its shared helpers once at the
crate root, and the included test files import them with `use crate::app_support;`
(or the appropriate helper name). This also preserves helpers included by other
crates without changing their crate-root imports.

Test functions keep their names; their listed name gains the source-file module
prefix. For example, select a single source file with:

```sh
cargo nextest run -p d2-client --features d2-client/dylib --test world -E 'test(/^scenario_smoke::/)'
```

The synthetic-install consumers use `test_fixtures::cache::copy_install`, with
`CARGO_TARGET_TMPDIR` as the cache parent. Archive builds are shared across test
processes by an input hash and OS lock. Each test gets separate archive copies.
The cache key also includes fixture, table, compiler and archive-writer sources,
and Cargo.lock. An interrupted build has no completion marker and is rebuilt.
`cargo clean` clears the cache along with other target artifacts.

For a local gate, `py -3 tools/ci/affected.py BASE` prints nextest `-p` flags for
changed packages and all their dependents, using the existing CI scope engine.
It includes committed changes since the merge base, tracked local edits, and
untracked files. Workspace build inputs select `--workspace`; docs-only changes
print nothing. **An empty result means skip the test command**, since invoking
nextest without package flags would select the whole workspace. The merge train
continues to run all packages.
