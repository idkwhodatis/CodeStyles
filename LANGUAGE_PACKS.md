# Optional language support

CodeStyles retains its bundled offline dprint plugins, including Python/PythonStub
(Ruff). Existing configurations and portable packages keep working. Rust is the
first optional **native language pack**, not a WASM plugin, linter, compiler,
arbitrary executable runner, or agent coding-rules engine.

## Current availability

The Language support page shows installation status, location, progress, errors,
cancellation and an uninstall confirmation. With explicit user approval, six
reviewed Rust bundles were published on October 29, 2026 in the
[toolchains-rustfmt-1.99.1-r1 pre-release](https://github.com/idkwhodatis/CodeStyles/releases/tag/toolchains-rustfmt-1.99.1-r1).
This toolchain pre-release does not replace the application alpha release.
The updated embedded catalog enables only the fixed, hash-pinned GitHub assets;
older application binaries do not acquire this catalog update automatically.
“Install verified local archive” accepts only the target's catalog-pinned archive,
not arbitrary binaries or manifests. Five hosted native targets passed acceptance,
with Linux x64 separately verified locally. Production GitHub download/install
acceptance is a separate pending check, not implied by publication or those tests.
See [packaging and approval requirements](TOOLCHAIN_PACKAGING.md).

## Package sizes

Language support displays the selected platform's pinned compressed download size,
installed file payload, and the installation payload staging bound when the
manifest supplies them. These are distinct quantities: archives and the new
runtime coexist during installation. The staging bound excludes manager metadata,
filesystem allocation overhead, and already retained runtimes; it is not a
measurement of current free disk space or an exact total disk peak. Repair retains
the previous runtime until uninstall. Missing size metadata is shown as unknown,
not zero, and size information for an unpublished/local-only candidate does not
mean its network download is available.

## Storage

The default is `CodeStyles-tools` next to the executable (next to the `.app` on
macOS, never inside the signed app bundle). If that location cannot be used, the
manager falls back to the user's data directory: `%LOCALAPPDATA%/CodeStyles/toolchains`,
`~/Library/Application Support/CodeStyles/toolchains`, or
`$XDG_DATA_HOME/CodeStyles/toolchains` / `~/.local/share/CodeStyles/toolchains`.
An executable beneath the OS temporary directory or macOS AppTranslocation uses
the user location. ZIP launch detection is not universally reliable: set
`CODESTYLES_TRANSIENT_LAUNCH=1` for other transient launchers, or explicitly set
`CODESTYLES_TOOLCHAINS_DIR` to the desired managed directory. Restart after changing
these variables. The UI pins its selected location for the session rather than
silently uninstalling from a newly chosen fallback.

The manager refuses an unmarked nonempty directory and unsafe managed paths.
New Unix managed directories are private (0700); existing group/world-writable managed
directories are rejected. Windows uses inherited user directory ACLs; this is not
a promise of universal owner-only ACLs. Do not share a writable pack directory
with untrusted users. Installation verifies archive and individual file hashes,
rejects links/path traversal/unknown entries, bounds decompression, stages on the
same filesystem and atomically switches the active pointer. Directory fsync after
activation is best-effort crash durability; failure there never deletes an already
activated runtime. Cancellation or
pre-activation failure leaves the old active installation untouched. A shared
runtime lease prevents removal while formatting; uninstall only removes recognized
managed files. Unknown files and unrecognized old-version layouts fail closed.

## Rust configuration and workflows

Select Rust to use settings, live preview and Diff. Its supported `rustfmt` object
is an **application-native** JSONC section, not a stock dprint plugin section.
Export **rustfmt.toml** for normal editor/CI use; do not configure dprint as the
Rust default formatter. `edition` and `style_edition` default independently to
2021 and accept 2015, 2018, 2021 and 2024. Only the twelve documented stable
settings in `resources/schemas/rustfmt-stable.json` are accepted. Nightly options,
commands, paths and unresolved dprint inheritance/overrides are rejected.

The **Import rustfmt.toml…** action explicitly selects an existing TOML file and
merges only its specified supported stable settings into the current unsaved
JSONC draft. Settings omitted from the file retain their current values; unrelated
JSONC properties and comments are preserved. Unknown/nightly options, malformed
TOML, wrong value types, empty configurations, nonregular files and files larger
than 1 MiB reject the whole import without changing the draft. The selected file
is never modified. Apply or discard any unapplied raw/field editor input first;
late dialog results cannot replace a changed draft. Import is not an automatic
project-discovery or editor integration mechanism, and saving remains explicit.

Rust Project mode intentionally checks **Rust files only**. Existing dprint
Project mode remains unchanged; it is not a mixed-language discovery pass.
Rust mode respects project-local `.gitignore` and `.ignore` files (not parent,
global Git rules or `.git/info/exclude`), skips links, `.git` and `target`, and fails on
invalid ignore rules, nested dprint configurations or dprint includes/excludes
that it cannot faithfully resolve. Review, selected-file writes, source/config/
ignore staleness checks and automatic recheck are retained. Individual source and
process output are limited to 32 MiB. The formatter receives source via stdin in
an isolated directory, with fixed arguments, explicit config and a restricted
environment; it cannot recursively format sibling modules. No Rust installation,
PATH executable, rustup, network request or shell is needed to format after install.

## Remaining validation / release gates

- The current six-bundle release and catalog activation were explicitly approved.
  Future releases or changed pins still require approval. Production GitHub
  download/install acceptance is pending; an ignored opt-in integration test
  exercises that path separately from local candidate acceptance.
- All six target runtimes have native execution evidence: five hosted native
  jobs and Linux x64 local execution. Minimum-OS checks remain pending. Static
  runtime closure findings and remaining gaps are in `TOOLCHAIN_PACKAGING.md`.
- Desktop compilation/unit tests do not substitute for manual six-platform GUI,
  permission, relocation and packaging acceptance.
