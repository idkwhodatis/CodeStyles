# Windows window-drag investigation — 2026-10-19

## Verified read-path improvement

The pinned gpui-pre 0.3.9 Windows backend routes `WM_MOVE` through the window's
moved callback. GPUI's `bounds_changed` refreshes the window, so CodeStyles
rebuilds settings rows while moving. Each `SchemaField::state` checks plugin
identity and reads its value; previously every `ConfigDocument::value` reparsed
the complete JSONC document. Unchanged redraws now reuse exact path-read results.
CST editing and duplicate-key/error semantics are unchanged. Successful edits,
raw apply and discard invalidate the cache; clones detach on mutation.

This removes verified work from the redraw path. It is **not confirmation that
Windows ghosting is fixed**: no Windows desktop reproduction or GPU/frame-time
measurement was available. The pinned backend already has the modal move/size
loop timer fix described in [upstream #10703](https://github.com/zed-industries/zed/issues/10703).
No extra timer, renderer override or dependency fork was introduced.

## Measurement

The ignored `config::tests::settings_redraw_read_cost` test measures 100 redraws
of 30 distinct bundled TypeScript schema fields, including their identity checks.
It includes the first cold-cache redraw. It does not measure layout, GPU work,
actual frame rate or the default visible category's row count.

Linux x86-64, Rust 1.99.1, optimized release profile, same benchmark transplanted
onto the pre-change `72dc79e` implementation for comparison:

- Small pinned config: 42.64 ms before, 1.15 ms after, total for 100 redraws.
- Same config plus 100 unknown object properties: 2,996.49 ms before,
  9.04 ms after, total for 100 redraws.

Run with:

```sh
cargo test --release --locked --no-default-features --lib settings_redraw_read_cost -- --ignored --nocapture
```

These are single-run microbenchmarks, not a Windows performance guarantee. The
small-config release read cost was already below one millisecond per redraw.

## Automated checks

- Linux: full Rust tests, all-target clippy with warnings denied, formatting,
  desktop debug build, resource verification and whitespace checks.
- Regression coverage: repeated reads; duplicate keys/parents; non-object
  parents; missing versus null; literal dotted keys; invalid paths; raw edits;
  structured set/remove; rejected raw input; discard; cloned documents; save;
  fresh disk reload; shared concurrent reads and Send/Sync preservation.
- Existing native Windows x64 and ARM64 CI lanes run the same library and UI
  tests, clippy, desktop build and package checks. Their status must be checked
  on the published commit. They do not exercise interactive window dragging.

## Pending Windows visual acceptance

Compare the old and new builds with the same config, window size and monitor:

1. Repeatedly drag the operating-system title bar for 10 seconds, release, and
   repeat. Check whether trails appear only during motion or remain afterward.
2. Repeat with a larger imported config, the Styles page's fuller categories,
   and the Project/Editor pages. Resize, maximize/restore and minimize/restore.
3. Test light/dark themes and moving between different DPI/refresh-rate monitors
   when available. Confirm settings changes, reset, unset, raw apply and discard
   remain immediately visible after dragging.
4. If artifacts persist, compare a separate launch using the backend's supported
   `GPUI_DISABLE_DIRECT_COMPOSITION=1` environment variable against its default
   DirectComposition path. This is a diagnostic A/B only, not a persisted app
   setting or a shipped default. Record Windows build, GPU/driver, scaling,
   refresh rate, and which presentation path changes the symptom.
