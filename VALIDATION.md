# UI simplification validation — 2026-10-19

- `cargo test --locked --offline`: 76 library, 12 native UI/display,
  19 real-backend, and 11 project tests pass on Linux x86-64.
- `cargo clippy --locked --offline --all-targets -- -D warnings`, formatting,
  desktop build, and `git diff --check` pass.
- New tests cover exhaustive category membership for every schema field across
  all eight dialects, populated JetBrains-style tabs with no All tab or repeated
  category headings, unique compact labels, original-key and readable-label
  search, and display-only syntax highlighting in both themes with Unicode-safe
  ranges and empty-output reset. Raw-field hover help retains explicit values
  separately from schema defaults, including Unicode and unset states.
- The formatted preview now renders highlighted static text, with a default
  pointer and no input/caret handler. Copy retains the exact full output.
  Category and value editing behavior remains covered by existing state tests.

# P4 validation — 2026-10-18

## Executed locally on Linux x86-64

- Native optimized release build, `cargo fmt --check`, all-target clippy with
  warnings denied, and `git diff --check` pass.
- 72 library tests, 18 real preview-parity tests, 11 project tests and 7 native
  UI-state tests pass. New tests exercise resource lookup and the real eight-
  language/four-plugin headless diagnostic path.
- Actual release tar.gz and actual extracted Debian payload passed cold-cache
  formatting/idempotence checks in Unicode/space installation and working paths.
  Installation write probes were denied and content/metadata snapshots stayed
  unchanged. Fresh formatter caches were created outside installation trees.
- The release executable copied alone to an unrelated directory returned exit 1
  and a missing-resource JSON report, proving it cannot use the developer checkout
  fallback. Malformed diagnostic arguments and failure reporting are tested.
- 73 Python tests pass for package structure/permissions, Debian control metadata (independently checked
  by dpkg-deb), ELF/Mach-O/PE fixtures, native ELF/readelf comparison, signing
  sequencing, and safe extraction.
- Independent reviews corrected helper-specific DLL/dylib search paths, invalid
  imported-library types/architectures, Windows extended path comparisons,
  static Windows CRT linkage, privileged-container test semantics and temporary
  notarization-keychain selection. These are regression-tested where possible.

## Current boundaries

