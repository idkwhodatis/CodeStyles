# Native distribution and compatibility

P4 produces native x86-64 and ARM64 application bundles for Windows, macOS, and
Linux. Build-runner versions are **not** minimum supported operating systems.
The machine-readable policy is `resources/compatibility.json`. These are build
and validation targets, not a promise that every older machine has been tested.

| Platform | Compatibility goal | Distribution |
| --- | --- | --- |
| Windows x64 | Windows 10 22H2; older builds unverified | Portable ZIP, per-user Inno Setup installer |
| Windows ARM64 | Windows 10 ARM64 22H2 to be validated; current CI is Windows 11 | Native application/dprint ZIP and installer |
| macOS Intel | Catalina 10.15.7 | `.app.zip`, DMG |
| macOS Apple Silicon | Big Sur 11.0; M1 and later | `.app.zip`, DMG |
| Linux x64 / ARM64 | glibc 2.28 ABI goal plus documented desktop dependencies | tar.gz, Debian `.deb` |

## Build and artifact checks

`cargo build --locked --release`, then:

```
python scripts/bundle_resources.py fetch --target <native-target>
python scripts/package_app.py --target <native-target> --output-dir dist
python scripts/check_distribution.py --target <native-target> --directory dist
```

The packager verifies committed upstream SHA-256 pins before copying resources.
Resources are resolved relative to the executable, including standard macOS
`Contents/Resources`, regardless of the working directory. Release builds never
fall back to a developer's build checkout. `CODESTYLES_RESOURCES` remains an
explicit development/test override; distribution tests remove it.

CI builds Linux on native x64/ARM64 hosts **inside digest-pinned
manylinux_2_28 containers**, not against the Ubuntu host's libc. The base is
AlmaLinux 8/glibc 2.28. RPM development dependencies receive distribution
updates; each artifact includes their exact installed inventory. This is not a
bit-for-bit reproducible dependency snapshot. Never use `target-cpu=native`.
The whole-bundle ELF audit rejects newer GLIBC/GLIBCXX/CXXABI requirements rather
than silently raising the baseline. A failed dependency build or ABI audit blocks
that lane and requires investigation before choosing a newer baseline.

The auditor examines every ELF, Mach-O or PE in the payload, including dprint,
and reports architectures, deployment commands, imported libraries, RPATH and
symbol-version requirements. Windows GUI subsystem is checked for the app;
dprint is a subprocess CLI. Installer bootstrap/uninstaller architecture is
reported separately from the native application. ABI headers and import names do
not prove API availability or successful rendering on an older OS.

`--self-test <report.json>` runs the installed application's real formatter path
without initializing a GUI. It checks all four bundled WASM plugins/eight source
languages. The archive and native-container tests use fresh caches, Unicode/space
paths, an unrelated working directory and installation-tree content snapshots.
Windows tests silently install into a disposable per-user directory and uninstall;
macOS tests mount the DMG read-only; Linux tests inspect/extract the actual Debian
payload. Linux repeats its tests in an unprivileged Docker container with
`--network none`, a read-only root and a read-only source/artifact mount. The
writable temporary/cache/report directories are outside the installation tree.
CI requires an effective installation write-denial probe on every platform; a
Windows attribute-only fallback is reported but cannot pass that gate.
Windows/macOS unreachable proxies are only a network-deterrence check, **not** a
network isolation guarantee. These headless tests do not establish GUI first
launch or display behavior.

## Runtime requirements and evidence

- Linux needs usable X11 or Wayland, font discovery/fonts, xkbcommon/XCB and a
  working GPU/driver backend. Locked GPUI enables Vulkan **or** GL; do not describe
  Vulkan as the only rendering route. Portal/file-dialog, clipboard and IME
  services depend on the desktop session. The Debian dependencies are explicit;
  tarball users must provide the same runtime libraries. A glibc floor alone does
  not make every distribution with that libc supported.
- Fonts are currently supplied by the system. GPUI Kit embeds icons, not a full
  text-font family. Font fallback and Chinese/emoji glyph coverage remain part of
  desktop acceptance. No bundled-font guarantee is made.
