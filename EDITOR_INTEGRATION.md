# Editor and CI integration

CodeStyles previews and saves dprint configuration. Your editor and CI should use
the **same saved configuration, dprint 0.63.0, and exact plugin versions**.
Unsaved CodeStyles settings have no effect in either place.

The integration page supplies a copyable absolute path and JSON settings. These
are instructions for you to apply: the Editor & CI page does not install an extension,
modify editor settings, or alter your project source files.

## VS Code

1. Install the official [dprint extension, `dprint.dprint`](https://marketplace.visualstudio.com/items?itemName=dprint.dprint).
2. Save your configuration in the project using a discoverable name:
   `dprint.json`, `dprint.jsonc`, `.dprint.json`, or `.dprint.jsonc`.
   The extension discovers configuration for the file being edited. Its documented
   limitations include arbitrary custom config locations. CodeStyles can open a
   differently named config, but that alone does not connect it to VS Code.
   See [dprint setup](https://dprint.dev/setup/) and the
   [extension guide](https://github.com/dprint/dprint-vscode#setup).
3. In VS Code's Command Palette, open **Preferences: Open User Settings (JSON)**.
   Merge the generated entries into your existing settings. Keep unrelated
   entries, and merge language sections rather than creating duplicate keys.
   The executable path is machine-specific, so keep `dprint.path` in user settings
   instead of committing it in `.vscode/settings.json`. Workspace settings can
   override user settings; inspect them if the selected executable differs.
   See [VS Code settings](https://code.visualstudio.com/docs/configure/settings).
4. Use **Dprint: Restart**, then **Format Document** on a file in the project.
   The generated snippet selects dprint for JavaScript, JSX, TypeScript, TSX,
   JSON, JSONC, Markdown, and TOML. Format on save is optional: add
   `"editor.formatOnSave": true` in the desired language sections if wanted.

For example, on Linux (replace the path with the one shown by CodeStyles):

```json
{
  "dprint.path": "/opt/CodeStyles/resources/bin/x86_64-unknown-linux-gnu/dprint",
  "dprint.ensureStableFormat": true,
  "[javascript]": { "editor.defaultFormatter": "dprint.dprint" },
  "[javascriptreact]": { "editor.defaultFormatter": "dprint.dprint" },
  "[typescript]": { "editor.defaultFormatter": "dprint.dprint" },
  "[typescriptreact]": { "editor.defaultFormatter": "dprint.dprint" },
  "[json]": { "editor.defaultFormatter": "dprint.dprint" },
  "[jsonc]": { "editor.defaultFormatter": "dprint.dprint" },
  "[markdown]": { "editor.defaultFormatter": "dprint.dprint" },
  "[toml]": { "editor.defaultFormatter": "dprint.dprint" }
}
```

`dprint.path` includes the executable name, including `.exe` on Windows. JSON
requires escaped backslashes (for example, `"C:\\CodeStyles\\dprint.exe"`). The
generated snippet uses JSON serialization to preserve backslashes, quotes,
Unicode, and control characters; the copy-path action supplies the raw path.
Relative or non-Unicode paths cannot produce a reliable snippet and receive an
error. The extension supports path expansion, so if an installation directory
contains expansion syntax such as `$HOME` or `${HOME}`, check how the extension
resolves it and prefer a path without those characters. Moving or removing
CodeStyles can invalidate this setting. A local path also does not identify an
executable inside SSH, WSL, or container environments; configure the matching
0.63.0 executable in the environment where the extension runs.

### What `dprint.ensureStableFormat` means

The extension's default is **false**. Setting it to **true** repeats formatting
until output stops changing, matching the stable-format behavior of `dprint fmt`
used by CodeStyles preview. The option requires dprint 0.59 or later, so the
bundled 0.63.0 supports it. Additional passes may increase formatting time.
It is a formatting-convergence setting, not a version pin or a promise that
different versions produce the same output. These semantics are defined in the
[official extension settings](https://github.com/dprint/dprint-vscode#extension-settings)
and [settings schema](https://github.com/dprint/dprint-vscode/blob/main/package.json).

### Configuration and offline boundaries

New CodeStyles configurations use exact TypeScript **0.96.1**, JSON **0.25.2**,
Markdown **0.26.0**, and TOML **0.9.0** plugin URLs. Imported configurations keep their original references;
do not silently replace project-specific plugins just to match this example.
Save the reviewed config and commit it through your normal workflow so the
editor, coworkers, and CI see the same values.

Pointing VS Code at the bundled executable does **not** activate CodeStyles'
preview-only mapping from pinned URLs to bundled WASM. The editor and CI read the
saved configuration normally and may download plugins or inherited configs into
their own caches. For an offline team setup, arrange trusted, portable local
resources or prepopulate the relevant cache separately. Avoid committing the
absolute paths of this installation. Review configurations before trusting them:
process plugins can execute programs, and remote references may use the network.

Use the real source path when diagnosing discrepancies: include/exclude patterns,
plugin overrides, nested configuration, relative `extends`, and inherited config
can affect a real project file differently from the built-in sample. Confirm the
editor has selected dprint and that the CodeStyles draft was actually saved.

## CI: pin the CLI as well as plugins

The official [`dprint/check` action](https://github.com/dprint/check#specific-version)
defaults to the latest dprint version. Set its `dprint-version` input explicitly.
Add this step **after your existing repository checkout**:

```yaml
- uses: dprint/check@v2
  with:
    dprint-version: "0.63.0"
```

This pins the CLI, not the action code. Pin the action itself to a reviewed commit
if required by your repository policy. The action uses the discovered project
configuration; its documented `config-path` input can select a custom config for
CI. Such a custom path does not remove the editor's discovery limitation.

For another CI provider, install the platform-matching executable from the
[official 0.63.0 release](https://github.com/dprint/dprint/releases/tag/0.63.0),
verify the downloaded artifact through your normal provenance/checksum process,
and run from the project root:

```sh
dprint --version
dprint check
```

Check that the first command reports **dprint 0.63.0**, and make sure these
commands invoke that installed executable. For an explicit CLI-only config, use
`dprint check --config path/to/dprint.jsonc`. Commit and use the same configuration
and exact plugin URLs; do not run a fresh `dprint init` in CI, since it can select
newer plugins. Do not copy a developer's absolute `dprint.path` setting into CI.
See [dprint CI documentation](https://dprint.dev/ci/).

## Validation scope

The guidance and settings were checked against the official extension README,
package settings (extension 0.18.2), VS Code settings documentation, dprint setup,
and `dprint/check` documentation on **2026-10-18**. The helper tests check JSON
round trips for Windows/Unix paths, Unicode and special characters, fail-closed
path handling, language formatter selection, and resource-manifest version
consistency. They do not launch VS Code or demonstrate an interactive extension
integration test. Extension settings may evolve; review the official links when
upgrading.
