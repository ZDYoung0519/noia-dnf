use crate::language::text;
use gpui_kit::{
    component::{
        ActiveTheme as _, Sizable as _, StyledExt as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        progress::ProgressCircle,
        scroll::ScrollableElement as _,
    },
    *,
};
use noia_updater::{Changelog, Installer};
use std::sync::Arc;

pub const VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));

pub struct About {
    bundled: Vec<Changelog>,
    recent: Option<Changelog>,
}

impl About {
    pub fn new() -> Self {
        #[derive(serde::Deserialize)]
        struct History {
            releases: Vec<Changelog>,
        }
        let history: History =
            serde_json::from_str(include_str!("../../change_log.json")).expect("Bundled changelog");
        Self {
            bundled: history
                .releases
                .into_iter()
                .skip_while(|log| log.version != env!("CARGO_PKG_VERSION"))
                .collect(),
            recent: None,
        }
    }
}

impl Render for About {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let locale = gpui_kit::component::locale();
        let mut content = div()
            .flex()
            .flex_col()
            .gap_4()
            .p_8()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .child(img(crate::assets::APP_LOGO).size(px(64.)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().text_2xl().font_semibold().child("Noia DNF"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(VERSION),
                            ),
                    ),
            )
            .child(div().pt_4().font_semibold().child(text("about.changelog")));
        let logs = self.recent.iter().chain(self.bundled.iter().filter(|log| {
            self.recent
                .as_ref()
                .is_none_or(|latest| latest.version != log.version)
        }));
        for log in logs {
            content = content.child(
                div()
                    .text_sm()
                    .font_semibold()
                    .child(format!("v{}", log.version)),
            );
            for item in &log.items {
                if let Some(line) = item.get(&*locale).or_else(|| item.get("zh-CN")) {
                    content = content.child(div().text_sm().child(format!("• {line}")));
                }
            }
        }
        div()
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .child(content)
    }
}

enum State {
    Checking,
    Downloading(u8),
    Current,
    Ready(Arc<Installer>),
    Failed(String),
}

enum Event {
    Notes(Changelog),
    State(State),
}

pub struct Updater {
    state: State,
    about: Entity<About>,
}

impl Updater {
    pub fn new(about: Entity<About>, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            state: State::Current,
            about,
        };
        this.check(cx);
        this
    }

    pub fn check(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.state,
            State::Checking | State::Downloading(_) | State::Ready(_)
        ) {
            return;
        }
        self.state = State::Checking;
        cx.notify();
        // At most one event per integer percentage, plus status transitions.
        let (tx, rx) = async_channel::unbounded();
        cx.background_executor()
            .spawn(async move {
                let result =
                    noia_updater::check(env!("NOIA_RELEASE_URL"), env!("CARGO_PKG_VERSION"))
                        .and_then(|check| {
                            if let Some(notes) = check.changelog {
                                let _ = tx.try_send(Event::Notes(notes));
                            }
                            let Some(release) = check.update else {
                                return Ok(State::Current);
                            };
                            let _ = tx.try_send(Event::State(State::Downloading(0)));
                            let installer = noia_updater::download(release, |percent| {
                                let _ = tx.try_send(Event::State(State::Downloading(percent)));
                            })?;
                            Ok(State::Ready(Arc::new(installer)))
                        });
                let state = result.unwrap_or_else(|error| State::Failed(format!("{error:#}")));
                let _ = tx.try_send(Event::State(state));
            })
            .detach();
        cx.spawn(async move |this, cx| {
            while let Ok(event) = rx.recv().await {
                if this
                    .update(cx, |this, cx| match event {
                        Event::Notes(notes) => this.about.update(cx, |about, cx| {
                            about.recent = Some(notes);
                            cx.notify();
                        }),
                        Event::State(state) => {
                            this.state = state;
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }
}

impl Render for Updater {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ring = ProgressCircle::new("update-progress")
            .small()
            .color(cx.theme().link);
        let tag = Button::new("update-status").xsmall().compact();
        match &self.state {
            State::Checking => ring
                .loading(true)
                .accessibility_label(text("update.checking"))
                .into_any_element(),
            State::Downloading(percent) => div()
                .flex()
                .items_center()
                .gap_1()
                .text_xs()
                .child(
                    ring.value(*percent as f32)
                        .accessibility_label(text("update.downloading")),
                )
                .child(format!("{percent}%"))
                .into_any_element(),
            State::Current => tag
                .outline()
                .label(text("update.current"))
                .tooltip(text("update.check"))
                .on_click(cx.listener(|this, _, _, cx| this.check(cx)))
                .into_any_element(),
            State::Failed(reason) => tag
                .outline()
                .label(text("update.retry"))
                .tooltip(reason.clone())
                .on_click(cx.listener(|this, _, _, cx| this.check(cx)))
                .into_any_element(),
            State::Ready(installer) => {
                let installer = installer.clone();
                tag.info()
                    .label(text("update.ready"))
                    .on_click(move |_, window, cx| {
                        let installer = installer.clone();
                        window.open_alert_dialog(cx, move |dialog, _, _| {
                            let installer = installer.clone();
                            dialog
                                .confirm()
                                .title(text("update.confirm"))
                                .ok_text(text("update.restart"))
                                .cancel_text(text("update.later"))
                                .on_ok(move |_, window, cx| {
                                    match noia_updater::launch(&installer) {
                                        Ok(()) => cx.quit(),
                                        Err(error) => window.push_notification(
                                            format!("{} {error}", text("update.launch_error")),
                                            cx,
                                        ),
                                    }
                                    true
                                })
                        });
                    })
                    .into_any_element()
            }
        }
    }
}
