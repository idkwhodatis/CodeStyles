# Third-party resources

- dprint 0.63.0: https://github.com/dprint/dprint/releases/tag/0.63.0
- dprint TypeScript plugin 0.96.1: https://github.com/dprint/dprint-plugin-typescript/releases/tag/0.96.1
- dprint JSON plugin 0.25.2: https://github.com/dprint/dprint-plugin-json/releases/tag/0.25.2

- dprint Markdown plugin 0.26.0: https://github.com/dprint/dprint-plugin-markdown/releases/tag/0.26.0
- dprint TOML plugin 0.9.0: https://github.com/dprint/dprint-plugin-toml/releases/tag/0.9.0
- dprint Ruff plugin 0.11.3 (including Ruff): https://github.com/dprint/dprint-plugin-ruff/releases/tag/0.11.3

These projects are distributed under the MIT License. Exact upstream license
texts from their pinned release tags are in `licenses/`. Their original
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
native validation. Upstream releases are used unchanged.

- dprint-plugin-shfmt 0.1.2: MIT; includes mvdan/sh 3.14.1 (BSD-3-Clause)
  and buger/jsonparser 1.6.1 (MIT). Combined notices: licenses/shfmt-0.1.2.txt.
- dprint-plugin-stylua 0.1.0: MIT; StyLua 2.6.0: MPL-2.0. Exact upstream
  license and corresponding source links: licenses/stylua-upstream-2.6.0-MPL-2.0.txt
  and licenses/stylua-0.1.0-SOURCE.txt. The wrapper disables upstream output
  verification; parsing/idempotence do not prove semantic equivalence.
- dprint-plugin-dockerfile 0.7.0: MIT, licenses/dockerfile-0.7.0.txt.
- Caddy 2.11.7: Apache-2.0, licenses/caddy-2.11.7.txt. Source:
  https://github.com/caddyserver/caddy/tree/v2.11.7. Complete upstream binary
  packaged; CodeStyles invokes only fmt -, never server startup or adaptation.

Source pins and review: script-formatter-sources.json,
script-formatter-source-review.json, caddy-formatter-sources.json and
scripts/build_wasm_language_pack.py. These notices do not claim a completed
transitive license audit.
