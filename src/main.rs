#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

mod app_window;
mod assets;
mod keyboard;
mod language;
#[cfg(windows)]
mod privileges;
mod repeat;
mod settings;
rust_i18n::i18n!("locales", fallback = "zh-CN");
mod navigation;
#[cfg(windows)]
mod single_instance;
mod title_bar;
#[cfg(windows)]
mod tray;
#[cfg(windows)]
mod updater;

use gpui_kit::{
    component::{Theme, ThemeMode, TitleBar},
    *,
};

fn main() -> Result<()> {
    #[cfg(windows)]
    if !privileges::ensure_high_integrity()? {
        return Ok(());
    }
    #[cfg(windows)]
    let Some(instance) = single_instance::SingleInstance::acquire()? else {
        return Ok(());
    };
    #[cfg(windows)]
    let activations = instance.listen()?;

    gpui_kit::application()
        .with_assets(assets::AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            language::init();
            Theme::change(ThemeMode::Dark, None, cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1440.0), px(860.0)),
                    cx,
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Noia DNF".into()),
                    ..TitleBar::title_bar_options()
                }),
                window_min_size: Some(size(px(720.0), px(520.0))),
                ..TitleBar::window_options()
            };

            let (_window, _) = gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| navigation::AppView::new(window, cx))
            })
            .expect("Failed to open the Noia DNF window");
            #[cfg(windows)]
            if let Err(error) = tray::init(_window, cx) {
                eprintln!("Failed to create system tray icon: {error:#}");
            }
            #[cfg(windows)]
            cx.spawn(async move |cx| {
                while activations.recv().await.is_ok() {
                    cx.update(|cx| app_window::show(_window, cx));
                }
            })
            .detach();
            app_window::show(_window, cx);
        });
    // Keep the named mutex alive until GPUI's event loop has stopped.
    #[cfg(windows)]
    drop(instance);
    Ok(())
}
