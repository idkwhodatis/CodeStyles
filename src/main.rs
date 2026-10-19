#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod preview_display;
mod ui;

use gpui_kit::*;

gpui_kit::actions!(codestyles_app, [Quit, SaveConfig, SearchSettings]);

fn main() {
    // This branch deliberately precedes GPUI/platform initialization. Package
    // diagnostics run on headless CI without a display, GPU, or window server.
    match codestyles::self_test::requested_report(std::env::args_os().skip(1)) {
        Ok(Some(path)) => {
            if let Err(error) = codestyles::self_test::run(&path) {
                eprintln!("CodeStyles self-test failed: {error}");
                std::process::exit(1);
            }
            return;
        }
        Ok(None) => {}
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
        let quit_shortcut = if cfg!(target_os = "macos") {
            "cmd-q"
        } else {
            "ctrl-q"
        };
        cx.bind_keys([KeyBinding::new(quit_shortcut, Quit, None)]);
        let modifier = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        cx.bind_keys([
            KeyBinding::new(&format!("{modifier}-s"), SaveConfig, None),
            KeyBinding::new(&format!("{modifier}-shift-f"), SearchSettings, None),
        ]);
        cx.set_menus([Menu::new("CodeStyles").items([MenuItem::action("Quit CodeStyles", Quit)])]);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let options = WindowOptions {
            window_min_size: Some(size(px(1100.0), px(700.0))),
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1440.0), px(900.0)),
                cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("CodeStyles".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        match gpui_kit::open_window(options, cx, |window, cx| {
            let view = cx.new(ui::PreviewWindow::new);
            let weak = view.downgrade();
            window.on_window_should_close(cx, move |window, cx| {
                weak.update(cx, |view, cx| view.should_close(window, cx))
                    .unwrap_or(true)
            });
            view
        }) {
            Ok((handle, view)) => {
                let save_view = view.downgrade();
                cx.on_action(move |_: &SaveConfig, cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        let _ = save_view.update(cx, |view, cx| view.save(window, cx));
                    });
                });
                let search_view = view.downgrade();
                cx.on_action(move |_: &SearchSettings, cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        let _ = search_view.update(cx, |view, cx| view.focus_search(window, cx));
                    });
                });
                let weak = view.downgrade();
                // App-owned Quit uses the same cancellable path as window close.
                // GPUI's native OS/Dock termination callback cannot be vetoed;
                // do not imply this covers forced termination or crash recovery.
                cx.on_action(move |_: &Quit, cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        if weak
                            .update(cx, |view, cx| view.should_close(window, cx))
                            .unwrap_or(true)
                        {
                            window.remove_window();
                        }
                    });
                });
            }
            Err(error) => {
                eprintln!("Could not open CodeStyles window: {error}");
                cx.quit();
            }
        }
        cx.activate(true);
    });
}
