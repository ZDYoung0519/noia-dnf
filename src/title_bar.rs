use crate::language::text;
use gpui_kit::{
    component::{
        ActiveTheme as _, Icon, Sizable as _, StyledExt as _, TitleBar, WindowExt as _,
        button::{Button, ButtonVariants as _},
    },
    *,
};

pub const HEIGHT: Pixels = px(52.0);
pub const SIDEBAR_WIDTH: Pixels = px(224.0);

/// The sidebar owns the top-left branding, aligned with the content title bar.
pub fn brand(cx: &App) -> impl IntoElement + use<> {
    div()
        .id("sidebar-brand")
        .window_control_area(WindowControlArea::Drag)
        .flex()
        .items_center()
        .h(HEIGHT)
        .w_full()
        .flex_shrink_0()
        .px_5()
        .gap_3()
        .border_r_1()
        .border_b_1()
        .border_color(cx.theme().sidebar_border)
        .bg(cx.theme().tokens.sidebar)
        .text_color(cx.theme().sidebar_foreground)
        .text_sm()
        .font_semibold()
        .child(img(crate::assets::APP_LOGO).size(px(24.0)))
        .child("Noia DNF")
}

/// Only spans the content column. Official controls retain Windows hit testing.
pub fn title_bar(cx: &App) -> TitleBar {
    TitleBar::new()
        .h(HEIGHT)
        .bg(cx.theme().tokens.sidebar)
        .border_color(cx.theme().sidebar_border)
        .child(
            div()
                .flex()
                .w_full()
                .h_full()
                .items_center()
                .justify_end()
                .pr_4()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_1()
                        .rounded_full()
                        .bg(cx.theme().tokens.sidebar_accent)
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            div()
                                .size(px(6.0))
                                .rounded_full()
                                .bg(cx.theme().muted_foreground),
                        )
                        .child(text("game.unknown")),
                )
                .child(
                    Button::new("sign-in")
                        .small()
                        .info()
                        .icon(Icon::default().path("icons/lucide/log-in.svg"))
                        .label(text("login.button"))
                        .tooltip(text("login.tooltip"))
                        .on_click(|_, window, cx| {
                            window.push_notification(text("login.pending"), cx);
                        }),
                ),
        )
}
