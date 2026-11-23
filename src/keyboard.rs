use crate::language::text;
use crate::repeat::{MAX_INTERVAL_MS, MIN_INTERVAL_MS, Settings};
use crate::repeat::{Stats, Worker};
use gpui_kit::{
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        input::{InputEvent, InputState, NumberInput},
        scroll::ScrollableElement as _,
        slider::{Slider, SliderEvent, SliderState},
        switch::Switch,
    },
    *,
};
use std::sync::{Arc, atomic::Ordering};

// Positions and sizes are in standard key widths. Each run occupies one row
// within a keyboard cluster. A tall key spans the following row as well.
struct Key(&'static str, u16, f32, f32);
const fn key(label: &'static str, scan: u16) -> Key {
    Key(label, scan, 1.0, 1.0)
}
const fn wide(label: &'static str, scan: u16, width: f32) -> Key {
    Key(label, scan, width, 1.0)
}

const ROWS: &[(f32, f32, &[Key])] = &[
    (0.0, 0.0, &[key("Esc", 0x01)]),
    (
        2.0,
        0.0,
        &[
            key("F1", 0x3b),
            key("F2", 0x3c),
            key("F3", 0x3d),
            key("F4", 0x3e),
        ],
    ),
    (
        6.5,
        0.0,
        &[
            key("F5", 0x3f),
            key("F6", 0x40),
            key("F7", 0x41),
            key("F8", 0x42),
        ],
    ),
    (
        11.0,
        0.0,
        &[
            key("F9", 0x43),
            key("F10", 0x44),
            key("F11", 0x57),
            key("F12", 0x58),
        ],
    ),
    (
        15.5,
        0.0,
        &[key("PrtSc", 0x0), key("ScrLk", 0x0), key("Pause", 0x0)],
    ),
    (
        0.0,
        1.5,
        &[
            key("`", 0x29),
            key("1", 0x02),
            key("2", 0x03),
            key("3", 0x04),
            key("4", 0x05),
            key("5", 0x06),
            key("6", 0x07),
            key("7", 0x08),
            key("8", 0x09),
            key("9", 0x0a),
            key("0", 0x0b),
            key("-", 0x0c),
            key("=", 0x0d),
            wide("Backspace", 0x0e, 2.0),
        ],
    ),
    (
        15.5,
        1.5,
        &[key("Ins", 0x152), key("Home", 0x147), key("PgUp", 0x149)],
    ),
    (
        19.0,
        1.5,
        &[
            key("Num", 0x0),
            key("/", 0x135),
            key("*", 0x37),
            key("-", 0x4a),
        ],
    ),
    (
        0.0,
        2.5,
        &[
            wide("Tab", 0x0f, 1.5),
            key("Q", 0x10),
            key("W", 0x11),
            key("E", 0x12),
            key("R", 0x13),
            key("T", 0x14),
            key("Y", 0x15),
            key("U", 0x16),
            key("I", 0x17),
            key("O", 0x18),
            key("P", 0x19),
            key("[", 0x1a),
            key("]", 0x1b),
            wide("\\", 0x2b, 1.5),
        ],
    ),
    (
        15.5,
        2.5,
        &[key("Del", 0x153), key("End", 0x14f), key("PgDn", 0x151)],
    ),
    (
        19.0,
        2.5,
        &[
            key("7", 0x47),
            key("8", 0x48),
            key("9", 0x49),
            Key("+", 0x4e, 1.0, 2.0),
        ],
    ),
    (
        0.0,
        3.5,
        &[
            wide("Caps Lock", 0x0, 1.75),
            key("A", 0x1e),
            key("S", 0x1f),
            key("D", 0x20),
            key("F", 0x21),
            key("G", 0x22),
            key("H", 0x23),
            key("J", 0x24),
            key("K", 0x25),
            key("L", 0x26),
            key(";", 0x27),
            key("'", 0x28),
            wide("Enter", 0x1c, 2.25),
        ],
    ),
    (19.0, 3.5, &[key("4", 0x4b), key("5", 0x4c), key("6", 0x4d)]),
    (
        0.0,
        4.5,
        &[
            wide("Shift", 0x0, 2.25),
            key("Z", 0x2c),
            key("X", 0x2d),
            key("C", 0x2e),
            key("V", 0x2f),
            key("B", 0x30),
            key("N", 0x31),
            key("M", 0x32),
            key(",", 0x33),
            key(".", 0x34),
            key("/", 0x35),
            wide("Shift", 0x0, 2.75),
        ],
    ),
    (16.5, 4.5, &[key("↑", 0x148)]),
    (
        19.0,
        4.5,
        &[
            key("1", 0x4f),
            key("2", 0x50),
            key("3", 0x51),
            Key("Enter", 0x11c, 1.0, 2.0),
        ],
    ),
    (
        0.0,
        5.5,
        &[
            wide("Ctrl", 0x0, 1.25),
            wide("Win", 0x0, 1.25),
            wide("Alt", 0x0, 1.25),
            wide("Space", 0x39, 6.25),
            wide("Alt", 0x0, 1.25),
            wide("Win", 0x0, 1.25),
            wide("Menu", 0x0, 1.25),
            wide("Ctrl", 0x0, 1.25),
        ],
    ),
    (
        15.5,
        5.5,
        &[key("←", 0x14b), key("↓", 0x150), key("→", 0x14d)],
    ),
    (19.0, 5.5, &[wide("0", 0x52, 2.0), key(".", 0x53)]),
];

pub fn selectable(scan: u16) -> bool {
    scan != 0
        && ROWS
            .iter()
            .any(|(_, _, keys)| keys.iter().any(|k| k.1 == scan))
}

pub struct Keyboard {
    slider: Entity<SliderState>,
    interval_input: Entity<InputState>,
    config: Settings,
    worker: Option<Worker>,
    stats: Arc<Stats>,
    load_failed: bool,
}

impl Keyboard {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (tx, rx) = async_channel::bounded(1);
        cx.spawn(async move |this, cx| {
            while rx.recv().await.is_ok() {
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(250))
                    .await;
            }
        })
        .detach();
        let loaded = Settings::load();
        let load_failed = loaded.is_err();
        let config = loaded.unwrap_or_default();
        let slider = cx.new(|_| {
            SliderState::new()
                .min(MIN_INTERVAL_MS as f32)
                .max(MAX_INTERVAL_MS as f32)
                .step(1.)
                .default_value(config.interval_ms as f32)
        });
        let interval_input = cx.new(|cx| {
            InputState::new(window, cx)
                .min(MIN_INTERVAL_MS as f64)
                .max(MAX_INTERVAL_MS as f64)
                .step(1.)
                .validate(|s, _| s.len() <= 10 && s.bytes().all(|b| b.is_ascii_digit()))
                .default_value(config.interval_ms.to_string())
        });
        cx.subscribe_in(&slider, window, |this, _, event, window, cx| match event {
            SliderEvent::Change(value) => this.display_interval(value.end() as u64, window, cx),
            SliderEvent::Release(value) => this.commit_interval(value.end() as u64, window, cx),
        })
        .detach();
        cx.subscribe_in(
            &interval_input,
            window,
            |this, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let ms = input
                        .read(cx)
                        .value()
                        .parse()
                        .unwrap_or(this.config.interval_ms);
                    this.commit_interval(ms, window, cx);
                }
            },
        )
        .detach();
        let mut this = Self {
            config,
            slider,
            interval_input,
            worker: None,
            stats: Arc::new(Stats::new(tx)),
            load_failed,
        };
        this.restart();
        this
    }

    fn display_interval(&self, ms: u64, window: &mut Window, cx: &mut Context<Self>) {
        let value = ms.to_string();
        if self.interval_input.read(cx).value().as_ref() != value.as_str() {
            self.interval_input
                .update(cx, |input, cx| input.set_value(value, window, cx));
        }
    }

    fn commit_interval(&mut self, ms: u64, window: &mut Window, cx: &mut Context<Self>) {
        let ms = ms.clamp(MIN_INTERVAL_MS, MAX_INTERVAL_MS);
        if self.config.interval_ms != ms {
            self.change(window, cx, |config| config.interval_ms = ms);
        }
        let saved = self.config.interval_ms;
        self.display_interval(saved, window, cx);
        self.slider
            .update(cx, |slider, cx| slider.set_value(saved as f32, window, cx));
    }

    fn controls(&self, window: &Window, cx: &Context<Self>) -> Div {
        let slider_width =
            (window.viewport_size().width - crate::title_bar::SIDEBAR_WIDTH - px(410.))
                .clamp(px(58.), px(180.));
        div()
            .flex()
            .items_center()
            .justify_end()
            .gap_3()
            .child(text("repeat.interval"))
            .child(
                div()
                    .w(slider_width)
                    .child(Slider::new(&self.slider).accessibility_label(text("repeat.interval"))),
            )
            .child(
                div().w(px(83.)).child(
                    NumberInput::new(&self.interval_input)
                        .small()
                        .stepper(false)
                        .accessibility_label(text("repeat.interval"))
                        .suffix(div().text_xs().child("ms")),
                ),
            )
            .child(
                Switch::new("enable-repeat")
                    .small()
                    .checked(self.config.enabled)
                    .label(text(if self.config.enabled {
                        "repeat.on"
                    } else {
                        "repeat.off"
                    }))
                    .on_change(cx.listener(|this, checked, window, cx| {
                        this.change(window, cx, |config| config.enabled = *checked);
                    })),
            )
    }

    fn restart(&mut self) {
        self.worker.take();
        self.stats.higher_integrity.store(false, Ordering::Relaxed);
        self.stats.failed.store(false, Ordering::Relaxed);
        if self.config.enabled && !self.config.keys.is_empty() {
            match Worker::start(self.config.clone(), self.stats.clone()) {
                Ok(worker) => self.worker = Some(worker),
                Err(error) => {
                    self.stats.failed.store(true, Ordering::Relaxed);
                    eprintln!("Key repeat unavailable: {error}");
                }
            }
        }
    }

    fn change(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut Settings),
    ) {
        let mut config = self.config.clone();
        edit(&mut config);
        if let Err(error) = config.save() {
            eprintln!("Saving key repeat settings: {error}");
            window.push_notification(text("repeat.save_error"), cx);
            // Always honor an explicit stop for this session.
            if self.config.enabled && !config.enabled {
                self.config.enabled = false;
                self.restart();
            }
        } else {
            self.load_failed = false;
            self.config = config;
            self.restart();
        }
        cx.notify();
    }
}

