# P1 validation — 2026-10-18

## Executed on Linux x86-64

- `cargo fmt --check` and `git diff --check`: passed.
- `cargo test --locked --no-default-features`: 24 unit tests and 15 real-CLI
  integration tests passed.
- `cargo test --locked --bin codestyles`: 4 pure UI-state tests passed (schema
  identity, stale-dialog guard, raw/invalid dirty state, revision-based freshness).
- `cargo check --locked`, `cargo clippy --locked --all-targets -- -D warnings`,
  and native debug `cargo build --locked`: passed.
- Python integrity/packaging tests: 6 passed. Linux runtime bundle: 5 resources
  verified without network access.
- Fresh-cache parity run with unusable HTTP/HTTPS/ALL proxy settings passed,
  including portable standalone export/reopen using bundled plugins offline.
- CST coverage includes comments, untouched/unknown fields, Unicode boundaries,
  explicit defaults versus deletion, duplicate-key and invalid-edit rollback.
- Save coverage includes external modification/deletion, final-check conflicts,
  temporary cleanup, canonical/symlink paths, permissions and no-clobber Save As.
  Save-time validation covers runtime-compatible core indentation and confirmed
  TypeScript semicolon values without assuming unknown plugin schemas.
- Preview coverage retains six-language CLI parity, relative extends/configDir,
  ancestor inherit, overrides, immutable queued config, A-B-A source stability,
  revision cancellation, failure retention, timeout and close cleanup.

## Scope and remaining boundaries

P1 covers JSONC CST editing, saved/draft state, immutable queued configuration,
project import/trust, raw editing and conflict-aware same-directory saves. It does
not introduce a full schema-driven form or recursively snapshot inherited config
and plugin files. Compare-before-replace cannot lock out unrelated external
writers during the final filesystem race window. Window-close and the app-owned
Quit action route through the dirty-draft guard. GPUI native OS/Dock/session
termination can bypass that guard; unsaved drafts are not crash-persisted.

No native display server is available in this environment. Interactive file
pickers, trust/discard prompts, raw-editor focus and layout, and visual quality
must be smoke-tested on a desktop. Compilation and headless tests do not establish
that interactive acceptance. The six native CI lanes remain required for the P1
commit; earlier P0 success is not evidence for new P1 platform builds.

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
