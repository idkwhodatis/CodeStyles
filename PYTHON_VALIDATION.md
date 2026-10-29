# Python formatting integration — 2026-10-28 (Toronto)

Base: `521c51aa6fb2907a0a82afb6e9e5cb66ce2209a3`.

## Design and scope

- Use the official dprint Ruff adapter **0.11.3** WASM, not a new process backend.
  Its release WASM and schema digests match the upstream GitHub release assets.
  The existing dprint **0.63.0** executes both Python and Python-stub fixtures.
- Add `.py` / `.pyi` language choices, immutable examples, Python Tree-sitter
  highlighting, and the exact pinned schema through existing forms and JSONC.
  Reuse preview cancellation/revision gating, A/B diff, project review/selection,
  protected files, stale review checks, and post-format verification.
- No Python interpreter, pip, or separately installed Ruff dependency. The WASM
  adds about 12.1 MB uncompressed to all six platform resource bundles. The
  native application adds only the GPUI Kit Python grammar feature.
- Export remains portable dprint JSONC. Existing configurations are not silently
  changed. Missing Python plugin controls explain the exact URL to add.
- No new lint/agent-rule/notebook feature. Existing explicit `fixLintErrors`
  configuration is preserved; new defaults do not enable lint/import changes.
  Standalone Ruff `pyproject.toml` / `ruff.toml` settings are not imported.

## Review findings addressed

The upstream schema declares `indentWidth` and `lineLength` as unconstrained
numbers, but the pinned adapter parses unsigned u8/u16 values. The form model
now supplements those two fields with exact integer representation/ranges,
including rejection of decimal JSON encoding, without changing upstream schema
bytes or other plugins' JSON Schema semantics. Zero remains allowed, matching
the adapter. Tests cover both input parsing and document validation.

Old four-plugin configurations retain their text and existing controls. The
missing-plugin message includes the exact portable reference and manual JSONC
instructions; no automatic upgrade or source write is introduced.

## Verification boundaries

Linux x64 local checks are distinct from the six-platform CI and interactive
desktop acceptance. This branch has not been pushed: Windows/macOS/ARM64 CI
has not run for these changes. No claim of Windows dragging/IME/GPU fixes or
manual GUI interaction is made. Old-OS compatibility still requires the existing
native matrix and manual acceptance process. Signing and publication are not
part of this change.

Commands use the pinned Rust toolchain, a dedicated writable dprint cache,
and the existing local Linux native libraries. Build jobs are limited to two.

## Final local results

- `cargo test --locked --no-default-features`: **134 passed**, one intentionally
  ignored microbenchmark (88 library, 1 shell integration, 19 real preview parity,
  20 project workflow, 6 focused Python integration tests).
- `cargo test --locked --release --bin codestyles`: **14 passed**, including
  real syntax-highlight spans for Python and stubs in both themes.
- `cargo clippy --locked --release --all-targets -- -D warnings`: passed.
- `cargo fmt --check`, `git diff --check`: passed.
- `cargo build --locked --release`: passed on Linux x64.
- Python packaging script suite: **109 tests, 1 platform-specific skip**.
- Manifest verification: **14 runtime resources** verified.

Focused integration covers `.py`/`.pyi` bytes against direct dprint, quotes,
line length, tabs/CRLF, A→B→A, invalid syntax without writes, default preservation
of unused/unsorted imports, mixed Python/TS projects, exclusions, nested config
inheritance, reviewed selection, automatic recheck, stale-review rejection and
cancelled writes. These tests invoke the actual pinned formatter, not mocks.

The actual new release executable was packaged as a Linux x64 tarball and passed
`scripts/smoke_package.py --require-readonly`: all ten languages/five plugins,
changed output plus idempotence, Unicode/space install paths, unrelated cwd,
fresh external cache, verified write denial, and an unchanged installation tree.
The smoke used unreachable proxy settings, **not genuine network isolation**,
and this host's prepared native library path; it is not a manylinux ABI or GUI
acceptance claim. Six-platform CI retains the stronger native distribution gates.

Local evidence: `/tmp/codestyles-python-final-core.log`,
`/tmp/codestyles-python-clippy.log`, `/tmp/codestyles-python-packaging-tests.log`,
and `/tmp/codestyles-python-smoke.json`.
