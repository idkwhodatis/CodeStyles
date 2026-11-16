# Script and container/config language support

This adds Shell, Lua, Luau, Dockerfile and Caddyfile as five editor/project modes.
Their samples and settings work without runtimes installed. Formatting requires
explicit installation of a complete verified optional runtime pack. No PATH tool,
host shell interpreter, Lua interpreter, Docker daemon, SDK or running Caddy server
is assumed. The application never executes source files to format them.

## Exact runtimes and publication state

| Mode | Pack | Upstream |
| --- | --- | --- |
| Shell | shfmt 0.1.2 WASM | mvdan/sh 3.14.1 |
| Lua and Luau | stylua 0.1.0 WASM | StyLua 2.6.0 |
| Dockerfile | dockerfile 0.7.0 WASM | dprint-plugin-dockerfile 0.7.0 |
| Caddyfile | caddyfmt 2.11.7-p1 WASM | exact Caddy 2.11.7 formatter source |

All four WASM packs require the unchanged managed dprint 0.63.0-p1 pack. All six target entries for each of the four new packs
remain `available: false`; no runtime has been published by this change.

The source and archive pins are recorded in `resources/script-formatter-sources.json`,
`resources/script-formatter-source-review.json`, `resources/caddy-formatter-sources.json`
and the embedded catalog. Offline pack builders reject hash/size mismatches and
retain upstream license/source notices. StyLua's MPL source notice identifies the
exact dependency revision. Its dprint adapter uses `OutputVerification::None`;
we do not claim AST-equivalence verification. Caddy uses a CodeStyles-owned
formatter-only adapter with unchanged upstream formatting source, not the Caddy
server binary. Exact source, compiler archive pins and build scripts accompany
the candidate. The native Caddy reference binaries remain recorded for comparison
only: their macOS 12 minimum is above the application's baseline.

## File routing and source scope

- Shell: `.sh` and `.bash`. Exact `sh`, `dash` or `bash` shebangs are supported,
  including `/usr/bin/env` followed by one shell name. Shebang options, zsh,
  fish, mksh and Bats are excluded. Extensionless files and shell rc files such
  as `.bashrc` are not inferred. Hidden files with supported extensions still
  receive the same source safety checks.
- Lua: `.lua`, with explicit syntax selected from Lua 5.1/5.2/5.3/5.4/LuaJIT.
  An omitted setting becomes Lua 5.4 only in the temporary runtime config.
- Luau: `.luau`, with Luau syntax. An omitted setting becomes Luau only in the
  temporary runtime config. Mixed/all/CfxLua modes and extension/syntax conflicts
  reject rather than guess.
- Dockerfile: `Dockerfile`, `Dockerfile.<nonempty suffix>`, and `*.dockerfile`.
- Caddyfile: `Caddyfile`, `Caddyfile.<nonempty suffix>`, and `*.caddyfile`.
- Exact Dockerfile/Caddyfile names and their dotted variants take precedence over
  final extension aliases. For example `Caddyfile.dockerfile` is Caddyfile and
  `Dockerfile.rs` is Dockerfile. Non-UTF-8 names that could match these modes are
  refused, never allowed to bypass source guards through another formatter.

Project operations use the explicitly selected language scope. Existing ignore,
build/vendor exclusions, symlink/trust/staleness and review-before-write checks
remain active. Native editor configuration files and `.dockerignore` are protected.
The Dockerfile stdin hint is normalized to `Dockerfile` for its pinned plugin's
matcher; review and filesystem writes retain the actual path.

## Validation before formatting

The backend validates supported settings and conservative source boundaries
before resolving a runtime or starting a formatter. Rejected input returns a clear
error and does not overwrite the original source or saved JSONC. Shell and Lua
syntax parsing happens inside their formatter; parser failures also return errors
without applying output. These checks are not general language validation or a
proof of semantic equivalence for arbitrary input.

- Shell simplification and minification are forced off in temporary settings;
  explicit unsafe values reject. Only auto/bash/posix dialects are accepted.
  Explicit dialect/shebang conflicts reject.
- Lua require sorting is forced off. Saved/exported JSONC is not silently
  rewritten to insert these defaults. Lua/ Luau code is not executed.