The local optimized build used the current cloud Linux environment, **not** the
old-ABI container; it is runtime-test evidence, not a glibc 2.28 artifact. The ABI
auditor correctly rejects such newer-host builds. Both native Linux release lanes
must build inside the pinned manylinux_2_28 images and pass the complete audit.
[Run 39178574499](https://github.com/idkwhodatis/CodeStyles/actions/runs/39178574499)
at `f55092458edd14f24d330277ec9cedc90bb3fc30` passed both native Linux old-ABI
lanes, including whole-payload ELF checks and genuinely network-disabled,
unprivileged release tar/DEB smoke tests. It also passed both native macOS app ZIP
and DMG lanes, including Intel 10.15.7 and ARM11.0 deployment audits. Windows x64/ARM64 built native releases and installers, then the
auditor exposed four missing system-DLL catalog entries. Exact Microsoft-
documented names were added with regression tests; VC runtimes still fail when
unbundled. Final six-target acceptance must use the corrected head, not this
partial earlier run.

Local runs use unreachable proxies, not genuine network isolation. The Linux CI
workflow separately repeats extracted-package tests in an unprivileged,
network-disabled, read-only container. Windows/macOS native installer and disk-
image execution, final whole-bundle audits and all six native lanes must pass on
the final head before acceptance. No credentialed signing/notarization run has
been performed. No public release or tag is published.

No GUI is initialized by `--self-test`. Oldest-OS launch, real system fonts,
clipboard, IME, scaling, graphics drivers, file-dialog integration and Windows
console-window behavior still require desktop acceptance. See
[DISTRIBUTION.md](DISTRIBUTION.md) for the exact manual matrix and for the
separation of compatibility goals, static audit evidence and runtime validation.

# P3 validation — 2026-10-18

## Executed on Linux x86-64

- `cargo fmt --check`, `git diff --check`, `cargo check --locked`, native debug
  build, and all-target clippy with warnings denied pass.
- 65 library tests, 18 real preview parity tests, 11 project integration tests,
  and 7 pure native UI-state tests pass.
- Python integrity/packaging: 7 tests pass; 11 local runtime resources verify.
- Eight-language preview parity includes pinned Markdown 0.26.0/TOML 0.9.0,
  all built-in examples, changed settings and nested fenced-code formatting.
- Real dprint project coverage includes NDJSON check results/exit 20, no-write
  checks, explicit selected/all formatting, saved config gates, spaces/Chinese
  paths, LF/CRLF, nested configs, literal metacharacter/leading-dash names,
  local extends preservation, plugin preparation, missing plugins, refused
  downloads, live-download cancellation and source/config preservation.
- Unix fixture coverage verifies timeout and descendant-held pipe cleanup.
  Windows uses a suspended process assigned to an owned Job Object before resume;
  a Windows-only compiled regression fixture covers descendants that keep pipes
  open after their CLI-shaped parent exits. Native Windows CI must establish it.
- Review fixed Save As report invalidation, added safe settings-JSON copying,
  and closed formatting-protection gaps for local file-URL dependencies and
  unresolved remote/local dependency graphs. These forms now fail closed before
  writing; read-only Check can still use dprint's ordinary resolution. Local
  plugin files and descriptors are also excluded from formatting.

Initial six-target CI found a Windows-only dependency protection issue: expanded
`${configDir}` paths used the canonical verbatim prefix with forward slashes.
The fix normalizes dependency separators for Windows filesystem lookup; the
existing local-plugin protection regression covers this exact case. Every native
lane must rerun on the corrected head before acceptance.

## Boundaries and acceptance

Formatting is not transactional. Cancellation/error can leave already-formatted
sources changed. No source snapshot rollback is promised. Configuration/source
preflight checks do not lock out unrelated external writers between checks and
CLI execution; do not modify the project concurrently. Trust is not sandboxing:
user-approved process plugins have the user's account permissions.

Project writes currently require a locally resolvable configuration dependency
graph: ordinary relative/absolute paths and `${configDir}` are supported.
Remote/package extends, file URLs, home-relative dependencies and unresolved
placeholders are rejected for formatting rather than risking a configuration
rewrite. Preview and read-only Check retain their own dprint semantics. Path
representations ambiguous to dprint's stdin/glob/range protocol are rejected.

No native display/GPU session is available here. Interactive layout, focus,
file pickers, confirmation dialogs, IME, VS Code setup and actual clipboard use
are not established by headless tests or compilation. The six native CI lanes
must pass for the exact P3 head; they still do not prove interactive acceptance.

# P2 validation — 2026-10-18

## Executed on Linux x86-64

- `cargo fmt --check`, `git diff --check`, `cargo check --locked`: passed.
- `cargo test --locked --no-default-features`: 51 unit tests and 15 real-CLI
  integration tests passed.
- `cargo test --locked --bin codestyles`: 6 pure UI-state tests passed, including
  stale comparisons, raw-draft value synchronization, malformed/duplicate path
  presentation, pending scalar input protection, visible newline differences,
  dirty protection and asynchronous dialog revisions.
- `cargo clippy --locked --all-targets -- -D warnings` and native debug
  `cargo build --locked`: passed.
- Python resource/packaging tests: 6 passed; Linux bundle: 5 runtime files verified.
- All 18 built-in source combinations (three samples × six languages) formatted
  successfully with the actual bundled dprint and WASM plugins.
- Schema coverage includes all 201 pinned properties, all 200 supported-property
  set/unset round trips, chained/escaped/cyclic/unknown `$ref`, typed enum/const,
  numeric constraints, unsupported raw fallback, multiword search, strict identity,
  comments/unknown fields, malformed paths, and explicit reset versus removal.
- Comparison coverage includes identical source/path inputs with separate immutable
  saved/draft snapshots, cancellation during either side, newest-revision gating,
  independent failures, successful/failed Save baseline advancement, custom-source
  preservation and bounded-memory diff fallback for large repetitive input.
- Native dependencies were extracted from checksum-verified official Debian
  packages into the build workspace; no app dependency versions were changed.

## Interactive acceptance still requires a desktop

No native display/GPU session was available. Compilation and pure state tests do
not establish visual quality, actual keyboard focus traversal, IME behavior,
scroll-position retention, dialogs or interactive file-picker behavior. These
must be smoke-tested on a desktop. The six native CI lanes must pass for the
exact P2 head; prior P1 successes do not establish P2 platform acceptance.

Manual checks: switch light/dark at narrow and wide sizes; search and Tab through
settings; apply/reset/unset values; apply raw edits to a selected setting; inspect
malformed paths; retain custom Unicode source while changing groups, languages
and options; switch built-in/custom repeatedly; compare saved/draft output and
save to advance baseline; interrupt previews; exercise trust/open/save/close
Cancel and late-dialog paths; verify IME composition and source/preview reading
positions. Source text is session-only, and close warns before losing it.

P1 trust, lossless CST, path semantics, immutable snapshots and atomic-save
limitations remain. Diff is line-oriented; enormous repetitive unanchored regions
may appear as a whole replacement instead of a minimal diff. Schema defaults are
not resolved effective values; unsupported constraints are raw-only, and dprint
remains authoritative. Initial untitled documents compare against their initial
configuration until the first successful save.

# P1 validation — 2026-10-18

## Executed on Linux x86-64

- `cargo fmt --check` and `git diff --check`: passed.
- `cargo test --locked --no-default-features`: 24 unit tests and 15 real-CLI
  integration tests passed.
- `cargo test --locked --bin codestyles`: 6 pure UI-state tests passed (schema
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
