use crate::language::{self, text};
#[cfg(all(test, windows))]
mod tests;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    component::{
        ActiveTheme as _, Icon, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        sidebar::*,
    },
    *,
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Home,
    RapidFire,
    Dps,
    Character,
    Events,
    Dungeons,
    Rankings,
    Qq,
    WeChat,
    Settings,
    About,
}

impl Page {
    pub fn title(self) -> SharedString {
        text(match self {
            Self::Home => "nav.home",
            Self::RapidFire => "nav.rapid_fire",
            Self::Dps => "nav.dps",
            Self::Character => "nav.character",
            Self::Events => "nav.events",
            Self::Dungeons => "nav.dungeons",
            Self::Rankings => "nav.rankings",
            Self::Qq => "nav.qq",
            Self::WeChat => "nav.wechat",
            Self::Settings => "nav.settings",
            Self::About => "nav.about",
        })
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Home => "icons/lucide/house.svg",
            Self::RapidFire => "icons/lucide/keyboard.svg",
            Self::Dps => "icons/lucide/chart-no-axes-combined.svg",
            Self::Character => "icons/lucide/backpack.svg",
            Self::Events => "icons/lucide/bell.svg",
            Self::Dungeons => "icons/lucide/scroll-text.svg",
            Self::Rankings => "icons/lucide/trophy.svg",
            Self::Qq => "icons/lucide/message-circle.svg",
            Self::WeChat => "icons/lucide/messages-square.svg",
            Self::Settings | Self::About => "icons/lucide/settings-2.svg",
        }
    }

    pub fn description(self) -> SharedString {
        text(match self {
            Self::Home => "home.description",
            Self::Qq => "contact.qq",
            Self::WeChat => "contact.wechat",
            Self::Settings => "settings.language",
            _ => "page.pending",
        })
    }
}

pub const IN_GAME: [Page; 4] = [Page::RapidFire, Page::Dps, Page::Character, Page::Events];
pub const RECORDS: [Page; 2] = [Page::Dungeons, Page::Rankings];

pub struct AppView {
    selected: Page,
    keyboard: Entity<crate::keyboard::Keyboard>,
    #[cfg(windows)]
    dps_window: Option<AnyWindowHandle>,
    #[cfg(windows)]
    updater: Entity<crate::updater::Updater>,
    #[cfg(windows)]
    about: Entity<crate::updater::About>,
}

