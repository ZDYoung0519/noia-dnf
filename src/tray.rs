use crate::language::text;
use gpui_kit::{AnyWindowHandle, App, Global, Result};
use tray_icon::{
    Icon, MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

enum Command {
    Show,
    Quit,
}

// Keep the native icon alive on GPUI's Windows event-loop thread.
struct SystemTray {
    _icon: TrayIcon,
    show: MenuItem,
    quit: MenuItem,
}

impl Global for SystemTray {}

pub fn refresh_labels(cx: &App) {
    if let Some(tray) = cx.try_global::<SystemTray>() {
        tray.show.set_text(text("tray.show").as_str());
        tray.quit.set_text(text("tray.quit").as_str());
    }
}

pub fn init(window: AnyWindowHandle, cx: &mut App) -> Result<()> {
    let image = image::load_from_memory(crate::assets::APP_LOGO_BYTES)?
        .resize_exact(32, 32, image::imageops::FilterType::Lanczos3)
        .into_rgba8();
    let icon = Icon::from_rgba(image.into_raw(), 32, 32)?;
    let menu = Menu::new();
    let show = MenuItem::new(text("tray.show").as_str(), true, None);
    let quit = MenuItem::new(text("tray.quit").as_str(), true, None);
    menu.append_items(&[&show, &PredefinedMenuItem::separator(), &quit])?;

    let tray = TrayIconBuilder::new()
        .with_tooltip("Noia DNF")
        .with_icon(icon)
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()?;

    // Wake GPUI through a channel; never re-enter App from a native callback.
    let (sender, receiver) = async_channel::unbounded();
    let menu_sender = sender.clone();
    let show_id = show.id().clone();
    let quit_id = quit.id().clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if event.id == show_id {
            let _ = menu_sender.try_send(Command::Show);
        } else if event.id == quit_id {
            let _ = menu_sender.try_send(Command::Quit);
        }
    }));
    TrayIconEvent::set_event_handler(Some(move |event| {
        if matches!(
            event,
            TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            }
        ) {
            let _ = sender.try_send(Command::Show);
        }
    }));
    cx.set_global(SystemTray {
        _icon: tray,
        show,
        quit,
    });
    cx.spawn(async move |cx| {
        while let Ok(command) = receiver.recv().await {
            cx.update(|cx| match command {
                Command::Show => crate::app_window::show(window, cx),
                Command::Quit => cx.quit(),
            });
        }
    })
    .detach();
    Ok(())
}
