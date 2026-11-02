# Standalone Web, YAML and SQL support

This development slice adds **HTML** (`.html`, `.htm`), **CSS** (`.css`),
**SCSS** (`.scss`), **YAML** (`.yaml`, `.yml`) and **SQL** (`.sql`) to the language
picker, offline samples/schema settings, preview/Diff, and selected-language
Project Check → review → Format selected workflow. It does not register OS shell
file-open associations or turn CodeStyles into a source editor.

## Runtime and pack boundaries

| Languages | Optional pack | Pinned formatter |
| --- | --- | --- |
| HTML, CSS, SCSS (and existing Vue) | Existing shared Vue/Web pack `markup-0.27.6-malva-0.16.0` | markup_fmt 0.27.6 + Malva 0.16.0 |
| YAML | `pretty_yaml-0.6.0` | Pretty YAML 0.6.0 |
| SQL | `sql-0.4.1` | dprint SQL 0.4.1 |

All use the separately managed **dprint `0.63.0-p1`** engine pack. Its dprint
0.63.0 engine and five TypeScript/JSON/Markdown/TOML/Ruff WASMs are unchanged;
the existing Vue pack's archive bytes and catalog pins are unchanged too. YAML
and SQL are independent universal data-only WASM packs, not additions to either
existing pack. Their manifests explicitly require `0.63.0-p1`.

Settings, schemas, samples and notices work offline without any installed pack.
Formatting needs the relevant verified packs; it does not search PATH, install a
system formatter, fetch a runtime silently, use Node/npm, or fall back to a
different formatter. Missing/corrupt packs produce actionable errors.

All six target entries for each new YAML/SQL candidate are **disabled**. Candidate
archives are not releases, and their presence does not enable downloads or
production local installation. Runtime publication and catalog activation require
separate approval and acceptance.

## Supported source and deliberate exclusions

- **HTML:** standalone documents/fragments, with inline JS/TS or supported JSON
  script blocks and CSS/SCSS style blocks. Composition uses the shared Web WASMs
  plus the pinned dprint TypeScript/JSON plugins. External `src`, dynamic block
  language/type, unsupported preprocessors, ambiguous duplicate attributes,
  self-closing script/style syntax and unclosed raw-text blocks fail closed.
  The app normalizes the formatter's `.htm` input hint to `.html` while preserving
  the original project filename; upstream `.htm` handling alone can be a no-op.
  This is not framework-template support or an HTML semantic validator.
- **CSS/SCSS:** standalone formatting through Malva. SCSS is not indented Sass,
  and this slice does not add standalone Less or Stylus. It does not run Sass,
  resolve imports, compile styles, or build a project.
- **YAML:** ordinary YAML, including formatter-supported comments, anchors,
  aliases and supported block scalars. **Keep-chomping block scalars (`|+`/`>+`,
  including optional indentation indicators) are rejected before formatting**:
  Pretty YAML 0.6.0 was observed collapsing a trailing newline in a `>+`
  semantic fixture. The conservative guard also rejects matching markers in
  quoted strings; this scope stays closed until the upstream issue is corrected.
  No Helm/Jinja/template rendering, custom-tag execution, application-schema
  validation or configuration deployment.
- **Templates:** the markers `{{`, `{%`, `{#`, `<%` and `<?` are rejected even
  inside comments and literal strings. These intentional false positives apply
  to all five new languages: the scope gate does not guess whether a template
  is active. Remove/render templates outside CodeStyles first.

## YAML safety override and external editor/CLI use

Pretty YAML 0.6.0 defaults `yaml.trimTrailingWhitespaces` to **true**. A real
semantic fixture showed this removes significant trailing spaces inside YAML
block scalar values. CodeStyles therefore **injects false into the ephemeral
formatting configuration**, rejects an explicit true value, and presents a fixed
false setting. This is an intentional application safety override, not the
upstream default. Keep-chomping scalars remain excluded even with trimming off.

The override **does not rewrite saved or exported JSONC**: if the property was
absent, it stays absent. Consequently, opening that same config directly in an
external dprint/editor integration can restore the unsafe upstream default.
Explicitly include this setting when using YAML outside CodeStyles:

```jsonc
{
  "plugins": ["https://plugins.dprint.dev/g-plane/pretty_yaml-v0.6.0.wasm"],
  "yaml": {
    "trimTrailingWhitespaces": false
  }
}
```

Merge the `yaml` property into the intended configuration rather than replacing
unrelated settings or plugin references. An external formatter does **not** gain
CodeStyles' template or keep-chomping source guards from this JSONC setting;
exclude unsupported inputs yourself. `false` addresses the verified trailing-space
case, not every possible YAML semantic issue. Review output before applying it.

## SQL: choose a dialect, not a database connection

An explicit dialect is required; **generic, missing and automatic dialect
selection are not supported**. The upstream schema enumerates seven named
choices: `postgresql`, `mysql`, `sqlserver`, `sqlite`, `oracle`, `bigquery` and
`snowflake`. An upstream enum value is not an application support guarantee:
the initial conservative source gate admits only **`postgresql`, `mysql` and
`sqlite`**. Other dialects are outside this initial formatting scope. The
committed official schema stays byte-exact; application controls project only
these three choices and remove the misleading upstream generic-default hint.

