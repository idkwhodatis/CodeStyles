# Native distributions

`python scripts/package_app.py --target TARGET --output-dir dist` packages an
already-built native `target/release/codestyles` (or `codestyles.exe`). Use
`--binary PATH` for an audited/stripped binary. The version comes from `Cargo.toml`;
`--version` is an explicit override. Supported targets are the six triples in
`resources/manifest.json`.

| Platform | Outputs | Runtime resource location |
| --- | --- | --- |
| Windows x64 / ARM64 | portable `.zip`, per-user `-setup.exe` | `resources/` beside `codestyles.exe` |
| macOS Intel / Apple Silicon | `.app.zip`, drag-to-Applications `.dmg` | `CodeStyles.app/Contents/Resources/resources/` |
| Linux x64 / ARM64 | portable `.tar.gz`, `.deb` | beside the app binary, including `/usr/lib/codestyles/resources/` in the Debian install |

Names are `codestyles-VERSION-TARGET` followed by the extension above. Windows
ZIPs have a flat root. macOS ZIPs have one `CodeStyles.app` root. Linux tarballs
have a `codestyles-VERSION-TARGET/` root. Extract the complete portable package;
copying only the executable loses its offline formatter resources. Linux Debian
packages add a `/usr/bin/codestyles` launcher, desktop entry, SVG icon and licenses
under `/usr/share/doc/codestyles/`.

To generate only one artifact, use `--output FILE` with its desired extension.
The legacy `--output FILE.zip` form remains supported on every platform. On macOS
it now produces a proper application bundle. All archive names use forward slashes.
ZIP/tar/Debian file permissions and timestamps are deterministic; ZIP, tar and
Debian archives are reproducible for identical inputs. DMG and Inno Setup output
are not claimed byte-identical between builds.

## Native tools and platform metadata

- Windows installers require **Inno Setup 6.3+**. The compiler is discovered using
  `--iscc PATH`, the `ISCC` environment variable, `PATH`, or its standard Inno
  Setup 6 Program Files directory. `windows.iss` runs without elevation, installs
  under the current user's Local AppData, and supplies uninstall/start-menu
  integration. The optional desktop shortcut defaults off. A silent install does
  not launch the application. x64 uses `x64compatible`; ARM64 uses `arm64`.
  Both installers declare Windows 10 **22H2 (19045)** as their target floor;
  oldest-OS execution remains unvalidated. The setup/uninstall helper stays
  x86, which [Inno Setup documents](https://jrsoftware.org/ishelp/topic_setup_setuparchitecture.htm)
  as supported through emulation on Windows 10 ARM64 as well as Windows 11.
  Inno 7+ explicitly sets `SetupArchitecture=x86`; older supported compilers
  already emit that helper architecture.
- DMG creation requires macOS's `hdiutil`. The DMG contains the complete app plus
  an Applications shortcut. `Info.plist` declares Intel macOS **10.15.7** and Apple
  Silicon macOS **11.0**. These are compatibility targets, not proof that newer
  framework/library dependencies run there; the separate dependency audit and
  oldest-OS GUI tests must establish that.
- `.deb` creation uses only Python; it does not require `dpkg`, root, or Debian as
  the build host. `Architecture` is `amd64` or `arm64`, and all installed files
  are root-owned in the archive. Dependencies declare glibc **2.28** and the GUI
  libraries that can be loaded at runtime. `--deb-depends` accepts an explicitly
  audited dependency list for alternative build environments. ELF symbol/import
  auditing remains a separate gate; package metadata cannot lower a binary's ABI.

Packaging does **not** download tools or resources, sign, notarize, tag, release,
or upload anything. Installer and DMG generation are native CI steps. Artifact
existence does not establish OS compatibility, clean-machine installation,
uninstall behavior, or an interactive GUI smoke result.

## Integrity and licenses

Every selected formatter executable, WASM plugin, schema and pinned license is
checked against the committed manifest before copying and again after staging.
The full manifest, resource README, notices, all resource licenses, and application
MIT license are included. The original manifest is never rewritten. No other
platform's formatter is included. Corruption/missing files fail before artifacts
are published. An output is atomically replaced only after its format writer
succeeds; a failed native tool cannot replace the previous artifact with a
partial file.

Run `python -m unittest discover -s scripts -p 'test_*.py'` for offline fixture
coverage of all archive layouts, both CPU architectures, plist/control metadata,
resource integrity failures, reproducibility, permissions and native-tool command
contracts. When `dpkg-deb` is available, the tests independently parse the emitted
Debian packages. Mocked native-tool contracts do not replace real installer/DMG
validation on their native OS.

Windows installers are built with [Inno Setup](https://jrsoftware.org/),
by Jordan Russell and Martijn Laan. Its existing notices are retained in the
installer. The tested hosted compiler is 6.7.1; see its
[upstream license](https://github.com/jrsoftware/issrc/blob/is-6_7_1/license.txt)
and the vendor's [commercial-license FAQ](https://jrsoftware.org/isorder.php).
No license purchase or key configuration is performed by these scripts.