impl AppView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        #[cfg(windows)]
        {
            let view = cx.weak_entity();
            cx.on_window_closed(move |cx, id| {
                let view = view.clone();
                cx.defer(move |cx| {
                    let _ = view.update(cx, |this, cx| {
                        if this.dps_window.is_some_and(|w| w.window_id() == id) {
                            this.dps_window = None;
                            cx.notify();
                        }
                    });
                });
            })
            .detach();
            cx.defer_in(
                window,
                |this, window, cx| match crate::dps_overlay::Preferences::load() {
                    Ok(prefs) if prefs.enabled => this.set_dps_open(true, false, window, cx),
                    Ok(_) => {}
                    Err(err) => Self::dps_error(err, window, cx),
                },
            );
        }
        #[cfg(windows)]
        let about = cx.new(|_| crate::updater::About::new());
        Self {
            selected: Page::Home,
            keyboard: cx.new(|cx| crate::keyboard::Keyboard::new(window, cx)),
            #[cfg(windows)]
            dps_window: None,
            #[cfg(windows)]
            updater: cx.new(|cx| crate::updater::Updater::new(about.clone(), cx)),
            #[cfg(windows)]
            about,
        }
    }

    #[cfg(windows)]
    fn dps_error(err: impl std::fmt::Display, window: &mut Window, cx: &mut App) {
        use gpui_kit::component::WindowExt as _;
        window.push_notification(format!("{} {err}", text("dps.error")), cx);
    }

    #[cfg(windows)]
    fn set_dps_open(
        &mut self,
        open: bool,
        save: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if open == self.dps_window.is_some() {
            return;
        }
        if open {
            match crate::dps_overlay::open(window, cx) {
                Ok(handle) => self.dps_window = Some(handle),
                Err(err) => {
                    Self::dps_error(err, window, cx);
                    return;
                }
            }
        } else if let Some(handle) = self.dps_window.take() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
        if save && let Err(err) = crate::dps_overlay::Preferences::set_enabled(open) {
            Self::dps_error(err, window, cx);
        }
        cx.notify();
    }
    fn menu_item(&self, page: Page, cx: &Context<Self>) -> SidebarMenuItem {
        let active = self.selected == page;
        SidebarMenuItem::new(page.title())
            .icon(
                Icon::default()
                    .path(page.icon())
                    .size(px(18.0))
                    .text_color(if active {
                        cx.theme().link
                    } else {
                        cx.theme().muted_foreground
                    }),
            )
            .active(active)
            .border_l_2()
            .border_color(if active {
                cx.theme().link
            } else {
                transparent_black()
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = page;
                cx.notify();
            }))
    }

    fn sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let home = SidebarMenu::new().child(self.menu_item(Page::Home, cx));
        let footer_menu = SidebarMenu::new().children(
            [Page::Qq, Page::WeChat, Page::Settings].map(|page| self.menu_item(page, cx)),
        );
        Sidebar::new("main-sidebar")
            .w(crate::title_bar::SIDEBAR_WIDTH)
            .collapsible(false)
            .header(home.render("home-navigation", window, cx))
            .child(
                SidebarGroup::new(text("nav.in_game")).child(
                    SidebarMenu::new().children(IN_GAME.map(|page| self.menu_item(page, cx))),
                ),
            )
            .child(
                SidebarGroup::new(text("nav.records")).child(
                    SidebarMenu::new().children(RECORDS.map(|page| self.menu_item(page, cx))),
                ),
            )
            .footer(
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .gap_2()
                    .pt_3()
                    .border_t_1()
                    .border_color(cx.theme().sidebar_border)
                    .child(
                        div()
                            .px_2()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(text("nav.contact")),
                    )
                    .child(footer_menu.render("footer-navigation", window, cx))
                    .child(
                        div()
                            .px_2()
                            .pt_2()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                Button::new("about-version")
                                    .ghost()
                                    .xsmall()
                                    .compact()
                                    .label(concat!("v", env!("CARGO_PKG_VERSION")))
                                    .tooltip(text("nav.about"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.selected = Page::About;
                                        cx.notify();
                                    })),
                            )
                            .map(|row| {
                                #[cfg(windows)]
                                let row = row.child(self.updater.clone());
                                row
                            }),
                    ),
            )
    }
}

pub fn page_header(page: Page, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .flex_shrink_0()
        .px_8()
        .py_5()
        .border_b_1()
        .border_color(cx.theme().border)
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(div().flex_shrink_0().child(page.title()))
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = self.selected;
        div()
            .flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(crate::title_bar::SIDEBAR_WIDTH)
                    .h_full()
                    .flex_shrink_0()
                    .bg(cx.theme().tokens.sidebar)
                    .child(crate::title_bar::brand(cx))
                    .child(div().flex_1().min_h_0().child(self.sidebar(window, cx))),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(crate::title_bar::title_bar(cx))
                    .map(|column| {
                        #[cfg(windows)]
                        if page == Page::Dps {
                            let switch =
                                gpui_kit::component::switch::Switch::new("dps-overlay-toggle")
                                    .label(text("dps.overlay"))
                                    .checked(self.dps_window.is_some())
                                    .on_change(cx.listener(|this, checked, window, cx| {
                                        this.set_dps_open(*checked, true, window, cx);
                                    }));
                            return column.child(page_header(page, cx).child(switch));
                        }
                        #[cfg(windows)]
                        if page == Page::About {
                            return column
                                .child(page_header(page, cx))
                                .child(self.about.clone());
                        }
                        if page == Page::RapidFire {
                            column.child(self.keyboard.clone())
                        } else {
                            column.child(page_header(page, cx)).child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .items_center()
                                    .justify_center()
                                    .gap_4()
                                    .p_8()
                                    .child(img(crate::assets::APP_LOGO).size(px(72.0)))
                                    .child(div().text_2xl().font_semibold().child(
                                        if page == Page::Home {
                                            text("home.welcome")
                                        } else {
                                            page.title()
                                        },
                                    ))
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(page.description()),
                                    )
                                    .when(page == Page::Settings, |view| {
                                        let view = view.child(language::picker());
                                        #[cfg(windows)]
                                        let view = view.child(
                                            Button::new("check-updates")
                                                .small()
                                                .label(text("update.check"))
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.updater
                                                        .update(cx, |updater, cx| updater.check(cx))
                                                })),
                                        );
                                        view
                                    }),
                            )
                        }
                    }),
            )
    }
}