The formatter is **not a SQL validator**. For example, upstream formatting can
accept `SELECT FROM WHERE`; a successful preview or clean Check does not establish
valid syntax, equivalent query behavior, compatibility with a server version, or
safe execution. CodeStyles opens no database connection and executes no SQL or
network database request.

The `sql_scope` scanner excludes procedural/client scripts rather than attempting
to interpret them: stored routines/blocks, transaction `BEGIN`, client commands,
dollar quoting/parameters, executable MySQL comments and other excluded lexical
constructs fail closed. The scanner is deliberately conservative, not a complete
SQL parser. Review the proposed diff, especially when formatting database code.

## Configuration and project workflow

Working files remain **CodeStyles JSONC**. Schema settings use the pinned plugin
sections; comments and unrelated/unknown JSONC content are preserved. Existing
configurations are not silently upgraded. Native formatter config import actions
(such as rustfmt TOML or clang-format YAML import) are not YAML/SQL source import
or configuration conversion mechanisms.

The new managed project modes operate on the **selected language**, not all five
at once and not arbitrary mixed-language discovery. Save the configuration,
choose the language, run Check, inspect differences, then explicitly select and
confirm files to format. The existing trust, cancellation, busy-state leases,
source/config staleness checks and post-apply recheck remain required. Preview and
Check do not write source files; already completed writes are not rolled back on
cancellation. No framework build, dependency restore, database execution or
external-source traversal is part of this workflow.

## Reproducible unpublished packs

Use already downloaded, verified source WASMs and committed schema/license files:

```sh
python3 scripts/build_wasm_language_pack.py --toolchain yaml \
  --source-dir /path/to/verified-inputs --output /path/to/codestyles-yaml-pretty_yaml-0.6.0.tar.gz \
  --candidate /path/to/yaml-candidate.json
python3 scripts/build_wasm_language_pack.py --toolchain sql \
  --source-dir /path/to/verified-inputs --output /path/to/codestyles-sql-sql-0.4.1.tar.gz \
  --candidate /path/to/sql-candidate.json
```

The input directory must contain `pretty_yaml-0.6.0.wasm` and/or `sql-0.4.1.wasm`.
The builder is offline, checks fixed size/SHA-256 pins, includes schemas, MIT
notices and provenance, and emits `available: false` candidate manifests. It does
not execute inputs or publish artifacts. Optional `--resources` selects the
metadata directory; those bytes must still match the same fixed pins.

## Acceptance and remaining gates

Upstream MIT notices are included, but transitive dependency/target-feature
license review remains a release gate. Linux formatter probes are not evidence
for all six native host targets. Six-target managed lifecycle/package acceptance,
minimum-OS checks and interactive GUI acceptance must be recorded separately.
An existing green dprint CI run does not validate this newer language slice.
Runtime publication, download enablement and native file-open associations are
not delivered by this local implementation. Current test results should be
recorded against the final combined implementation, not inferred from this scope
document.

### Local verification — 2026-11-02

Executed on Linux x86-64 with the exact pinned dprint/WASM bytes and cached native
fixtures; no artifact was published and no production download entry enabled:

- `cargo test --locked --offline --no-default-features -- --test-threads=2`:
  **242 passed**, 13 explicitly ignored tests (180 library tests included).
- The ignored real Web/YAML/SQL acceptance test was then explicitly run and
  passed: all five samples, settings effects, JS/TS/JSON/CSS/SCSS composition,
  `.htm`/`.yml` aliases, three SQL dialects, YAML semantic guards, immutable
  JSONC, selected-language Check/review/apply, member corruption/refusal/repair,
  bad archives, cancellation and uninstall.
- Explicit existing managed dprint, Vue, Go and Dart acceptance tests passed.
  Rust, Java, C# and C/C++ native-runtime acceptance was not rerun; their
  non-runtime unit/config tests remain in the regular suite.
- All-target desktop `cargo check` and `cargo clippy -- -D warnings`, formatting
  and `git diff --check` passed. These are compilation/static checks, not an
  interactive GUI test or execution of desktop-bin UI tests.
- Python: **189 tests run, 188 passed, 1 platform skip**, including the opt-in
  pinned-host/WASM YAML semantic oracle. Eight accepted typed-value cases cover
  aliases/comments, multiple documents, implicit/quoted types, block scalars
  and significant trailing spaces; separate regression witnesses establish the
  two upstream behaviors excluded/overridden by the adapter. This is not YAML
  conformance certification.
- Both new WASM pack archives reproduce byte-for-byte, matching all six disabled
  target manifests. Existing dprint and Vue catalog entries remain unchanged.

Exact unpublished candidates:

| Pack | Bytes | SHA-256 |
| --- | ---: | --- |
| YAML | 185054 | `0ee9a66019fde997bbfbc3619f6a080e10332ef74cba90c414ca3e83c0bcf7a5` |
| SQL | 218578 | `20f5af44e490af0b83ab63fcb46a692cb13da5cc24e8eea7143d8fbdf6a9e810` |

No Windows crash fix, non-Linux native acceptance, oldest-OS launch, actual GUI
layout/keyboard/clipboard behavior, release signing or transitive license
acceptance is implied by these local results. Local tests do not enforce a
network namespace; the managed formatter paths use only explicitly provisioned
pinned local runtime bytes.