- Relevant duplicate keys, inherited settings and overrides reject in these
  managed modes. Unknown/unrelated user metadata is preserved in the document.
- Dockerfile templates, frontend syntax/custom escape directives, backticks/carets, `$()` and
  `${}` forms, custom SHELL, ONBUILD, legacy `ENV/LABEL key value`, unknown instructions,
  incomplete continuations and unsupported quoted forms reject. The pinned
  formatter has observable whitespace-changing cases for several of these forms.
  Continued ARG/COPY/ADD instructions and actual backslash-tab sequences also
  reject because the formatter can split or alter literal tokens. This includes
  continuations with trailing horizontal whitespace.
- Dockerfile supports a single adjacent ASCII-identifier heredoc on RUN or COPY,
  optionally single/double quoted. Tab-stripping `<<-`, spaced delimiters,
  punctuation/numeric delimiters, multiple heredocs and continued headers reject.
  The body and delimiter, including whitespace and opening quote state, must be
  byte-preserved in returned output. Ordinary quoted literal tokens are checked
  too, as is the sequence of logical instructions. If protected content changes,
  the result is rejected.
- Caddyfile has fixed upstream tab-based formatting, no arbitrary style/command
  options. An isolated environment/home/working directory and cleared caller
  environment are used for the verified dprint engine and Caddy WASM only. A
  private cache/config disables discovery and normalizes every Caddy alias to a
  `Caddyfile` stdin hint. Caddy-only `--skip-stable-format` preserves the original
  one-pass `caddy fmt -` behavior; other languages keep their existing semantics.
  Both runtime leases remain held through subprocess exit. Output and diagnostics
  are bounded, and cancellation/timeouts terminate the process tree. No `adapt`, `validate`,
  import loading, server start, certificate management or network operation is
  requested. Lexical token preservation is checked, including quoted/backtick
  values and comments. Unterminated quoted values and templates reject. Other
  malformed Caddy directives may still be formatted because `caddy fmt` does not
  perform full syntax or semantic validation.

These source guards apply in CodeStyles, not to an exported JSONC file used by an
external formatter. External CLI/editor use must separately enforce safe settings
and source boundaries.

## Verification and release gates

`tests/script_config.rs` contains runtime-independent preflight regressions and
explicit ignored real-runtime acceptance tests. The latter require trusted local
archives in `CODESTYLES_{DPRINT,SHFMT,STYLUA,DOCKERFILE,CADDYFMT}_ARCHIVE` and do not
download anything. They cover settings effects, idempotence, protected literals,
file aliases, review/apply, unchanged invalid files/configs, corrupt/reinstall/
uninstall handling and the explicit Caddy dependency on dprint. A Unix-only test compares
six fixed, side-effect-free Bash fixtures before and after formatting; it never
executes user source. Parser and runtime tests are distinct from GUI acceptance.

Before release: complete native runs on every target and oldest supported OS/
glibc baseline, interactive desktop acceptance, transitive license review,
artifact publication/signing and explicit catalog promotion. Building the universal Caddy WASM on Linux and auditing dprint Mach-O minimum
versions does not certify native runtime behavior on any other platform.


## Caddy formatter-only source build

`formatters/caddyfmt` retains unmodified Caddy 2.11.7 `formatter.go` and upstream
formatter tests, plus the unchanged required heredoc regexp declaration. No
Caddy server or external Go module dependencies are compiled. The adapter uses
TinyGo 0.43.0 and supported Go 1.26.10; it has no host imports, including WASI,
filesystem, environment, network or nested formatter callbacks. Runtime notices
include Caddy, Go, TinyGo, LLVM compiler-rt and the MIT adapter. Final linked-feature
license review remains a release gate.

`compile_caddy_formatter_wasm.py` verifies exact local source/toolchain archive
hashes, runs Go tests and rejects rebuilt bytes that differ from the reviewed pin.
`build_caddy_formatter_pack.py` creates a deterministic disabled data-only pack
from those exact bytes, the complete source/build recipe, schema and notices.
Neither script downloads, publishes or enables a runtime. See the adapter README
for offline invocation.

