# P0 validation — 2026-10-18

## Executed on Linux x86-64

- Rust 1.99.1, GPUI Kit 0.7.3, dprint 0.63.0.
- `cargo fmt --check`: passed.
- `cargo test --locked --no-default-features`: 4 unit and 11 integration tests passed.
- `cargo clippy --locked --all-targets -- -D warnings`: passed, including desktop UI.
- `cargo check --locked`: passed, including desktop UI.
- Native debug `cargo build`: passed; every linked library resolves with `ldd`.
- Python resource integrity tests: 3 passed.
- Resource verification: all 9 runtime files across 5 platform targets pass SHA256.
- Real CLI parity: TS, JS, TSX, JSX, JSON, JSONC across both controls;
  relative extends, ancestor inherit:true, configDir and per-file overrides.
- Source/config preservation, cleanup, timeout, blocked-pipe cancellation/reaping,
  revision gating, debounce, argv/environment handling and tamper rejection tested.
- Fresh-cache standalone TS and JSON previews passed with unusable HTTP, HTTPS
  and ALL_PROXY settings, using only bundled local WASM.
- A portable Linux debug ZIP was assembled with a stripped application executable
  and the verified dprint/WASM/schema resources. It is a development build.

This cloud environment had runtime XCB/XKB/fontconfig libraries but no development
pkg-config files/linker aliases. Validation used the dependency's supported
`RUST_FONTCONFIG_DLOPEN=1` option and workspace-local linker aliases via
`LIBRARY_PATH`. No system packages, permissions or security settings were changed.

## Not executed / limitations

- No display server is available: interactive window rendering, clicks, resize,
  closing, GPU behavior and visual quality have not been tested.
- macOS/Windows native builds did not run in this local environment; subsequent
  six-target CI builds are recorded below. Interactive GUI smoke tests remain unrun.
- The original local pass did not execute Linux ARM64. The expanded six-target
  native CI matrix and Linux ARM64 resource support were added afterward.
- Local validation used a debug build; CI subsequently built release packages.
  Signed installers and notarization are not covered.
- Project loading is a backend capability, not a P0 UI feature. A project's config
  is read when its queued job starts, not snapshotted at submission; external edits
  during debounce are not isolated. Standalone UI request values/source are copied.
- Existing project configs may resolve their own remote extends/plugins. Only the
  standalone first-preview path is guaranteed to use bundled local resources.

## CI follow-up

Run [39093760140](https://github.com/idkwhodatis/CodeStyles/actions/runs/39093760140)
passed backend tests, desktop checks, clippy, release builds and package upload on
Linux x64, macOS ARM64 and Windows x64 at commit 48705c5.
The expanded matrix additionally targets Linux ARM64, macOS x64 and Windows ARM64;
check the current commit’s run before treating those lanes as validated.
All lanes execute natively with an explicitly asserted Rust host triple.

## Six-target CI and review follow-up

Run [39095107565](https://github.com/idkwhodatis/CodeStyles/actions/runs/39095107565)
passed all six native OS/architecture lanes at commit
`2cce371b67552e29bfe343d47b67d553bb59f3b4`, including release packaging.
This is build/backend evidence, not interactive GUI acceptance.

The next review pass fixed literal leading-dash source paths in formatter argv,
cleanup of interrupted resource downloads, and deterministic license metadata in
portable packages. Archive member names use forward slashes on every build host.
Regression tests exercise these cases without downloading untrusted executables.
