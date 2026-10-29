# Downloadable rustfmt toolchain packaging (unpublished)

## Distribution boundary

The application must consume a complete, reviewed **GitHub release bundle**.
It must not silently download Rust components from static.rust-lang.org, run
rustup/install.sh, search PATH for an existing Rust installation, or claim an
unpublished URL exists. `resources/toolchains.json` is the application manifest;
every target is currently unavailable for network installation. Linux x64 has a
real local candidate with exact archive/member hashes for offline-install tests.
No GitHub release or asset was created by this work.

`resources/toolchain-sources.json` is separate maintainer build input. Official
Rust components are permissible build sources, not final application endpoints.
`Rust 1.99.1` is the distribution pin; its rustfmt reports
`rustfmt 1.10.0-stable (58cf8f9c35 2026-10-13)`.

## Sources and trust

- Official channel: <https://static.rust-lang.org/dist/channel-rust-1.99.1.toml>
  (2026-10-15). SHA256
  `fb152be80975c194aa5fc684b2aec3a6e14e2b855efe2faa4e5f8ae6e590f463`
  matches the official `.sha256` response.
- GitHub Rust release: <https://github.com/rust-lang/rust/releases/tag/1.99.1>
  only exposes source assets; it is not the six-platform binary bundle source.
- Both `rustfmt-preview` and `rustc` exist in the official channel for Windows,
  macOS and GNU/Linux, each x64/ARM64. Their fixed source URLs and archive hashes
  are recorded, but availability alone does **not** establish the complete
  runtime closure or successful execution.
- Platform policies: <https://doc.rust-lang.org/rustc/platform-support.html>.
  The GNU/Linux host baseline is glibc 2.17; Windows host tools require modern
  Windows, macOS baselines vary by architecture. Application compatibility is
  still the stricter CodeStyles/native dependency policy, not only Rust's.

## Exact runtime closure

Linux x64 includes `bin/rustfmt`, the pinned `librustc_driver-*.so`, the actual
208 MB LLVM shared object, the upstream LLVM linker-name file, both components'
MIT/Apache license texts, Rust COPYRIGHT and complete component copyright HTML.
It does **not** include rustc/rustdoc, cargo/cargo-fmt, linkers, Rust standard
libraries, installers or compiler headers. Omitting rustc_driver/LLVM makes the
5 MB formatter fail to load; requiring a preinstalled Rust would defeat the
portable-package goal.

The source archives total **86,587,760 bytes** compressed. The selected runtime
and complete notices total **387,612,130 bytes**, plus a small provenance receipt.
The deterministic gzip candidate is approximately 126 MiB; each final asset's
exact bytes and SHA256 live in the application manifest. Compression formats
are intentionally distinct: upstream input is tar.xz; final single bundle is
tar.gz. Each allowlisted member has an exact destination, size and SHA256.

The formatter's Linux RUNPATH is `$ORIGIN/../lib`; its compiler driver resolves
LLVM from the same layout. A clean `env -i PATH=/usr/bin:/bin` invocation
formatted stdin successfully without rustup/RUSTUP_HOME/LD_LIBRARY_PATH.
ELF symbol-version inspection of all three actual ELF files found maximum
GLIBC requirement **2.16**, below the application's desired glibc 2.28 ceiling.
This is ABI inspection plus a current Linux x64 runtime test, **not** execution
on the minimum OS, six-platform verification, or GUI acceptance.

After authorization, the other five official archive pairs were downloaded and
SHA256/member-verified sequentially without executing their binaries. Static
loader inspection confirmed these additional closures:

- Linux ARM64: rustfmt, rustc_driver, LLVM; RUNPATH `$ORIGIN/../lib`, maximum
  GLIBC symbol requirement 2.17. Both Linux variants still rely on platform
  system libraries, including libgcc_s and zlib; “no installed Rust” does not
  mean “no operating-system shared-library dependencies”.
- macOS x64/ARM64: rustfmt, rustc_driver and LLVM, with `@rpath` imports and
  `@loader_path/../lib`. Mach-O commands indicate minimum macOS 10.12 for x64
  and 11.0 for ARM64. System frameworks, libSystem, libc++, zlib and iconv remain
  OS dependencies. These binary deployment fields are not a supported-app OS
  promise. Unreferenced proc-macro/sanitizer dylibs are excluded.
- Windows x64: rustfmt.exe and rustc_driver DLL, together in `bin/`; recorded
  PE imports name only that bundled driver and Windows system DLLs/API sets.
  No separate LLVM or VC redistributable DLL appears in these import tables.
  This does not establish successful execution on a clean Windows machine.
- Windows ARM64: official archive/member hashes and static PE32+ imports now
  verified. Host GNU objdump lacked ARM64 support, so the bounded stdlib parser
  `scripts/inspect_pe_imports.py` inspected both binaries without loading them.
  rustfmt.exe imports the bundled rustc_driver DLL; all other imports are Windows
  system DLLs/API sets. Neither binary has delay imports. No separate LLVM or
  VC runtime DLL is named in these tables. This is not native execution or proof
  against dynamically constructed LoadLibrary dependencies.

The source manifest records the actual ELF/PE/Mach-O static observations.
All five targets retain `native_execution: not-tested`. No additional final
bundles were built; all six application targets remain network-unavailable.
Native Windows/macOS/Linux ARM64 CI and minimum-OS testing remain required.

## Reproducible maintainer workflow

The maintainer generator requires Python 3.11+ for tomllib. Targets must be
explicit; there is no accidental all-platform download default. Refresh results
stay unavailable until a maintainer reviews the runtime dependency closure;
archive hashing alone does not promote a source target.