The candidate passed all 34 upstream fixtures and 560 official-binary differential
cases, repeated-call/GC and 32 MiB boundary tests. dprint's default repeated
stabilization differed on 43 generated malformed cases, which is why Caddy uses
its own one-pass invocation. These are formatting tests, not general language or
semantic validation. Boundaries can reserve hundreds of MiB of guest memory;
input/output limits, cancellation and process isolation remain necessary.

The exact managed dprint binaries declare macOS 10.12.0 on Intel and 11.0.0 on
ARM64, within the application's desired 10.15.7/11.0 baselines. This is static
metadata evidence only; oldest-supported-OS execution remains unverified.

### Local Linux verification snapshot (2026-11-04 Toronto / 2026-11-05 UTC)

The uncommitted follow-up to `16da61b` passed the broad Linux Rust run (261
passed, 18 initially ignored), explicit trusted-runtime reruns of all seven
`script_config` tests and all three `managed_dprint` tests, and 33 GUI-state unit
tests. Six of the initially ignored runtime tests were covered by those explicit
reruns; the other twelve remain unexecuted in this snapshot. The full Python
suite, including compiler trust-boundary and oracle-replay tests, ran 204 tests:
202 passed and two skipped (native Windows ACL semantics and provisioned PyYAML
runtime semantics).

Tracked offline Caddy validation scripts and deterministic fixtures are documented
in `formatters/caddyfmt/VALIDATION.md`. They passed 560 official-Caddy/dprint/WASM
comparisons, 10,000 repeated WASM calls, ABI/error/boundary recovery, and bounded
nested-brace probes through depth 2000. Oracle regeneration reproduced the stored
JSON byte-for-byte. These probes honor upstream's indentation cap of ten; they
are not an unbounded memory stress test.

The local Linux portable/DEB candidates passed static glibc-2.28 policy checks and
read-only startup/resource/offline-settings smoke checks. Smoke networking used
unreachable proxies, not enforced network isolation; optional runtimes were
absent, so its ten formatter checks were skipped. Separate trusted-runtime tests
cover actual formatting.

Focused interactive checks completed on the current Trixie desktop with local
Mesa and an isolated private portal, without global changes, using executable
SHA-256 `819fa87db6f2b6f68c9bfc1cd07b6238fa81daec8d79280ce83d08b528140653`.
Actual GTK Save As cancellation worked; saving to a spaced filename and reopening
retained TypeScript indent width 4 while trust stayed paused. The four new
Language support tabs switched with visible status. Local UI installs of Caddy,
dprint, shfmt, StyLua, and Dockerfile were verified. Fresh Untitled runtime
previews succeeded for Shell, Lua, Luau, Dockerfile, and Caddyfile; explicit Shell
indent width 4 persisted at revision 7. Light/dark themes rendered. Unsaved Close
Cancel worked, then discarding disposable changes exited cleanly with status 0.

The preceding binary had clipped GPUI prompt details. A scoped Linux-only
renderer now constrains and wraps the original disclosure text while preserving
GPUI response/dismissal/focus behavior and caller action order. The final binary
SHA-256 is `961c2428eddd73da745bf0b9de0839d83cacf716c9171a21b2467ef6ad8373ea`.
Actual GTK Open, complete trust/discard warnings in both themes, repeated trust
cancellation, dirty-close Cancel, and discard/clean exit passed at the normal
window size and enforced minimum 1100x700. No trust was granted by cancellation;
the saved QA fixture remained unchanged. Final formatting, 261 headless tests
(with 18 ignored), 34 GUI-state tests and all-target clippy passed. The earlier
explicit native-runtime reruns cover unchanged library source. Screenshots were
inspected in session, not saved. Escape remains unsupported as in the prior GPUI
fallback; keyboard accessibility, below-minimum sizing and clipboard/IME behavior
are not accepted by this scoped fix. These are not full GUI sign-off.
Neither these runs nor static macOS metadata establish native oldest-OS or
all-platform behavior. Broader GUI/native acceptance, final license review,
signing/publication, and explicit release/catalog approval remain outstanding.
Packs remain unpublished/disabled.
