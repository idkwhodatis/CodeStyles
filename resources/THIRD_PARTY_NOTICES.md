# Third-party resources

- dprint 0.63.0: https://github.com/dprint/dprint/releases/tag/0.63.0
- dprint TypeScript plugin 0.96.1: https://github.com/dprint/dprint-plugin-typescript/releases/tag/0.96.1
- dprint JSON plugin 0.25.2: https://github.com/dprint/dprint-plugin-json/releases/tag/0.25.2

- dprint Markdown plugin 0.26.0: https://github.com/dprint/dprint-plugin-markdown/releases/tag/0.26.0
- dprint TOML plugin 0.9.0: https://github.com/dprint/dprint-plugin-toml/releases/tag/0.9.0
- dprint Ruff plugin 0.11.3 (including Ruff): https://github.com/dprint/dprint-plugin-ruff/releases/tag/0.11.3

These top-level projects declare the MIT License; their compiled dependencies
can have additional licenses. In particular, the dprint native runtime includes
Wasmtime 49.0.2/Cranelift 0.136.2. Their full Apache-2.0 WITH LLVM-exception
text is preserved in `licenses/wasmtime-49.0.2-cranelift-0.136.2.txt` and now
accompanies each rebuilt dprint pack. Exact source pins and scope are in
`licenses/dprint-0.63.0-dependencies-SOURCE.txt`. A complete target-specific
runtime notice bundle remains a redistribution gate; these named notices do
not cover every transitive dependency. Exact upstream
license texts from the pinned release tags are in `licenses/`. Their original
copyright and permission notices are retained. SHA-256 values in the manifest
for downloaded archives, WASM and schemas match the official GitHub release
asset digests; executable digests were computed after verified extraction.
Markdown, TOML and Ruff license digests are computed from the exact tagged upstream
LICENSE files, whose source URLs are also retained in the manifest.

## Optional standalone language pack resources

- Pretty YAML 0.6.0 (MIT): https://github.com/g-plane/pretty_yaml/tree/v0.6.0;
  exact notice: `licenses/pretty_yaml-0.6.0.txt`.
- dprint SQL 0.4.1 (MIT): https://github.com/dprint/dprint-plugin-sql/tree/0.4.1;
  exact notice: `licenses/sql-0.4.1.txt`.

Their schemas are embedded for offline settings. Runtime WASMs are separate,
unpublished optional pack candidates, not shipped app runtimes. Fixed source
URLs, sizes and SHA-256 pins are in `scripts/build_wasm_language_pack.py` and pack
provenance. Upstream notices are included; a complete transitive dependency and
target-feature license audit remains required before runtime redistribution.

## Optional Shell, Lua, Dockerfile and Caddyfile candidates

All candidates remain disabled and unpublished pending transitive license and
native validation. The first three use unchanged upstream plugin releases.

- dprint-plugin-shfmt 0.1.2: MIT; includes mvdan/sh 3.14.1 (BSD-3-Clause)
  and buger/jsonparser 1.6.1 (MIT). Combined notices: licenses/shfmt-0.1.2.txt.
- dprint-plugin-stylua 0.1.0: MIT; StyLua 2.6.0: MPL-2.0. Exact upstream
  license and corresponding source links: licenses/stylua-upstream-2.6.0-MPL-2.0.txt
  and licenses/stylua-0.1.0-SOURCE.txt. The wrapper disables upstream output
  verification; parsing/idempotence do not prove semantic equivalence.
- dprint-plugin-dockerfile 0.7.0: MIT, licenses/dockerfile-0.7.0.txt.
- Caddy formatter 2.11.7: Apache-2.0, licenses/caddy-2.11.7.txt. Exact
  formatter source is compiled into a CodeStyles MIT WASM adapter, not the
  upstream server binary. It requires the managed dprint engine. Source and
  extraction details: licenses/caddyfmt-2.11.7-p1-SOURCE.txt and formatters/caddyfmt.
  Runtime notices: caddyfmt-Go-BSD-3-Clause.txt, caddyfmt-TinyGo-BSD-3-Clause.txt,
  caddyfmt-LLVM-compiler-rt.txt, caddyfmt-musl-COPYRIGHT.txt,
  caddyfmt-wasi-libc-LICENSE.txt and caddyfmt-wasi-libc-LICENSE-MIT.txt under
  licenses/. Exact linked libc source pins are in caddy-formatter-sources.json.
  Final linked-feature review remains pending. No server startup, config adaptation or imports are provided.

Source pins and review: script-formatter-sources.json,
script-formatter-source-review.json, caddy-formatter-sources.json and
scripts/build_wasm_language_pack.py. These notices do not claim a completed
transitive license audit.

## Main application release gate

Formatter notices do not cover the CodeStyles application itself. The app now
preserves GPUI Kit 0.7.3's Apache-2.0 text and the exact `gpui-kit-assets` Lucide
ISC/Feather MIT composite notice, including original copyright and the list of
Feather-derived icons. See `licenses/gpui-kit-assets-0.7.3-SOURCE.txt`,
`licenses/gpui-kit-0.7.3-APACHE-2.0.txt` and
`licenses/gpui-kit-assets-0.7.3-LICENSE-LUCIDE.txt`. The icon notice is included
conservatively; retained SVG bytes in final app binaries have not been verified.

`option-ext` 0.2.0 declares MPL-2.0. Its unchanged source archive, full license
and extraction/source-availability instructions are retained under `licenses/`
as `option-ext-0.2.0.crate`, `option-ext-0.2.0-MPL-2.0.txt` and
`option-ext-0.2.0-SOURCE.txt`. Linux dprint packs include them conservatively.
The app keeps all checked-in legal resources on every target. The actual
`dirs-sys` use is in a non-Apple Unix XDG user-directory module; the reviewed
app/dprint call paths do not reach it. This is not a claim of MPL-covered code
in the final executable or an obligation to relicense unrelated app files.

The app still needs a complete target-specific Rust/native-library/font and
asset inventory before redistribution. Graph presence alone does not prove
final linkage or asset embedding. Neither these notices nor package-integrity
tests claim that all application obligations have been discharged. See the
repository's `DEPENDENCY_NOTICES.md` for evidence and remaining release gates.
