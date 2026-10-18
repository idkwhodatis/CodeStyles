#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod ui;

use gpui_kit::*;

gpui_kit::actions!(codestyles_app, [Quit]);

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
        let quit_shortcut = if cfg!(target_os = "macos") {
            "cmd-q"
        } else {
            "ctrl-q"
        };
        cx.bind_keys([KeyBinding::new(quit_shortcut, Quit, None)]);
        cx.set_menus([Menu::new("CodeStyles").items([MenuItem::action("Quit CodeStyles", Quit)])]);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let options = WindowOptions {
            window_min_size: Some(size(px(820.0), px(520.0))),
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1120.0), px(760.0)),
                cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("CodeStyles · P1".into()),
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
