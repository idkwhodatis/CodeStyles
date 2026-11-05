# Caddy formatter-only WASM candidate

This unpublished CodeStyles adapter compiles the exact Caddy 2.11.7 formatter to
WASM for dprint 0.63.0. It is not an official Caddy distribution and contains no
Caddy server. Application input guards and lexical-preservation checks remain
required. It accepts UTF-8 whole files up to 32 MiB, has fixed upstream style,
and rejects arbitrary style options and range formatting.

`internal/caddyfile/formatter.go` and `formatter_test.go` are unchanged from
Caddy commit `72dd0fb067f6d7826c7f79907670ba4a713bfe37`. `heredoc.go` retains the
one required regexp declaration from that revision's `lexer.go` with its notice.
The root adapter is CodeStyles MIT code implementing the public dprint schema-v4
ABI. Runtime and upstream notices are in `licenses/`.

The application must invoke this plugin with `--skip-stable-format`: ordinary
dprint stabilization can differ from one pass of `caddy fmt -` on malformed
input. Do not change stabilization for other plugins.

Build offline with the exact official archives pinned in
`resources/caddy-formatter-sources.json`:

```
python3 scripts/compile_caddy_formatter_wasm.py \
  --archive-dir /path/to/verified/downloads \
  --output /path/to/caddyfmt-2.11.7-p1.wasm
python3 scripts/build_caddy_formatter_pack.py \
  --wasm /path/to/caddyfmt-2.11.7-p1.wasm \
  --output /path/to/codestyles-caddy-2.11.7-wasm-p1-universal.tar.gz \
  --candidate /path/to/candidate.json
```

The compiler runner verifies source and official archive hashes, extracts a
private toolchain, runs Go tests, builds without module downloads, and rejects
bytes differing from the reviewed WASM pin. The pack builder does not execute
inputs. Neither script publishes or enables a runtime. All six target entries
remain disabled pending full application/native/oldest-OS/license release gates.

Updating Caddy or the toolchain requires a deliberate pin refresh, the original
upstream fixtures, differential tests against the official Caddy binary,
repeated-call/resource tests, license review and reproducibility checks. The
one-line heredoc dependency must be reconsidered if upstream formatting changes.
