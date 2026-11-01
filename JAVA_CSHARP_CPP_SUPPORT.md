# Java, C# and C/C++ managed formatter candidates

These languages use the existing visual configuration, JSONC settings, immutable
preview, project review-before-write, cancellation, and optional managed-runtime
boundary. **This change does not publish any formatter or enable a download.**
Go continues to use the existing canonical `gofmt` integration (no invented style
knobs). Its previously recorded candidate/acceptance status is unchanged.

## Pinned formatter contracts

| Language | Engine | Settings section | Native pack ID |
| --- | --- | --- | --- |
| Java | google-java-format 1.37.0 | `googleJavaFormat` | `google-java-format` |
| C# | CSharpier 1.3.0 | `csharpier` | `csharpier` |
| C and C++ | clang-format 23.1.4 | `clangFormat` | `clang-format` |

Java/Clang packs provide `bin/<pack-id>` (`.exe` on Windows). CSharpier
provides the exact managed .NET 10.0.14 runtime under `dotnet/` and net10 tool
payload under `lib/csharpier/`. Its fixed entry point is `dotnet/dotnet`
(`dotnet/dotnet.exe` on Windows), not an SDK/tool-install shim. All runtime
libraries and notices have every member’s digest and byte count pinned.
No JDK, .NET SDK, compiler, package manager, user-provided executable, PATH lookup,
installation script, or shell is invoked. A managed missing/unavailable runtime
is an error, not a successful unchanged preview. Working drafts remain CodeStyles JSONC. Explicit bounded native import/copy
interoperability is described below; it never automatically rewrites
`.clang-format`, `.csharpierrc`, IDE settings, `.editorconfig`, a Java build file,
or a .NET project/solution.

Java exposes Google/AOSP style, while import sorting/removal is explicitly
suppressed: a formatter preview must not silently delete imports. The pin is
intentional, not a claim to follow latest (2.0.0 was already visible upstream on
2026-11-01). C/C++ uses an inline allowlisted style and synthetic filename, not
`--style=file` or discovery of parent configuration; include sorting is disabled.
C# receives a private explicit configuration and an absolute synthetic `.cs`
stdin path. An explicit empty private ignore file prevents parent ignore
discovery; cache and MSBuild-version checks are disabled while syntax-tree
validation remains enabled.

C# is **standalone syntax formatting**, not `dotnet format`, analyzer execution,
semantic refactoring, solution loading, restore, or build. Project symbols,
references, source generators and language settings are not resolved. XML,
`.csproj`, `.sln`/`.slnx`, Razor and arbitrary project files are not implicitly
C# sources. Do not infer full project/compiler support from the source preview.

## Upstream and distribution evidence (2026-11-01)

`resources/extended-formatter-sources.json` records version-specific upstream
release API metadata. Those URLs are **build-research metadata only**, never
application download endpoints. Metadata hashes alone do not prove native
compatibility, complete redistribution rights, or managed install acceptance.

