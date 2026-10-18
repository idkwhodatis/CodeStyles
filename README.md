# CodeStyles

Native, offline-first code-style preview using Rust, GPUI Kit **0.7.3** and the
bundled **dprint 0.63.0** executable. P2 adds native schema-driven style controls,
editable source, and saved-versus-draft comparisons on the P1 lossless editor.

## P2 style tuning

- Search the pinned TypeScript 0.96.1 and JSON 0.25.2 schemas by setting name,
  description or group. Select a group and then a setting. All 185 TypeScript
  properties and 16 JSON properties appear; unsupported complex fields explicitly
  direct you to JSONC instead of approximating their schema.
- Native controls support local `$ref`, boolean, number/integer, string, typed
  `enum`, `const` and finite scalar `oneOf` choices. Unsupported constraints,
  remote/cyclic references and complex values fail closed to raw editing.
- Filled dots identify explicitly written values. Reset writes the declared
  schema default; Unset removes the property. The displayed schema default is
  documentation, not a claim about effective settings: global options, presets
  and inheritance may change what dprint uses. Existing global settings remain
  accessible in JSONC. Unknown fields and comments survive form edits. Pending
  scalar input is kept until Apply or Discard; it blocks Save and setting/category/
  language transitions and participates in open/close discard confirmation.
- Unknown plugin identities/versions, mixed unknown plugins and unresolved
  inheritance disable typed controls. Raw JSONC remains available. Supported
  known-version values are checked before Save; dprint supplies runtime diagnostics.
- Choose a language and a built-in Overview, Collections or Comments example,
  or edit Custom source. Group changes choose a relevant built-in example only
  in built-in mode. Custom bytes survive language, category and setting changes,
  as well as switching to a built-in example and back. Source is session-only;
  closing warns when a custom buffer would be lost.
- Preview formats the draft. Diff compares the saved configuration baseline
  against that draft using the exact same source and logical source path, never
  an earlier formatted output. Successful Save advances the baseline and queues
  a fresh pair. Invalid or cancelled pairs never masquerade as current comparisons.
  Both sides run serially off the UI thread under the same cancellation token.
  Changed diff rows annotate LF, CRLF or missing final newline explicitly.
- Review saved baseline shows the exact saved JSONC beside the current editor.
  Trust approval covers both the saved and draft configuration, because either
  may resolve resources or execute process plugins. Save As to a new location
  revokes that trust, preserving P1 path safeguards.
- Light/dark theme switching uses GPUI Kit theme colors. Persistent editor entities
  and named scroll panes retain source selection and reading position across
  setting changes. Tab navigates native controls; Enter/Space activate buttons,
  Enter applies the value input, Ctrl/Cmd+Shift+F focuses search and Ctrl/Cmd+S saves.

## P1 configuration workflow

- Open a local JSONC config, inspect its raw draft, explicitly trust it, then preview.
- Saved text, editable draft and immutable sample source stay separate. Setting edits
  use the JSONC concrete syntax tree, preserving comments and unknown fields.
- Indentation and semicolon controls support removing a property (inherit/default)
  separately from writing an explicit value. Existing plugin references stay intact.
- Every queued preview owns its exact config text. External edits during debounce
  cannot change that job. Referenced extends/plugin files are still resolved by
  dprint at execution time, not recursively snapshotted.
- Save validates JSONC, the editable core option and supported known-schema
  values. Unknown plugin schemas and complex constraints remain raw-editable;
  dprint remains authoritative for full runtime validation.
- Save checks disk contents against the loaded baseline and refuses observed
  external changes. It writes a synced same-directory temporary file and atomically
  replaces the selected file. This is a compare-before-replace guard, not an editor
  lock: an external writer can still race the final check and replacement.
- Standalone drafts/export use pinned standard plugin URLs, never installation-
  specific paths. Preview alone maps those exact versions to verified bundled WASM;
  imported and saved text keeps its original references.
- Save As creates a new destination only. It never overwrites an unrelated existing
  file. Relative references retain their text, so moving folders can change their
  resolution; preview uses the new location after saving.

## Preview foundation

- Native schema controls for TS/JS/TSX/JSX/JSON/JSONC formatting.
- Built-in or editable custom source beside formatted output or saved/draft diff.
- A 150 ms debounce, one background formatter worker, cancellation and revision
  gating. Stale results cannot replace the latest preview. The last successful
  output remains visible with an explicit stale label during updates or errors.
- CLI-only formatting using argv and three concurrently serviced pipes. Closing
  the view cancels, kills and reaps work. Errors and timeouts are displayed.
- Checksummed, pinned CLI, TypeScript 0.96.1 and JSON 0.25.2 WASM resources.
  The released application never downloads a formatter at startup.
- Draft project configuration is a unique, non-discoverable file beside the
  original config. Relative `extends`, ancestor `inherit: true`, `${configDir}` and plugin overrides are
  interpreted by dprint itself. Editing and preview never write original config or
  source files; explicit Save writes the configuration only.

Project preview supports local configuration files, not remote config URLs. The
selected config may itself contain remote references. Sample source paths for an
imported project are placed beside that config to exercise its path semantics.
Original project source files are never written.

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
The portable ZIP is unsigned; native installers/signing are outside this slice.

## Verification

```sh
python3 -m unittest discover -s scripts -p 'test_*.py'
python3 scripts/bundle_resources.py verify --target x86_64-unknown-linux-gnu
cargo fmt --check
cargo test --locked --no-default-features
cargo test --locked --bin codestyles
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

Opening a project config only reads it. Previewing requires an explicit trust
action: dprint may resolve remote extends/plugins and execute process plugins.
Trust is not sandboxing. Raw configuration changes require renewed trust. Only
standalone/reopened configs using the exact bundled pinned plugins without external
references are guaranteed offline. Unknown plugin versions
are retained; plugin-specific controls are disabled for unsupported
references and inherited configurations. Raw JSONC editing remains available.
A read-only project directory produces a clear error rather than relocating the
draft and silently changing relative path semantics.

Window-close and the app-owned Quit menu/shortcut ask before discarding unsaved
drafts, including invalid raw text and session-only custom source. OS/Dock/session termination can bypass the
framework close guard; drafts are not crash-persisted. Save important edits first.

See [resources/README.md](resources/README.md) for resource provenance and
[third-party notices](resources/THIRD_PARTY_NOTICES.md).

## CI runner selection

The six native runners are `ubuntu-24.04`, `ubuntu-24.04-arm`,
`macos-15-intel`, `macos-15`, `windows-2025` and `windows-11-arm`.
These are standard [GitHub-hosted runner labels](https://docs.github.com/en/actions/reference/runners/github-hosted-runners), not self-hosted hardware or paid larger
runners. macOS 15 follows GPUI Kit’s documented minimum. Every lane runs the
actual target dprint in the parity suite; none is a cross-compile-only claim.
Workflow success still does not prove interactive GUI behavior.