impl Render for Keyboard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let available = window.viewport_size().width - crate::title_bar::SIDEBAR_WIDTH - px(64.);
        let unit = (available / 23.).clamp(px(40.), px(54.));
        let mut board = div().relative().flex_shrink_0().w(unit * 23.).h(unit * 6.5);
        for (row, &(start, top, keys)) in ROWS.iter().enumerate() {
            let mut left = start;
            for (column, &Key(label, scan, width, height)) in keys.iter().enumerate() {
                let active = self.config.keys.contains(&scan);
                let button = Button::new(("key", row * 32 + column))
                    .xsmall()
                    .compact()
                    .disabled(scan == 0)
                    .label(label)
                    .toggled(active);
                let button = if active {
                    button.info()
                } else {
                    button.outline()
                };
                board = board.child(
                    div().absolute().left(unit * left).top(unit * top).child(
                        button
                            .w(unit * width - px(4.))
                            .h(unit * height - px(4.))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.change(window, cx, |config| {
                                    if !config.keys.remove(&scan) {
                                        config.keys.insert(scan);
                                    }
                                });
                            })),
                    ),
                );
                left += width;
            }
        }
        let content = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .p_6()
            .min_w_0()
            .child(
                div()
                    .flex()
                    .flex_1()
                    .items_start()
                    .justify_between()
                    .gap_3()
                    .child(div().text_sm().child(text("keyboard.hint")))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                rust_i18n::t!("keyboard.count", count = self.config.keys.len())
                                    .into_owned(),
                            ),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .flex_shrink_0()
                    .child(board)
                    .overflow_x_scrollbar(),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .items_end()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(if self.load_failed {
                        text("repeat.load_error")
                    } else if self.stats.higher_integrity.load(Ordering::Relaxed) {
                        text("repeat.higher_integrity")
                    } else if self.stats.failed.load(Ordering::Relaxed) {
                        text("repeat.failed")
                    } else {
                        rust_i18n::t!(
                            "repeat.sent",
                            count = self.stats.sent.load(Ordering::Relaxed)
                        )
                        .into_owned()
                        .into()
                    }),
            );
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(
                crate::navigation::page_header(crate::navigation::Page::RapidFire, cx)
                    .flex_wrap()
                    .child(self.controls(window, cx)),
            )
            .child(content)
    }
}
