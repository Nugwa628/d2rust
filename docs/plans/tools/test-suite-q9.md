# Q9. Static and robustness checks

Status: done except the `scenario` fuzz run (2026-10-18). Four decoder targets ran clean; `scenario` builds but its fuzz build does not link on Windows (follow-up below).

This records the Q9 section of the owner's Automated test suite plan. The
implementation and commands are described in
[`static-robustness.md`](../../tools/static-robustness.md).

- Table checks: a read-only `data-check [ROOT] [--mod NAME]`, using production
  schema linkers and the effective source's asset membership. Named mod runs
  include declared dependencies only. Target: under one second on real files.
- Fuzzing: client/server messages, save loading, native mod-pack metadata and
  patches, and scenario parsing. Synthetic seeds only; CI build checks
  and sixty-second local runs. Longer runs belong to Q10.
- Rule properties: bounded damage/stat arithmetic, legal affixes and level
  requirements, compiled treasure weights, and path reachability/blocked cells.

Done when:

- [x] Every static rule has a synthetic planted-failure test (all 12 data-check tests pass).
- [x] The CI synthetic set passes local nextest and real-file findings are recorded as Unverified.
- [x] Real-file timing is measured.
- [ ] Each fuzz target runs sixty seconds clean, or the toolchain gap is recorded.
- [x] Formatting, strict all-target clippy and touched-crate nextest gates pass.

All runtime/data/property findings remain **Unverified**. This work does not
change game rules or repair real tables. Detailed local logs and crash inputs
belong under `E:/work/artifacts/cx-q9-static/`; only measured counts belong here.

BIGRIG real-data run (release, 2026-10-18): 85 tables, 387.346 ms for
reading/checking tables and 474.331 ms total, exit 1. **Unverified** candidates:

- 2 duplicate-id reports for `monstats.txt` line 726, `Id=cr_lancer8`
  (the same row is registered in two linker namespaces).
- 20 unresolved missile `EType` references containing a space.
- 13 unresolved unique-item property references beginning with `*`.
- 1 unresolved monster-equipment sentinel reference.
- 1 missing misc-item drop graphic (`flptail.dc6`).
- 34 missing sound-file references.

These are 71 reported candidates (70 distinct messages), not verified data bugs.
No table or game-rule changes were made. Full per-cell results:
`E:/work/artifacts/cx-q9-static/round2/data-check-release.log`.
An earlier run used a stale debug executable and is excluded from these results.

Initial property candidate (**Unverified**, harness assumption):
`prop_walk::astar_serpentine_reaches_far_end`, minimized `w=3, h=3, flip=false`,
returned count 4 versus stored `point_count=0`. Initial nextest: exit 100,
5,411 passed, 1 failed, 139 skipped. The added assertion reused a `compute()`
helper after direct A*. Only `compute()` publishes the returned count into the
path metadata. The test now checks A*'s returned points and every segment directly.
No engine code changed. This initial failure does not establish an engine defect.
The initial log and minimized parameters remain in the local Unverified report.

Completed local fuzz runs so far: `client_messages` 911 executions / 61 s, exit 0;
`server_messages` 1,342,861 executions / 61 s, exit 0;
`save_loader` 702,132 executions / 61 s, exit 0;
`mod_pack` 88,042 executions / 61 s, exit 0.

Final touched-crate nextest (after the harness correction): exit 0, all 5,412 tests
passed, 139 skipped, 357.238 s including resource-guard suspensions. All six new
rule properties and the strengthened path property passed. Main-workspace and
fuzz-workspace formatting checks exit 0. The complete fuzz target compile check
(`--bins --features scenario-parser`) passed. Strict all-target main-crate and fuzz Clippy passed (exit 0 each).

`scenario` fuzz run: not run. `cargo fuzz run scenario --features scenario-parser`
fails at link time on Windows: the target pulls `d2-client` with its `dylib`
feature, and the instrumented release build of `bevy_dylib` ends in
`LNK2001: unresolved external symbol main` (twice, once with sccache off). Follow-up:
build the scenario target against `d2-client` without `dylib`, or run it on the
Linux farm (Q10).