```sh
python scripts/generate_toolchain_manifest.py \
  --cache /path/to/staging --target x86_64-unknown-linux-gnu
python scripts/build_language_pack.py \
  --target x86_64-unknown-linux-gnu \
  --work-dir /path/with/adequate/free/disk \
  --output /path/codestyles-rustfmt-1.99.1-x86_64-unknown-linux-gnu.tar.gz \
  --candidate /path/candidate.json
```

The builder downloads each approved official component once, verifies size and
SHA256 before inspection, extracts only regular allowlisted members, verifies
member hashes, and never executes archive contents or installer scripts. Source
and final manifests remain separate. The final bundle contains PROVENANCE.json
with source archive identities and file pins. Tar ordering, uid/gid, mode,
timestamp, gzip header timestamp and filename are deterministic.

After a candidate is reviewed, obtain explicit publishing approval. Only then
upload the actual bundle to the selected repository/release, verify asset bytes
and digest, write the exact HTTPS GitHub download URL and enable that target.
A local candidate is not a successful publication or an available download.

## Space and validation

Use a dedicated workspace staging directory, not a nearly full shared `/tmp`.
For Linux x64 allow at least 700 MiB free for one compressed input, selected
runtime and final output; use at least 1 GiB for verification slack. Keep final
candidate, manifests, native test reports and provenance. Build remaining
platforms sequentially rather than retaining six toolchain archives/stages.
The compressed archive is capped by exact reviewed size, decompressed members
by a reviewed expansion bound, and installed output by a target-specific sum.

Unit coverage verifies deterministic metadata, path rejection, exact selected
members, expansion limits and exclusion of compiler/cargo/installers. Successful
archive construction is not native cross-platform execution. Native CI should
install each final candidate, clear Rust environment/PATH assumptions, test
stdin formatting/idempotence and syntax errors, and audit dynamic libraries.

### Additional targets: verified source sizes (no final bundles)

The authorized sequential inspection verified these compressed source bytes
and exact selected installed files, including complete notices. Installed sizes
below exclude the future small provenance receipt. No foreign binary was run.

| Target | Verified compressed bytes | Selected installed bytes |
|---|---:|---:|
| `aarch64-unknown-linux-gnu` | 71,004,604 | 288,553,386 |
| `x86_64-apple-darwin` | 62,986,420 | 257,819,896 |
| `aarch64-apple-darwin` | 52,236,076 | 247,421,312 |
| `x86_64-pc-windows-msvc` | 79,685,620 | 226,687,568 |
| `aarch64-pc-windows-msvc` | 65,217,448 | 200,585,808 |

Total additional source download: **331,130,168 bytes** (about 316 MiB).
Inspection used one process and one archive at a time; all downloaded archives
and extracted inspection temporaries were removed after pins were recorded.
Allow at least 1 GiB staging per sequential target. Final gzip bytes/hashes are
still unknown for these five targets; native testing and publishing are separate
steps, not implied by verified build inputs. Windows ARM64's import audit required a second verified 65,217,448-byte
source download; that inspection also removed its own temporary files.

### Repeating the Windows static audit

Run `python scripts/inspect_pe_imports.py /path/to/verified/rustfmt.exe` and the
same command for its pinned rustc_driver DLL. Only inspect files after verifying
the source archive and exact member pins. The parser checks file bounds, PE32+
headers, data directories, section overlap and RVA-to-file mapping, DLL-name
termination, and ordinary/delay import termination. It never executes or loads
the binaries. It intentionally rejects unsupported PE layouts rather than
inferring a closure. Tests cover ARM64 headers, ordinary/delayed imports and
malformed bounds/addressing. Static imports cannot establish OS compatibility;
a clean native runner remains required before publishing/enabling a bundle.

## Manual native candidate acceptance (no release)

The `Verify unpublished native language packs` Actions workflow is deliberately
`workflow_dispatch` only. It uses the same existing native Linux ARM64, macOS
x64/ARM64 and Windows x64/ARM64 hosted runners, at most two at once. It does not
publish releases, upload candidate bundles, enable a download URL, or alter the
checked-in application catalog. Each job has a 30-minute limit and requires at
least 2 GiB free before candidate work; the five official component inputs total
approximately 316 MiB, in addition to ordinary Cargo/toolchain/backend setup.

`scripts/verify_native_language_pack.py` builds one candidate from reviewed source
pins, verifies the complete candidate/source/provenance relationship, temporarily
embeds that exact unpublished candidate for the current target, and runs the real
managed install/format/project/uninstall integration test. The temporary catalog
continues to say `available: false`. Original catalog bytes are restored even on
ordinary failure, and the workflow independently checks that tracked files and
production download availability remain unchanged. Temporary source/bundle/runtime
files are removed; only bounded logs, candidate metadata and an acceptance report
are retained for seven days. No foreign binary is executed on this local executor.

The Rust test uses the absolute managed executable and a cleared runtime
environment, not the runner's installed rustfmt. It checks the stable version,
preview idempotence/settings/edition/syntax diagnostics, selected project writes,
module isolation and stale review rejection, then verifies uninstall removes the
managed runtime while preserving unrelated files. The Rust compiler installed on
the runner is used only to build the test harness.

A successful native job is evidence for that runner OS, not every minimum OS,
GUI/quarantine/signing scenario, or a published GitHub-download installation.
Maintainer review and separate release approval are still required before adding
final GitHub asset URLs and enabling a target. Before sharing a successful result,
check the run's exact commit, all five job conclusions and per-target reports.
