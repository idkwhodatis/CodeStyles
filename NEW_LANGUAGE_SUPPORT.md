# Dart / Flutter, Go and Vue support (local candidate)

This work is based on master `b7f7ed8`. It also incorporates the separately tested
Rust configuration-import and pack-size UI changes. No new language-pack release
or new application release has been published by this change.

| Language | Source | Formatter | Installation / settings |
| --- | --- | --- | --- |
| Go | `.go` | Official Go 1.27.3 `gofmt` | Optional native pack; fixed canonical style, no invented style controls, no `go fmt`, module downloads, goimports or simplification mode |
| Dart / Flutter source | `.dart` | Official Dart SDK 3.13.7 `dart format` | Optional minimal native runtime, not Flutter SDK; explicit languageVersion, pageWidth and trailingCommas |
| Vue SFC | `.vue` | markup_fmt 0.27.6 + TypeScript 0.96.1 + JSON 0.25.2 + Malva 0.16.0 | Optional WASM pack adds markup and style plugins; no Node/npm |

All three use the existing settings draft, preview, diff, review-before-write and
managed install/uninstall mechanisms. They use explicit **single-language**
project scopes. They never turn a project check into a package installation.
Existing ten bundled language variants remain offline and unchanged.

## Native runtime boundaries

Go has no configurable formatting styles: a nonempty `gofmt` section is rejected.
Dart settings live in `dartfmt`. Its invocation reads source from stdin, with no
source filename, package discovery or recursive file traversal. The working/home/
configuration/temp directories are isolated; Dart analytics are suppressed and
Go telemetry is explicitly disabled within its private configuration directory.
The runtime is selected only from the verified managed catalog, never PATH.

Dart `languageVersion` defaults to `3.13`, `pageWidth` to 80, and `trailingCommas`
to `automate`. Preserving trailing commas requires language version 3.8 or newer.
Official source directives (`// @dart=...`, `// dart format width=...`) retain their
upstream meaning and can override these defaults; the UI does not promise to
rewrite or suppress them. `pubspec.yaml`, `analysis_options.yaml` and package
configuration are protected review metadata, **not automatically imported**.
Exported JSONC preserves the explicit CodeStyles settings; this is not a Flutter
SDK installation or IDE/toolchain configuration command.

## Vue coverage and deliberate rejection

Supported inline blocks: HTML template, JS/TS script (including setup), JSON
script, and CSS/SCSS/Sass/Less style. A tokenizer-based gate rejects Pug, Stylus,
unknown/custom blocks and external `src` rather than letting an upstream success
exit disguise partially unformatted contents. Syntax errors remain formatter
errors. Source ignore directives retain their normal intentional meaning.

The complete four-plugin chain is required. Managed Vue mode builds an isolated
transient configuration using only these verified local WASM paths, forces the
script formatter to dprint, and rejects inherited/overridden configuration. It
never runs arbitrary plugins from a user configuration. Original JSONC comments,
unknown fields and plugin references are preserved on disk; they are not silently
rewritten to install Vue. Vue `markup` and `malva` controls are separate groups.

## Distribution and evidence boundaries

All new download buttons remain disabled (`available: false`, no invented URLs).
Linux x64 Go/Dart local candidate archives are pinned for exact local import;
other native targets have explicit pending entries, not made-up hashes. The same
Vue WASM candidate can run on all six hosts, but cross-platform native execution
has not been claimed from Linux testing alone.

Official SDK metadata covers Windows/macOS/Linux x64/ARM64. Building and testing
the other five native runtime bundles, their closure/ABI audits and publication
require subsequent work. Dart's AOT snapshot includes transitive packages;
complete third-party license auditing remains a **release blocker**, explicitly
recorded in its candidate provenance. The Vue plugins include their upstream MIT
notices, but their statically linked Rust dependency notices also need a target/
feature-aware audit before release. Neither top-level MIT nor a Cargo.lock alone
proves that all required third-party notices are present. Do not publish these
test candidates as license-complete releases.

Local `strace` was denied `PTRACE_TRACEME: Operation not permitted`. No bypass or
escalated trace was attempted. Clean-environment formatting and private-file
snapshots are valid evidence, but are not proof of zero network syscalls.
Minimum OS, manual GUI, and cross-version-upgrade acceptance remain separate.

## Local validation (2026-10-30)

Against this final local candidate on Linux x64:

- Core release tests: **199 passed, 6 ignored**, no failures. The three new-pack
  acceptance tests were then explicitly run separately: **3 passed**, covering
  verified installation, real formatting and idempotence, syntax errors, project
  check, selected review write, automatic recheck and uninstall for Go/Dart/Vue.
- Python packaging/distribution tests: **132 passed, 1 skipped** (133 discovered).
- Strict core Clippy and desktop Clippy: passed with warnings denied.
- Desktop `cargo check --bin codestyles --tests`: passed. Desktop-only tests were
  compiled, not linked/executed; this is not manual GUI acceptance.

The six ignored default-core tests include the three explicit pack tests above,
  two Rust runtime tests and one benchmark. Existing published Rust acceptance is
  separate prior evidence, not a fresh execution in this local run. No new
  cross-platform native execution or production download is claimed here.

Official references:
- https://dart.dev/tools/dart-format
- https://dart.dev/get-dart/archive
- https://go.dev/cmd/gofmt/
- https://go.dev/doc/telemetry
- https://dprint.dev/plugins/markup_fmt/
- https://github.com/g-plane/markup_fmt/tree/v0.27.6
- https://github.com/g-plane/malva/tree/v0.16.0

## Native probe preparation (not yet run remotely)

The manual `probe-native-formatters.yml` workflow prepares 12 native jobs: six
existing runner platforms for each of Go and Dart. Each job verifies the fixed
upstream SDK digest, extracts only the explicit minimal member shortlist, records
member hashes and dependency diagnostics, and runs real stdin-formatting,
idempotence and invalid-syntax smoke checks in a private environment. Missing
runtime libraries fail; the script never silently copies the whole SDK.

This first stage uploads reports only, not binaries. Its newly measured member
hashes require review before becoming a reproducible package plan. It does not
claim managed installation acceptance or approve releases. The second stage must
use reviewed pins to build candidates and run the managed install/project-write/
uninstall tests. No remote jobs were triggered by the local preparation.

The probe has nine offline contract tests (including bounded formatter output,
redirect refusal, native-host gating and failure evidence cleanup). The combined
Python suite after this addition is 141 passed and one skipped (142 discovered).
