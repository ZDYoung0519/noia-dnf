use gpui_kit::{AnyWindowHandle, App};

/// Shared activation path for tray clicks and subsequent process launches.
pub fn show(window: AnyWindowHandle, cx: &mut App) {
    if let Err(error) = window.update(cx, |_, window, _| window.activate_window()) {
        eprintln!("Failed to activate the main window: {error:#}");
    }
    cx.activate(true);
}
