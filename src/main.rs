#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

mod app_window;
mod assets;
#[cfg(windows)]
mod autostart;
#[cfg(windows)]
mod dps_overlay;
#[cfg(windows)]
mod game_window;
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
    let autostart = cfg!(windows) && std::env::args_os().skip(1).any(|arg| arg == "--autostart");
    #[cfg(windows)]
    if !privileges::ensure_high_integrity(autostart)? {
        return Ok(());
    }
    #[cfg(windows)]
    let Some(instance) = single_instance::SingleInstance::acquire(!autostart)? else {
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

            let options = WindowOptions {
                show: !autostart,
                focus: !autostart,
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
            cx.on_window_closed(move |cx, id| {
                if id == _window.window_id() {
                    cx.quit();
                }
            })
            .detach();
            #[cfg(windows)]
            if let Err(error) = tray::init(_window, cx) {
                eprintln!("Failed to create system tray icon: {error:#}");
                if autostart {
                    app_window::show(_window, cx);
                }
            }
            #[cfg(windows)]
            cx.spawn(async move |cx| {
                while activations.recv().await.is_ok() {
                    cx.update(|cx| app_window::show(_window, cx));
                }
            })
            .detach();
            if !autostart {
                app_window::show(_window, cx);
            }
        });
    // Keep the named mutex alive until GPUI's event loop has stopped.
    #[cfg(windows)]
    drop(instance);
    Ok(())
}