* [google-java-format 1.37.0](https://github.com/google/google-java-format/releases/tag/v1.37.0)
  publishes GraalVM native executables for Linux x64/ARM64, macOS ARM64, and
  Windows x64. No macOS x64 or Windows ARM64 asset appeared in that release.
  Using a native executable avoids a user's JDK requirement; it does not remove
  the obligation to audit GraalVM, compiler and transitive notices. The upstream
  [LICENSE](https://github.com/google/google-java-format/blob/v1.37.0/LICENSE)
  includes Apache-2.0 and historical LLVM notices; native-image dependency
  redistribution audit remains pending.
* [CSharpier 1.3.0](https://github.com/belav/csharpier/releases/tag/1.3.0) publishes
  a NuGet tool package, **not a verified self-contained native CodeStyles pack**.
  The local candidate instead pairs its exact net10 tool DLL payload with the
  official .NET 10.0.14 runtime (not SDK). Fixed `exec --fx-version 10.0.14
  --roll-forward Disable` arguments select the absolute managed DLL; no host
  discovery or runtime roll-forward is allowed. CSharpier's
  [MIT license](https://github.com/belav/csharpier/blob/1.3.0/LICENSE) alone is not
  an audit of the runtime and all transitive dependencies. No `dotnet tool`
  installation or implicit user's SDK fallback is provided.
* [LLVM 23.1.4](https://github.com/llvm/llvm-project/releases/tag/llvmorg-23.1.4)
  publishes large platform distributions, not a reviewed minimal CodeStyles
  formatter pack. Selective extraction needs exact member pins, shared-library
  closure and OS baseline validation. LLVM uses
  [Apache-2.0 with LLVM exceptions](https://llvm.org/docs/DeveloperPolicy.html#license),
  with additional third-party obligations to review for the selected payload.
  Full LLVM SDKs are not bundled into the application or downloaded by it.

All 18 new formatter/target entries in `resources/toolchains.json` remain
`available: false`, with empty archives and actionable reasons. Catalog activation
requires separately reviewed license evidence, native platform/architecture and
managed end-to-end acceptance, digest-verified approved publication, and an
explicit catalog update. Cross-compilation or fixture tests do not satisfy that
native acceptance gate.

## Offline candidate packaging

`scripts/build_extended_formatter_pack.py` reads **only fixed plans** in
`resources/extended-formatter-sources.json`, verifies every local regular input's
size/SHA-256, stages it under a bounded allowlist and produces a deterministic
`.tar.gz` plus an unpublished candidate receipt. Symlink inputs, unsafe paths,
case-folded/Unicode-normalized path collisions, file/directory prefix conflicts,
reserved Windows names, SDK-shim entry points, missing notices, unsupported
targets, altered member pins and replacement of existing outputs are rejected. It never downloads or executes
anything, never modifies the runtime catalog, and cannot mark its output
available. A source receipt is provenance metadata; member hashes establish local
byte identity, not a completed upstream supply-chain/redistribution audit.

Linux x64 Java, CSharpier and clang-format have member-level candidate plans. For the Java
example below, obtain the exact listed native binary and LICENSE into a local
input directory and materialize `AUDIT_STATUS.txt` using the exact text in that
plan’s `generated_inputs` map. CSharpier instead requires the complete pinned
`dotnet/` runtime tree, `lib/csharpier/` net10 payload and `licenses/` inputs;
never execute a NuGet archive directly. Then:

```sh
python scripts/build_extended_formatter_pack.py \
  --tool google-java-format --target x86_64-unknown-linux-gnu \
  --inputs /path/to/verified-inputs \
  --output /path/to/new-candidate.tar.gz \
  --candidate /path/to/new-candidate.json \
  --work-dir /path/to/large-enough-private-work
```

A missing other-platform member plan fails closed. Do not substitute a
NuGet package for a native executable or guess missing binary checksums. The
builder's test suite exercises three tools × six target layouts with non-executed
fixtures; those tests are packaging-contract proof, not native platform proof.

Official behavior references: [CSharpier CLI](https://csharpier.com/docs/CLI),
[CSharpier options](https://csharpier.com/docs/Configuration),
[clang-format command line](https://clang.llvm.org/docs/ClangFormat.html), and
[google-java-format usage](https://github.com/google/google-java-format/tree/v1.37.0).

## Local validation recorded for this change

The Linux x64 Java input (33,294,600 bytes, SHA-256
`881821ffd1a52b175886c4ebeeacce2ba5b5d7326b52d32a18d384d8a783bd77`)
was downloaded and matched the upstream release API digest. It identifies itself
as 1.37.0 with `--version`. Initial short probes hit 15/60-second timeouts while
process inspection showed disk sleep. A subsequent probe with one shared
300-second deadline passed all four cases: Google formatting (54.54 seconds on
the cold first run), idempotence (0.05 seconds), distinct AOSP indentation (0.01
seconds), and invalid syntax rejected with exit 1 (0.01 seconds). Both unused
imports remained present, in their original order. The working/home/temp
locations were private, with empty PATH; no external JDK was used.

This is Linux x64 **direct executable smoke only**, not managed installation or
six-platform acceptance. ELF inspection found `libz.so.1` and `libc.so.6`
requirements, so “native” does not mean fully static. The deterministic local
candidate is 11,308,170 bytes (33,310,924 installed payload bytes), SHA-256
`a11959bb9ca7588d9d6b766a2fd8ced0b442099bb41ca82bffa71014dce1363e`.
It remains unavailable and not approved for publication because native-image
license auditing and remaining platform acceptance remain outstanding.
The later Linux managed acceptance result is recorded below.

Offline Python packaging/distribution suite: 171 tests run, 170 passed and one skipped,
including the new candidate tests. Those fixture tests do not execute native formatters or provide desktop visual
or six-platform runtime QA. Direct native process evidence is recorded separately below.

### Managed CSharpier Linux x64 proof

The official CSharpier 1.3.0 NuGet tool archive is 23,347,493 bytes, SHA-256
`87089e1284137004f2299219ec241258034f06ffa9112860e9c0d75391087b15`.
Its net10 payload (excluding PDBs, retaining satellite assemblies) is 19,173,108
bytes. The exact [.NET runtime 10.0.14](https://dotnet.microsoft.com/en-us/download/dotnet/10.0)
Linux x64 archive is 36,673,986 bytes. Its SHA-512 matched the official
[release metadata](https://builds.dotnet.microsoft.com/dotnet/release-metadata/10.0/releases.json);
the locally verified SHA-256 is
`bd3a25b2c5b617934de7ca55fc410711afd772c0d95a1af9205db6c3cc2c59f1`.
The full runtime plus selected tool payload and notices is 101,936,963 bytes.
No SDK was downloaded, extracted, or executed. Runtime `LICENSE.txt` and
`ThirdPartyNotices.txt` are retained; the tool’s 27 dependency entries still need
a complete transitive notice audit before redistribution approval.

Using an empty PATH and private home/temp/config/ignore files, the fixed managed
host command passed formatting, idempotence, tab indentation and invalid-syntax
rejection (0.32–0.39 seconds per invocation). An initial relative
`--stdin-path sample.cs` exposed an upstream `ArgumentNullException`; the
validated contract uses an **absolute synthetic private** `sample.cs` path,
never the user’s project path. These are direct-process tests, not managed
installation or desktop QA. Microsoft documents this runtime-only
[application-execution contract](https://learn.microsoft.com/en-us/dotnet/core/tools/dotnet).

Additional C# isolation checks passed with hostile parent `.csharpierignore`,
`.editorconfig` and `.csharpierrc` files. No `runtimeconfig.dev.json`, additional
probing paths or startup-hook configuration exists in the pinned tool payload.
Generated-source ignore directives retain upstream meaning.


### clang-format Linux x64 proof and ABI blocker

The official LLVM 23.1.4 Linux x64 `.tar.xz` was processed as a bounded stream,
without storing its 2,017,061,664 compressed bytes or extracting the full SDK.
The complete compressed SHA-256 matched the release metadata before execution:
`25d814fda50d5fb0084b42be09605d8b1595d49d5e2e03a3345d7e112ff6f9f3`.
The alternative `.zst` declares a 1 GiB decode window and was rejected before
extraction by the local 256 MiB memory bound. The `.xz` decoder stayed bounded;
only the formatter, candidate notices and an inspected but unused library were
selected. That unused library was removed from the task's own staging output.

The retained `clang-format` is 5,586,416 bytes, SHA-256
`5e49296e3cf369160e8361e20c95f7717e4c812e681206dfb634a7be8f0fbbf2`.
It reports 23.1.4 at LLVM commit `7a941dfb2877a0b8cf30323fefa85ad019e0b0f5`.
C and C++ source formatting/idempotence, distinct Allman indentation, all seven
base styles, five tab choices and eight brace choices passed direct-process
checks with private working/home/temp directories and empty PATH. Include order
was retained with `--sort-includes=false`. No compiler or project discovery ran.
The known incomplete input `int main( {` returns exit 1 with
`--fail-on-incomplete-format`; an unterminated comment and many other syntactically
invalid sources return success. **clang-format is not a C/C++ syntax validator.**

ELF symbol analysis requires **GLIBC 2.34 and GLIBCXX 3.4.29**, exceeding this
application's Linux glibc 2.28 baseline. System requirements also include
`libm`, `libz`, `libstdc++`, `libgcc_s`, `libc` and the ELF loader; it does not
require the extracted `libclang-cpp`. This binary cannot be approved as the
existing baseline-compatible Linux pack without a different reviewed build.
LLVM/Clang license texts are pinned at the exact source commit, but complete
transitive redistribution auditing remains pending.

The deterministic generic candidate is 2,017,217 compressed bytes, SHA-256
`158347f530e7ac4d1982fb11474943d19353747d657c245fe07d1ca87122311f`.
A separate provenance-bound pipeline bundle supports the combined exact ignored
C/C++ managed acceptance test. Direct CLI proof and a successfully built candidate
are **not** managed acceptance, oldest-OS proof, release clearance or publication
approval. All six clang-format catalog targets remain unavailable and empty.

### Header scope

C project mode includes `.c` and `.h`. C++ project mode includes common C++
source/header suffixes **and `.h`**, using the selected C++ context for the private
stdin hint. Fingerprints, review diffs and writes retain the real `.h` filename.
A direct `.h` preview without an explicit project scope defaults to C. Neither
mode uses a compilation database or reads adjacent formatter configuration.

## Initial local application validation

Production implementation `3f0e6401cd0c5d06e405de696c11b2ab39741cf2` passed
215 core/library/integration tests (11 opt-in/performance tests ignored by the
ordinary suite), core all-target clippy, desktop all-target check/clippy,
formatting and whitespace checks. The packaging Python suite ran 171 tests:
170 passed and one skipped. Go’s existing pinned pack passed its explicit managed
install/preview/project-review/write/uninstall test separately.

At `9c9addb755cbf128f27eb34fe76043e71f0f4a38`, isolated clean-commit managed
acceptance passed for Java, CSharpier, and combined C/C++ (including `.h` in an
explicit C++ scope). These checks installed the exact provenance-bound candidate,
verified preview/idempotence/error behavior, checked without writes, applied only
reviewed selections, retained other sources/config bytes, uninstalled, and
verified missing-runtime failure. Every report confirms the canonical checkout
and catalog were unchanged, the isolated catalog was restored, and temporary
clones were removed. An initial `.h` acceptance mismatch was corrected only in
the test: direct `.h` preview defaults to C, while the selected C++ project uses a
C++ stdin hint. No production formatting change was needed for that correction.

Additional static symbol inspection found the Java native executable also
requires **GLIBC 2.34**, so it shares clang’s glibc 2.28 release blocker. All 16 ELF
members of the managed .NET payload require at most GLIBC 2.27 / GLIBCXX 3.4.22;
that does **not** establish complete dependency closure or oldest-OS execution.
Transitive notices and all platform/runtime release gates remain required.

No GUI visual QA, full desktop link/package build, non-Linux native execution,
public release, download activation, or redistribution approval is claimed.
All 18 new production catalog targets remain unavailable with empty archives.
The initial documentation-only update at `9df3bee` did not change those tested
application sources, resources, scripts, or dependencies.

## Explicit native configuration interoperability

Editor & CI offers **Import** and **Copy** for these bounded subsets:

* `.csharpierrc.json`: strict JSON object containing only `printWidth`, `useTabs`,
  `indentSize`, and `endOfLine`. YAML, `.editorconfig`, overrides, unknown fields,
  duplicates and incorrect types reject the whole import.
* `.clang-format`: one flat scalar YAML mapping (or strict JSON) using the six
  displayed options. A single document start/end marker and comments are accepted;
  nesting, anchors, aliases, tags, multiple documents, `Language` overrides,
  ambiguous Unicode structural whitespace, duplicate/unknown fields and parent
  inheritance are rejected. `SortIncludes: Never` (or legacy `false`) is accepted
  as the fixed include-order policy; any enabled sorting is rejected. Exports
  always emit `SortIncludes: Never`.

Selected files must be regular, non-symlink UTF-8 files no larger than 1 MiB.
Imports merge only explicitly supplied supported keys into an unsaved cloned
JSONC draft, then validate before committing the whole edit. Existing comments,
unrelated settings and absent keys are kept; imported file comments are not
copied. No file is saved or runtime downloaded automatically. Pending edits,
busy work, repeat dialogs, cancellation, document revisions and exact-language
switches—including C to C++—prevent stale import/copy actions.

Exports reflect the applied draft and native defaults. They are clipboard text,
not an automatic project/IDE write. This is deliberately **not a general YAML or
arbitrary formatter-configuration converter**; unsupported configurations receive
an error instead of partial semantic approximation. Existing Rustfmt TOML import
uses the same bounded safe reader and retains its previous workflow.

The extended candidate builder also uses unique private archive/receipt staging
and atomic no-replace hard links. A late competing output or `.part` file is not
overwritten or removed. Filesystems without hard-link support fail closed; if the
archive is published but a competing receipt wins, the valid archive is retained
and the partial outcome is explicitly reported.

## Interoperability and final security review

Implementation `b03547d` adds the explicit native config workflow above and fixes
an independently identified candidate-builder output collision race. The final
ordinary suite passed **220 Rust core/library/integration tests** (11 ignored)
and **174 Python tests** (one skipped). Core and desktop all-target Clippy with
warnings denied, Rust formatting, and whitespace checks passed. Desktop tests
were compiled, not linked/executed; this is not visual/UI acceptance. Independent
final source review found no remaining actionable issue within this scope.

Real Linux engine checks accepted every supported exported setting for C, C++
and CSharpier, produced identical output to CodeStyles' private/inline settings,
and were idempotent. Managed acceptance results are recorded below separately;
all 18 new native-catalog targets remain unavailable.

### Remaining baseline-compatible runtime routes

The tested Java native image and clang binary both require GLIBC 2.34, above the
project's GLIBC 2.28 target. No baseline was relaxed.

* Java: the same-release official
  [google-java-format 1.37.0 all-deps JAR](https://github.com/google/google-java-format/releases/tag/v1.37.0)
  is 3,691,503 bytes, SHA-256
  `834b2a0c38cb774953322a84b5ca3f2f40dd3156650b3cd44d3b744345962f7a`.
  A separately pinned Temurin 25 `jlink` image including `jdk.compiler` is a
  plausible managed route, not an implemented or accepted replacement. The
  [Adoptium build FAQ](https://github.com/adoptium/temurin-build/blob/master/FAQ.md)
  describes older Linux baselines; actual artifact/module/ELF closure still
  requires proof. Local tools lack a full JDK/jlink, and the exact Adoptium
  metadata request returned HTTP 403; no retry or alternate-access bypass was
  attempted. The existing native executable adapter is unchanged.
* Clang: build only `clang-format` from the exact LLVM 23.1.4 source in the
  already digest-pinned manylinux 2.28 environment, using the official
  [clang-format target](https://github.com/llvm/llvm-project/blob/llvmorg-23.1.4/clang/tools/clang-format/CMakeLists.txt).
  Installed compilers satisfy upstream host requirements, but the workspace has
  no Docker/Podman or old-ABI sysroot. A local host-linked build would not prove
  compatibility. No large source/toolchain download or speculative build was
  performed. Static C++ runtime linkage alone does not solve the GLIBC baseline.

### Notice evidence, not release clearance

A separate local notice audit maps all 27 CSharpier declared libraries (25 NuGet
packages plus two project assemblies) to collected texts. Its 17 unique notice
bodies include the .NET runtime notices, exact Roslyn commit notices, and exact
versioned package/dependency notices. Twenty-three small NuGet packages match the
recorded unsigned content hashes, and 37 selected DLL/resource files match the
candidate. This is not certificate/signature validation. Two legacy packages
omit source commits; historical linked-repository license evidence and that
limitation are recorded. These texts are **not yet in the tested candidate**.

The Java audit records the verified same-release JAR's dependency notices,
including CommonMark BSD attribution, and GraalVM texts. Exact embedded
JDK/native-image module/source mapping and applicable distribution obligations
remain unresolved. Collecting aggregate license files is not legal clearance.

Before distribution: review notice coverage and provenance caveats, integrate
approved texts into fixed member pins, rebuild and reaccept changed candidates,
prove platform/ABI/dependency closure, and obtain publication authorization.
No user files, system formatter installs, repository remotes, public artifacts,
or production download availability were changed by this follow-through.

At `b03547dd01ab27dbd8eb6bd0cc7ef9a962279dd7`, renewed isolated managed
acceptance passed for Java, CSharpier and combined C/C++ (including C++ `.h`).
CSharpier/C/C++ acceptance now imports native settings, exports/reimports them,
and exercises the resulting settings through the real managed engine and
reviewed project writes. All three reports confirm unchanged canonical checkout
and catalog, restored isolated catalog, and removed temporary clones. The
following documentation-only commit changes no tested executable source.
