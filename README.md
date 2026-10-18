# CodeStyles

Native, offline-first code-style preview using Rust, GPUI Kit **0.7.3** and the
bundled **dprint 0.63.0** executable. This is the deliberately small P0 vertical
slice, not a full configuration editor.

## P0

- Native window with indentation (2/4 spaces) and TypeScript semicolon controls.
- Immutable TS/JS/TSX/JSX/JSON/JSONC samples beside formatted output.
- A 150 ms debounce, one background formatter worker, cancellation and revision
  gating. Stale results cannot replace the latest preview. The last successful
  output remains visible with an explicit stale label during updates or errors.
- CLI-only formatting using argv and three concurrently serviced pipes. Closing
  the view cancels, kills and reaps work. Errors and timeouts are displayed.
- Checksummed, pinned CLI, TypeScript 0.96.1 and JSON 0.25.2 WASM resources.
  The released application never downloads a formatter at startup.
- Draft project configuration is a unique, non-discoverable file beside the
  original config. Relative `extends`, ancestor `inherit: true`, `${configDir}` and plugin overrides are
  interpreted by dprint itself; original config/source files are never written.

Project configuration loading is backend-only in P0. The UI uses standalone
samples; file pickers, saving/export, schema-driven forms and project management
are later work. Preview supports local configuration files, not remote config URLs. Actual project source paths should be absolute; relative paths retain
dprint CLI working-directory semantics. The backend reads a project configuration
when its queued job starts, not when it is submitted. P0 does not yet snapshot
external configuration edits during debounce; the standalone UI has no external
config file and its setting values are copied into each request.

## Build and run

Install the pinned Rust toolchain (rustup reads `rust-toolchain.toml`), Python 3.9+
for resource preparation and your platform's GPUI native build requirements.
See [GPUI Kit installation](https://gpui-kit.com/docs/installation/).

For Linux x86-64:

```sh
python3 scripts/bundle_resources.py fetch --target x86_64-unknown-linux-gnu
python3 scripts/bundle_resources.py verify --target x86_64-unknown-linux-gnu
cargo run --locked
```

Other bundled targets: `aarch64-unknown-linux-gnu`, `aarch64-apple-darwin`, `x86_64-apple-darwin`,
`x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`. Build on the corresponding
native OS/architecture. Linux ARM64 is included. All six CI lanes use native hosted runners; each
asserts its Rust host triple before executing tests and building its package.

`fetch` is a build-time operation from pinned official sources. Binaries and ZIP
archives are ignored by Git; WASM, schemas, manifest and licenses are committed.
Do not ship a checkout without fetching and packaging its target executable.

The runtime finds `resources/` beside the executable, then the development
checkout. `CODESTYLES_RESOURCES` explicitly overrides that directory. It verifies
SHA256 before starting the bundled binary and never searches `PATH` for dprint.
The backend gives dprint a dedicated `CodeStyles/dprint` directory under the OS
user cache (LOCALAPPDATA on Windows, Library/Caches on macOS, XDG_CACHE_HOME or
~/.cache on Linux). `DPRINT_CACHE_DIR` explicitly overrides it, useful when the
normal user cache is read-only. A cold cache compiles bundled WASM locally without
network access. Config discovery is disabled; the selected draft is explicit.

```sh
cargo build --locked --release
python3 scripts/package_app.py --target x86_64-unknown-linux-gnu --output dist/codestyles-linux-x64.zip
```

Extract the whole ZIP, preserving executable permissions, and run `codestyles`.
The portable P0 ZIP is unsigned; native installers/signing are outside this slice.

## Verification

```sh
python3 -m unittest discover -s scripts -p 'test_*.py'
python3 scripts/bundle_resources.py verify --target x86_64-unknown-linux-gnu
cargo fmt --check
cargo test --locked --no-default-features
cargo check --locked
```

The backend test suite uses the actual pinned executable to compare preview
bytes with explicitly saved CLI configurations, including relative `extends`, ancestor `inherit: true`,
configDir and per-file overrides. It also tests source/config preservation,
cleanup, invalid configuration, timeout, full pipes and cancellation. GUI tests
require a working native display/GPU and are distinct from headless backend tests.
The CI matrix builds both x64 and ARM64 Linux, macOS and Windows packages; a workflow definition is
not evidence those builds or interactive GUI smoke tests have passed.

## Boundaries

Opening an existing project configuration can make dprint resolve that config's
plugins/extends, including remote references. The standalone P0 UI uses only the
bundled plugins. Only load project configs you trust. P0 does not sandbox arbitrary
third-party process plugins. A read-only project directory produces a clear error
rather than relocating the draft and silently changing path semantics.

See [resources/README.md](resources/README.md) for resource provenance and
[third-party notices](resources/THIRD_PARTY_NOTICES.md).

## CI runner selection

The six native runners are `ubuntu-24.04`, `ubuntu-24.04-arm`,
`macos-15-intel`, `macos-15`, `windows-2025` and `windows-11-arm`.
These are standard [GitHub-hosted runner labels](https://docs.github.com/en/actions/reference/runners/github-hosted-runners), not self-hosted hardware or paid larger
runners. macOS 15 follows GPUI Kit’s documented minimum. Every lane runs the
actual target dprint in the parity suite; none is a cross-compile-only claim.
Workflow success still does not prove interactive GUI behavior.
