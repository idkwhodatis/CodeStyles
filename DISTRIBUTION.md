# Native distribution and compatibility

P4 produces native x86-64 and ARM64 application bundles for Windows, macOS, and
Linux. Build-runner versions are **not** minimum supported operating systems.
The machine-readable policy is `resources/compatibility.json`. These are build
and validation targets, not a promise that every older machine has been tested.

| Platform | Compatibility goal | Distribution |
| --- | --- | --- |
| Windows x64 | Windows 10 22H2; older builds unverified | Portable ZIP, per-user Inno Setup installer |
| Windows ARM64 | Windows 10 ARM64 22H2 to be validated; current CI is Windows 11 | Native application ZIP and installer |
| macOS Intel | Catalina 10.15.7 | `.app.zip`, DMG |
| macOS Apple Silicon | Big Sur 11.0; M1 and later | `.app.zip`, DMG |
| Linux x64 / ARM64 | glibc 2.28 ABI goal plus documented desktop dependencies | tar.gz, Debian `.deb` |

## Build and artifact checks

`cargo build --locked --release`, then:

```
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

The auditor examines every ELF, Mach-O or PE in the application payload,
and reports architectures, deployment commands, imported libraries, RPATH and
symbol-version requirements. Windows GUI subsystem is checked for the app;
dprint is a separately distributed optional subprocess CLI. Installer bootstrap/uninstaller architecture is
reported separately from the native application. ABI headers and import names do
not prove API availability or successful rendering on an older OS.

`--self-test <report.json>` checks offline resources/configuration without
initializing a GUI. Clean application packages must pass without an installed
dprint runtime, explicitly skipping formatting rather than using PATH or fetching
anything. Real formatting is verified separately against the managed runtime pack.
The archive and native-container tests start with an absent formatter cache and
require it to remain absent. The isolated managed-tool root may contain only its
valid ownership marker, never runtime payloads or staging files. Tests also use
Unicode/space paths, an unrelated working directory and installation-tree content
snapshots.
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
  Vulkan as the only rendering route. [File dialogs](#linux-file-dialogs),
  clipboard and IME services depend on the desktop session. The Debian
  dependencies are explicit; tarball users must provide the same runtime
  libraries and services. A glibc floor alone does not make every distribution
  with that libc supported.
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

### Linux file dialogs

Open, Save As, configuration import and local-pack selection use GPUI's
`org.freedesktop.portal.FileChooser` calls on **both X11 and Wayland**. They need
`xdg-desktop-portal` and a backend that actually implements FileChooser in the
same graphical user's D-Bus session. There is no direct GTK dialog fallback.
The `.deb` requires `xdg-desktop-portal (>= 1.0)` plus one of
`xdg-desktop-portal-gtk`, `xdg-desktop-portal-kde` or `xdg-desktop-portal-gnome`.
Portable tar/ZIP users must install the equivalent packages from their distro.
Use the backend appropriate to the desktop (GTK is a general fallback, KDE for
Plasma, GNOME for GNOME), with its distro-declared dependencies. A generic portal
backend is insufficient: [xdg-desktop-portal-wlr](https://github.com/emersion/xdg-desktop-portal-wlr#readme)
implements screenshot/screencast, not FileChooser, and needs another backend for
file dialogs.

The current picker flows select files, not directories. Their `OpenFile` and
`SaveFile` methods and required options are already in the
[FileChooser v1 API shipped with portal 1.0](https://github.com/flatpak/xdg-desktop-portal/blob/1.0/data/org.freedesktop.portal.FileChooser.xml).
Directory selection would require a separate review of FileChooser v3 and backend
support. This service requirement does not change the **glibc 2.28** binary ABI
goal or establish compatibility with every distro at that floor.

If a picker reports that `org.freedesktop.portal.Desktop` has no service file,
check that the portal frontend is installed and visible to the graphical session's
D-Bus. If FileChooser is unavailable, check the installed backend and the
desktop's portal selection configuration. The portal activation environment must
also know the session's display and desktop variables. Follow upstream
[system integration](https://flatpak.github.io/xdg-desktop-portal/docs/system-integration.html)
and [backend selection](https://flatpak.github.io/xdg-desktop-portal/docs/configuration-file.html)
guidance for the installed portal version; do not replace the desktop's
configuration indiscriminately. CodeStyles does not install or reconfigure these
services. Verify Open, Save As, cancel and reopen in the actual desktop session;
package inspection and headless self-tests do not establish working file dialogs.

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

The script verifies offline resources, signs and seals the application, and reruns
the missing-runtime startup smoke test before notarization. It requires an explicit
Apple `Accepted` result and staples/validates the app and DMG. Runtime packs are
separate artifacts: their signing, JIT entitlements, notarization and native
execution must be validated before enabling a macOS dprint catalog target. No
signing/notarization success is claimed until the opt-in workflow actually passes.

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

## Fixed alpha release notes finalization

The native packages in the existing `v0.1.0-alpha.1` release (ID `416881817`)
were refreshed from commit `50d54d8a299eca891167dd251306fc67f1bffac1` and native
CI run [39200102985](https://github.com/idkwhodatis/CodeStyles/actions/runs/39200102985).
All fourteen assets were uploaded before the subsequent tag update was denied.
The tag remains at `5135f7af5cf1da08b985e19470291f0d1cdcfe85` by the owner's
explicit decision to leave source handling alone for now. GitHub's automatic
Source code ZIP/tar.gz therefore still represent that older source. The release
notes clearly disclose this and link directly to archives of the exact binary
build commit for anyone who needs matching source.

`.github/workflows/release-alpha.yml` now performs **notes-only finalization**.
Its read-only preparation job verifies the fixed new CI run's six successful
native jobs, repository/event/commit provenance and artifact SHA-256 values. It
copies only the twelve expected packages, never executes artifact contents,
and recreates their `PROVENANCE.json` and `SHA256SUMS`.

The finalization job verifies the exact release ID/repository/tag, old tag SHA,
prerelease status, and all fourteen new asset digests and uploaded states before
editing anything. Its sole mutation is a release PATCH containing `body` only.
It never uploads/deletes assets, moves tags, changes `target_commitish`, changes
release status, or retries the denied tag operation. A missing, duplicate,
unexpected or mismatched asset blocks the notes update instead of repairing it.
The final check repeats tag, asset and body verification. Repeating a completed
finalization is read-only.

The existing job-scoped ephemeral `contents: write` permission is retained for
the notes PATCH; no additional permission, stored credential, or persistent
access is added. The workflow runs on changes to its release workflow/script on
master or manually from master, and stays pinned to this exact build. It does
not automatically publish future builds.

Download links in the release body are ordered Windows, macOS, Linux, with x64
before ARM64 on each platform. GitHub controls its separate native Assets list.
Package filenames and embedded `0.1.0` versions are unchanged. macOS packages
are unsigned and not notarized; Windows installers are unsigned. Existing
OS/dependency compatibility notes and manual desktop acceptance requirements
above continue to apply. Successful headless CI does not prove desktop launch.

The earlier refresh workflow retained original packages and a pre-refresh
manifest in `alpha-before-refresh-1` under run
[39204651813](https://github.com/idkwhodatis/CodeStyles/actions/runs/39204651813),
with requested 90-day retention subject to repository policy. This notes-only
workflow does not replace or delete that backup. Restoring packages or changing
the tag would require a separate authorized task.
