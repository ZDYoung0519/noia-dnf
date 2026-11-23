use gpui_kit::{
    component::{
        WindowExt as _,
        button::{Button, ButtonVariants as _},
    },
    *,
};
use rust_i18n::t;
use std::io;

pub const OPTIONS: [(&str, &str); 2] = [("zh-CN", "简体中文"), ("en", "English")];
const DEFAULT: &str = "zh-CN";

pub fn text(key: &str) -> SharedString {
    t!(key).into_owned().into()
}

fn supported(code: &str) -> Option<&'static str> {
    OPTIONS
        .iter()
        .find(|(id, _)| *id == code.trim())
        .map(|(id, _)| *id)
}

pub fn init() {
    let saved = crate::settings::path("language.json")
        .and_then(|path| {
            crate::settings::read_or_create(&path, || {
                let code = match crate::settings::legacy("language") {
                    Ok(bytes) => supported(&String::from_utf8_lossy(&bytes)).unwrap_or(DEFAULT),
                    Err(err) if err.kind() == io::ErrorKind::NotFound => DEFAULT,
                    Err(err) => return Err(err),
                };
                Ok(code.to_owned())
            })
        })
        .unwrap_or_else(|err| {
            eprintln!("Reading language preference: {err}");
            DEFAULT.to_owned()
        });
    // GPUI Component uses the same rust-i18n locale as our application catalog.
    gpui_kit::component::set_locale(supported(&saved).unwrap_or(DEFAULT));
}

pub fn select(code: &str, cx: &mut App) -> io::Result<()> {
    let code = supported(code).ok_or_else(|| io::Error::other("Unsupported language"))?;
    crate::settings::save(&crate::settings::path("language.json")?, &code)?;
    gpui_kit::component::set_locale(code);
    #[cfg(windows)]
    crate::tray::refresh_labels(cx);
    cx.refresh_windows();
    Ok(())
}

pub fn picker() -> impl IntoElement {
    div().flex().gap_2().children(OPTIONS.map(|(code, name)| {
        let current = &*gpui_kit::component::locale() == code;
        let button = Button::new(code).label(name).toggled(current);
        let button = if current { button.primary() } else { button };
        button.on_click(move |_, window, cx| {
            if let Err(error) = select(code, cx) {
                eprintln!("Failed to save language: {error}");
                window.push_notification(text("settings.save_error"), cx);
            }
        })
    }))
}
