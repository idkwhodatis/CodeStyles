#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod ui;

use gpui_kit::*;

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
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
                title: Some("CodeStyles · P0".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        if let Err(error) =
            gpui_kit::open_window(options, cx, |_, cx| cx.new(ui::PreviewWindow::new))
        {
            eprintln!("Could not open CodeStyles window: {error}");
            cx.quit();
        }
        cx.activate(true);
    });
}
