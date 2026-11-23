use super::AppView;
use gpui_kit::{AnyWindowHandle, App, AppContext, Entity, WindowOptions};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{process::Command, time::Duration};
use windows_sys::Win32::{
    Foundation::{HWND, RECT},
    Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromWindow},
    UI::{Accessibility::NotifyWinEvent, WindowsAndMessaging::*},
};

fn native_handle(window: AnyWindowHandle, cx: &mut App) -> HWND {
    window
        .update(cx, |_, window, _| {
            let RawWindowHandle::Win32(raw) =
                HasWindowHandle::window_handle(window).unwrap().as_raw()
            else {
                unreachable!()
            };
            raw.hwnd.get() as HWND
        })
        .unwrap()
}

#[test]
#[ignore = "Creates a native DPS window on an interactive Windows desktop"]
fn dps_window_lifecycle() {
    // Settings are next to the executable. Give the test its own copy and profile.
    const CHILD: &str = "NOIA_DPS_WINDOW_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("DNF.exe");
        std::fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();
        for phase in ["create", "restart"] {
            assert!(
                Command::new(&exe)
                    .env(CHILD, phase)
                    .args([
                        "--ignored",
                        "--exact",
                        "navigation::tests::dps_window_lifecycle",
                        "--nocapture"
                    ])
                    .status()
                    .unwrap()
                    .success()
            );
            let bytes = std::fs::read(dir.path().join("settings/.windows_state.json")).unwrap();
            let state: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(state["dps_overlay"]["enabled"], false);
            assert!(
                !state["dps_overlay"]["positions"]
                    .as_object()
                    .unwrap()
                    .is_empty()
            );
        }
        return;
    }
    let passed = std::rc::Rc::new(std::cell::Cell::new(false));
    let finished = passed.clone();
    gpui_kit::application()
        .with_assets(crate::assets::AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            let foreground = unsafe { GetForegroundWindow() };
            let (main, view) = gpui_kit::open_window(
                WindowOptions {
                    show: false,
                    focus: false,
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| AppView::new(window, cx)),
            )
            .unwrap();
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                if std::env::var(CHILD).as_deref() == Ok("restart") {
                    finished.set(cx.update(|cx| view.read(cx).dps_window.is_none()));
                    cx.update(|cx| cx.quit());
                    return;
                }
                let expected = crate::game_window::dnf_process(foreground).is_some()
                    && unsafe { IsIconic(foreground) == 0 };
                // Perturb the native window, then let a foreground notification
                // reconcile it. This exercises the real hook without taking focus.
                let reconciled = cx.update(|cx| unsafe {
                    let first = native_handle(view.read(cx).dps_window.unwrap(), cx);
                    let initial = (IsWindowVisible(first) != 0) == expected;
                    // Our isolated test executable is named DNF.exe. Use its real
                    // HWND and the desktop HWND to exercise both focus policies
                    // without activating either window on the user's desktop.
                    let game = native_handle(main, cx);
                    let desktop = GetDesktopWindow();
                    let mut changed = true;
                    for (candidate, visible) in [
                        (game, true),
                        (desktop, false),
                        (if expected { desktop } else { game }, !expected),
                    ] {
                        changed &= crate::dps_overlay::apply_foreground(first, candidate).is_ok()
                            && (IsWindowVisible(first) != 0) == visible
                            && GetForegroundWindow() == foreground;
                    }
                    NotifyWinEvent(
                        EVENT_SYSTEM_FOREGROUND,
                        foreground,
                        OBJID_WINDOW,
                        CHILDID_SELF as i32,
                    );
                    initial && changed
                });
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    cx.update(|cx| {
                        let native = |view: &Entity<AppView>, cx: &mut App| -> HWND {
                            native_handle(view.read(cx).dps_window.unwrap(), cx)
                        };
                        let toggle = |enabled, cx: &mut App| {
                            main.update(cx, |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    this.set_dps_open(enabled, true, window, cx)
                                })
                            })
                            .unwrap();
                        };
                        let first = native(&view, cx);
                        unsafe {
                            assert_eq!(GetForegroundWindow(), foreground);
                            assert!(reconciled);
                            assert_eq!(IsWindowVisible(first) != 0, expected);
                            assert_eq!(
                                MonitorFromWindow(first, MONITOR_DEFAULTTONEAREST),
                                MonitorFromWindow(
                                    native_handle(main, cx),
                                    MONITOR_DEFAULTTONEAREST
                                )
                            );
                            assert_eq!(
                                GetWindowLongW(first, GWL_STYLE) as u32
                                    & (WS_CAPTION | WS_THICKFRAME),
                                0
                            );
                            let mut rect = RECT::default();
                            let mut client = RECT::default();
                            assert_ne!(GetWindowRect(first, &mut rect), 0);
                            assert_ne!(GetClientRect(first, &mut client), 0);
                            assert_eq!(
                                (rect.right - rect.left, rect.bottom - rect.top),
                                (client.right, client.bottom)
                            );
                            for (x, y) in [
                                (rect.left + 1, rect.top + 1),
                                ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2),
                                (rect.right - 1, rect.bottom - 1),
                            ] {
                                let pos = ((y as u16 as usize) << 16) | x as u16 as usize;
                                assert_eq!(
                                    SendMessageW(first, WM_NCHITTEST, 0, pos as isize),
                                    HTCAPTION as isize
                                );
                            }
                            let (x, y) = (rect.left + 35, rect.top + 41);
                            assert_ne!(
                                SetWindowPos(
                                    first,
                                    std::ptr::null_mut(),
                                    x,
                                    y,
                                    0,
                                    0,
                                    SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER
                                ),
                                0
                            );
                            SendMessageW(first, WM_EXITSIZEMOVE, 0, 0);
                            toggle(false, cx);
                            assert!(!crate::dps_overlay::Preferences::load().unwrap().enabled);
                            toggle(true, cx);
                            let second = native(&view, cx);
                            toggle(true, cx);
                            assert_eq!(cx.windows().len(), 2);
                            assert_ne!(GetWindowRect(second, &mut rect), 0);
                            assert_eq!((rect.left, rect.top), (x, y));
                            assert_eq!(IsWindowVisible(second) != 0, expected);
                            assert_eq!(GetForegroundWindow(), foreground);
                            toggle(false, cx);
                            (first, second)
                        }
                    })
                }));
                if let Ok((first, second)) = result {
                    cx.background_executor()
                        .timer(Duration::from_millis(150))
                        .await;
                    finished.set(unsafe { IsWindow(first) == 0 && IsWindow(second) == 0 });
                }
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    assert!(passed.get());
}
