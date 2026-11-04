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
| Caddyfile | caddyfmt native | Caddy 2.11.7 |

The WASM packs require the unchanged managed dprint 0.63.0-p1 pack. Caddyfile
does not require dprint. All six target entries for each of the four new packs
remain `available: false`; no runtime has been published by this change.

The source and archive pins are recorded in `resources/script-formatter-sources.json`,
`resources/script-formatter-source-review.json`, `resources/caddy-formatter-sources.json`
and the embedded catalog. Offline pack builders reject hash/size mismatches and
retain upstream license/source notices. StyLua's MPL source notice identifies the
exact dependency revision. Its dprint adapter uses `OutputVerification::None`;
we do not claim AST-equivalence verification. Caddy's full upstream binary is
redistributed in a candidate, but the app exposes only its formatter invocation.

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
  environment are used for `fmt -`. Output and diagnostics are bounded, and
  cancellation/timeouts terminate the process tree. No `adapt`, `validate`,
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
uninstall handling and Caddy independence from dprint. A Unix-only test compares
six fixed, side-effect-free Bash fixtures before and after formatting; it never
executes user source. Parser and runtime tests are distinct from GUI acceptance.

Before release: complete native runs on every target and oldest supported OS/
glibc baseline, interactive desktop acceptance, transitive license review,
artifact publication/signing and explicit catalog promotion. Building/reproducing
all six Caddy archives on Linux does not certify their native runtime behavior.