- The locked GPUI Windows source uses DirectWrite custom-font APIs introduced in
  the Windows 10 era (its `IDWriteFactory5` code says 1703+). That is a component
  lower bound, not proof of the entire application's oldest working Windows.
- The locked GPUI Apple Metal build sets 10.15.7, and Rust's Apple ARM target starts
  at 11.0. Explicit deployment targets and final Mach-O audits keep these goals
  separate from the SDK and runner OS versions.
- The pinned dprint 0.63.0 payload was independently inspected: Linux GLIBC 2.17,
  macOS Intel minOS 10.12 and ARM minOS 11.0. Those facts do not replace auditing
  the final CodeStyles binary and full package.
- manylinux is being used as an old-ABI **build environment**. CodeStyles is not a
  Python wheel and does not claim manylinux/auditwheel certification.

Sources: [Rust Apple targets](https://doc.rust-lang.org/rustc/platform-support/apple-darwin.html),
[Rust MSVC targets](https://doc.rust-lang.org/rustc/platform-support/windows-msvc.html),
[DirectWrite custom fonts](https://learn.microsoft.com/en-us/windows/win32/directwrite/custom-font-sets-win10),
[manylinux](https://github.com/pypa/manylinux),
[locked wgpu 29.0.4](https://github.com/gfx-rs/wgpu/tree/v29.0.4),
[dprint 0.63.0](https://github.com/dprint/dprint/releases/tag/0.63.0).

## Optional Apple signing and notarization

Ordinary CI artifacts are **unsigned development distributions**, not notarized
public releases. No signing account, certificate or credential is included or
created by this change, and no GitHub release/tag is published.

A separate manual `sign-macos.yml` workflow is provided but is not run by PRs or
ordinary pushes. To enable it, the owner must first configure a protected
`macos-signing` environment (approval required is recommended) with their existing
Developer ID certificate and notarization account. Required secrets:
`MACOS_SIGNING_IDENTITY`, `MACOS_CERTIFICATE_BASE64`, `MACOS_CERTIFICATE_PASSWORD`,
`MACOS_TEMP_KEYCHAIN_PASSWORD`, `MACOS_APPLE_ID`, `MACOS_TEAM_ID`,
`MACOS_APP_PASSWORD`. Never commit or paste those values into issues or logs.
The manual run ID must identify a successful push build of the exact current
master SHA; PR build artifacts are refused. Importing the existing identity is
limited to a temporary runner keychain, removed even on failure.

The script signs dprint with the JIT entitlement, checks its original bytes
against the committed upstream pin, preserves `upstream_sha256`, updates only the
runtime manifest checksum for the signed binary, and seals the app. WASM pins
remain unchanged. It reruns all formatter paths on the signed app before
notarization, requires an explicit Apple `Accepted` result, staples/validates the
app and DMG, and runs Gatekeeper assessment. The signed manifest is protected by
the app's code signature. No claim of signing/notarization success is made until
the opt-in workflow succeeds with real credentials. Actual entitlement behavior
must be established by that run; do not relax hardened-runtime policy to hide a
failure.

Alternatively, on a Mac with an existing unlocked signing identity and an
already-configured notarytool profile:

```
python scripts/sign_macos.py --app CodeStyles.app --target aarch64-apple-darwin --identity '<Developer ID identity>' --notary-profile '<existing profile>' --output signed
```

## Required manual desktop acceptance (unperformed unless recorded)

For each OS/architecture, record OS build, GPU/driver, artifact SHA-256, tester,
date, and result. Check the compatibility-goal OS as well as current systems.

- Install/extract as an ordinary user; launch by the normal OS shortcut/Finder/
  file manager from a read-only install location, with a clean profile and network
  disconnected. Render a window and format TS, JS, TSX, JSX, JSON, JSONC, Markdown
  and TOML before any network access.
- Confirm readable system fonts and fallback glyphs (Chinese/emoji), light/dark,
  100%/150%/200% scaling, multi-monitor movement and resize.
- Copy/paste through the real system clipboard; compose Chinese/Japanese with
  IME, commit/cancel composition, and verify keyboard shortcuts/focus traversal.
- Exercise native file picker, trust/save/discard dialogs, Unicode project paths,
  save/reopen, cancellation, repeated launches and uninstall cleanup.
- Windows: launch normally and verify **no console window**; ARM: verify native
  main/dprint processes and record installer helper emulation if applicable.
- macOS: verify Finder launch, Dock/menu behavior, Gatekeeper quarantine behavior,
  and signed/notarized download acceptance when credentials become available.
- Linux: test X11 and Wayland sessions, both available graphics backend paths,
  clipboard/IME/portal behavior and actual Debian install/remove on a supported
  distro. A container headless run is not a desktop GPU acceptance test.

## Fixed alpha refresh

`.github/workflows/release-alpha.yml` refreshes the existing `v0.1.0-alpha.1`
release (ID `416881817`) in place, retaining its release URL and prerelease status.
The only permitted source is commit `50d54d8a299eca891167dd251306fc67f1bffac1`
and native CI run [39200102985](https://github.com/idkwhodatis/CodeStyles/actions/runs/39200102985).
The only permitted previous source is commit
`5135f7af5cf1da08b985e19470291f0d1cdcfe85`, run
[39194115622](https://github.com/idkwhodatis/CodeStyles/actions/runs/39194115622).
It runs on changes to the release workflow/script on master, or manually from
master. It does not follow future build completions or publish future versions.

The read-only preparation job independently verifies both fixed CI runs: six
successful native jobs, repository/event/commit provenance, and each GitHub
artifact SHA-256. Only twelve expected native packages are copied from each
Actions artifact set; contents are never executed. Package bytes, filenames and
embedded `0.1.0` versions are retained. `PROVENANCE.json` records source artifact
IDs and digests; `SHA256SUMS` covers the packages and provenance. This is verified
byte reuse, not a claim of bit-for-bit reproducibility by rebuilding.

Only the publishing job receives ephemeral `contents: write`; no stored
credential, signing secret, or new persistent grant is required. Before changing
anything, it validates the existing release and saves a manifest of release/tag
metadata and asset checksums. That manifest and all original packages are uploaded
to an `alpha-before-refresh-<attempt>` workflow artifact with 90-day requested
retention (repository/organization policy may cap this). Failure to save this
backup blocks the refresh. The prepared new packages are also retained as a
workflow artifact. The original and new CI runs remain independent recovery
sources until their artifacts expire.

The script refuses any release ID/repository/tag outside this fixed scope, a tag
at a third commit, non-prerelease/draft/immutable releases, duplicate or unknown
asset names, or bytes that match neither independently validated source set.
It rechecks this allowlist before each replacement. Exact new assets are kept;
verified old assets are deleted individually and immediately replaced, with
server-side digest checks. This is not atomic: during refresh a download may be
briefly unavailable, and the public asset set can temporarily contain both builds.
Do not download the release until the workflow has completed successfully.
The release itself is never deleted or recreated.

After all fourteen new assets are verified, the approved lightweight tag is
moved from the old source to the new source using an exact `--force-with-lease`
check, so a concurrent move to an unexpected commit is not overwritten. Git
uses the same ephemeral token through a command-local credential helper; no
credential or persistent helper configuration is saved. Release metadata is updated with
`make_latest: false`. This deliberately changes the existing alpha tag; it is not
a new version. Retries accept missing assets and mixtures of exactly the known
old/new bytes, including interruption after a delete, after uploads, or after the
tag move. Fully refreshed retries perform no writes. Unexpected or incomplete
upload states fail closed and require investigation rather than arbitrary
clobbering. In particular, GitHub can leave an empty `starter` asset after a
failed upload; recovery requires inspecting that asset and explicit authorization
to remove it, not an unrestricted automatic deletion. Manually rerun the same workflow to resume a known interrupted refresh.
Restoration from the backup is a separate, explicitly authorized operation.

The release body provides direct downloads in Windows, macOS, Linux order, with
x64 before ARM64 on each platform. GitHub controls its separate native Assets
list order; the workflow does not claim to control it.

macOS assets are unsigned and not notarized, and Windows installers are unsigned.
All existing OS/dependency compatibility notes above continue to apply.
