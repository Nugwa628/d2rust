# Fuzz targets (METHODS M07)

cargo-fuzz crate for the untrusted-input parsers of `d2-formats` and
`d2-data`. Not a workspace member; needs a nightly toolchain
(`rustup toolchain install nightly --profile minimal`, `cargo install cargo-fuzz`).

Robustness targets also cover `client_messages`, `server_messages`, `save_loader`,
`mod_pack` and `scenario` (the last needs `--features scenario-parser`). Their
committed `seeds/` corpora are synthetic and require no game installation. See
[`static-robustness.md`](../docs/tools/static-robustness.md) for the run commands,
seed generator, scope and Unverified finding policy.

```
cd fuzz
cargo +nightly fuzz build
# Seed corpora from your own game files (gitignored, never committed):
D2_GAME_DIR=... cargo +nightly run --release --bin mkseeds
cargo +nightly fuzz run dc6 -- -max_total_time=3600 -max_len=65536
```

Windows MSVC: the ASan runtime (`clang_rt.asan_dynamic-x86_64.dll`, in the
Visual Studio `VC\Tools\MSVC\<ver>\bin\Hostx64\x64` directory) must be on
`PATH`, or the target exits with `0xc0000135`. Run `animdata` with
`-max_len=400000` (the live file is about 300 KB).

Targets: `dc6 dcc dt1 ds1 cof tbl font_tbl palette pl2 animdata`
(`d2-formats`), `mpq_archive mpq_sector mpq_explode mpq_huffman mpq_adpcm`
(MPQ container and decompressors, through the `fuzz` feature of
`d2-formats`), `txt bin patch_layer` (`d2-data`). A crash input is
minimized (`cargo +nightly fuzz tmin`), then becomes a synthetic
`regress_*` unit test in the owning crate with the fix.
