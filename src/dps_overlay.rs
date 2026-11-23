use crate::{language::text, settings};
use gpui_kit::{
    component::{ActiveTheme as _, StyledExt as _},
    *,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, collections::BTreeMap, fs, io, path::Path, ptr};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::{
        Dwm::{DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DwmSetWindowAttribute},
        Gdi::*,
    },
    UI::{
        Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
        HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
        Shell::{DefSubclassProc, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

const FILE: &str = ".windows_state.json";
const WIDTH: f32 = 232.;
const HEIGHT: f32 = 80.;
const SYNC_FOREGROUND: u32 = WM_APP + 1;

// OUTOFCONTEXT delivers events on the installing UI thread. A closing window
// and its replacement can briefly coexist until GPUI destroys the old HWND.
thread_local! {
    static WATCHED: RefCell<BTreeMap<HWINEVENTHOOK, HWND>> = const { RefCell::new(BTreeMap::new()) };
}

unsafe extern "system" fn foreground_changed(
    hook: HWINEVENTHOOK,
    _: u32,
    _: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    if let Some(hwnd) = WATCHED.with_borrow(|targets| targets.get(&hook).copied()) {
        unsafe { PostMessageW(hwnd, SYNC_FOREGROUND, 0, 0) };
    }
}

fn watch_foreground(hwnd: HWND) -> io::Result<()> {
    unsafe {
        let hook = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            ptr::null_mut(),
            Some(foreground_changed),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        if hook.is_null() {
            return Err(io::Error::last_os_error());
        }
        if SetWindowSubclass(hwnd, Some(overlay_proc), 1, hook as usize) == 0 {
            let err = io::Error::last_os_error();
            UnhookWinEvent(hook);
            return Err(err);
        }
        WATCHED.with_borrow_mut(|targets| targets.insert(hook, hwnd));
    }
    Ok(())
}

pub(super) fn apply_foreground(hwnd: HWND, foreground: HWND) -> io::Result<()> {
    unsafe {
        let show = !foreground.is_null()
            && IsIconic(foreground) == 0
            && crate::game_window::dnf_process(foreground).is_some();
        if (IsWindowVisible(hwnd) != 0) == show {
            return Ok(());
        }
        let (order, visibility) = if show {
            (HWND_TOPMOST, SWP_SHOWWINDOW)
        } else {
            (ptr::null_mut(), SWP_HIDEWINDOW | SWP_NOZORDER)
        };
        if SetWindowPos(
            hwnd,
            order,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | visibility,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub enabled: bool,
    positions: BTreeMap<String, [f32; 2]>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            enabled: true,
            positions: BTreeMap::new(),
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct WindowStates {
    #[serde(default)]
    dps_overlay: Preferences,
    #[serde(flatten)]
    other_windows: BTreeMap<String, serde_json::Value>,
}

impl WindowStates {
    fn load(path: &Path) -> io::Result<Self> {
        match fs::read(path) {
            Ok(bytes) => return Ok(serde_json::from_slice(&bytes)?),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
        let dps_overlay = match fs::read(path.with_file_name("dps-overlay.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(err) if err.kind() == io::ErrorKind::NotFound => Preferences::default(),
            Err(err) => return Err(err),
        };
        let states = Self {
            dps_overlay,
            ..Self::default()
        };
        settings::save(path, &states)?;
        Ok(states)
    }
}

#[cfg(test)]
mod tests {
    use super::{FILE, WindowStates};
    use std::fs;

    #[test]
    fn imports_previous_overlay_file_only_when_window_states_are_missing() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("dps-overlay.json");
        let path = dir.path().join(FILE);
        fs::write(
            &old,
            br#"{"enabled":false,"positions":{"display":[32,48]}}"#,
        )
        .unwrap();
        let imported = WindowStates::load(&path).unwrap();
        assert!(!imported.dps_overlay.enabled);
        assert_eq!(imported.dps_overlay.positions["display"], [32., 48.]);
        fs::write(old, b"invalid").unwrap();
        assert!(!WindowStates::load(&path).unwrap().dps_overlay.enabled);
        fs::write(&path, b"invalid").unwrap();
        assert!(WindowStates::load(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"invalid");
    }
}

impl Preferences {
    pub fn load() -> io::Result<Self> {
        Ok(WindowStates::load(&settings::path(FILE)?)?.dps_overlay)
    }

    fn update(edit: impl FnOnce(&mut Self)) -> io::Result<()> {
        let path = settings::path(FILE)?;
        let mut states = WindowStates::load(&path)?;
        edit(&mut states.dps_overlay);
        settings::save(&path, &states)
    }

    pub fn set_enabled(enabled: bool) -> io::Result<()> {
        Self::update(|prefs| prefs.enabled = enabled)
    }
}

struct Screen {
    name: String,
    work: RECT,
    scale: f32,
}

impl Screen {
    fn from_monitor(monitor: HMONITOR) -> io::Result<Self> {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        if unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let end = info
            .szDevice
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(info.szDevice.len());
        let (mut x, mut y) = (96, 96);
        let dpi = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) };
        Ok(Self {
            name: String::from_utf16_lossy(&info.szDevice[..end]),
            work: info.monitorInfo.rcWork,
            scale: if dpi >= 0 { x as f32 / 96. } else { 1. },
        })
    }

    fn placement(&self, prefs: &Preferences) -> [i32; 4] {
        let [x, y] = prefs
            .positions
            .get(&self.name)
            .copied()
            .unwrap_or([16., 16.]);
        let pixels = |v: f32| (v * self.scale).round() as i32;
        let w = pixels(WIDTH).min(self.work.right - self.work.left);
        let h = pixels(HEIGHT).min(self.work.bottom - self.work.top);
        [
            self.work.left + pixels(x).clamp(0, self.work.right - self.work.left - w),
            self.work.top + pixels(y).clamp(0, self.work.bottom - self.work.top - h),
            w,
            h,
        ]
    }
}

fn save_position(hwnd: HWND) -> io::Result<()> {
    let screen =
        Screen::from_monitor(unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) })?;
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let offset = [
        (rect.left - screen.work.left) as f32 / screen.scale,
        (rect.top - screen.work.top) as f32 / screen.scale,
    ];
    Preferences::update(|prefs| {
        prefs.positions.insert(screen.name, offset);
    })
}

unsafe extern "system" fn overlay_proc(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    _: usize,
    hook: usize,
) -> LRESULT {
    match msg {
        SYNC_FOREGROUND => {
            // A queued Alt+Tab event can already be stale.
            if let Err(err) = apply_foreground(hwnd, unsafe { GetForegroundWindow() }) {
                eprintln!("Updating DPS overlay visibility: {err}");
            }
            return 0;
        }
        WM_NCDESTROY => {
            let hook = hook as HWINEVENTHOOK;
            WATCHED.with_borrow_mut(|targets| targets.remove(&hook));
            unsafe {
                UnhookWinEvent(hook);
            }
        }
        WM_NCHITTEST => return HTCAPTION as LRESULT,
        WM_MOUSEACTIVATE => return MA_NOACTIVATE as LRESULT,
        WM_NCLBUTTONDBLCLK => return 0,
        WM_EXITSIZEMOVE => {
            if let Err(err) = save_position(hwnd) {
                eprintln!("Saving DPS overlay position: {err}");
            }
        }
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, msg, w, l) }
}

fn hwnd(window: &Window) -> io::Result<HWND> {
    let handle =
        HasWindowHandle::window_handle(window).map_err(|err| io::Error::other(err.to_string()))?;
    match handle.as_raw() {
        RawWindowHandle::Win32(raw) => Ok(raw.hwnd.get() as HWND),
        _ => Err(io::Error::other("Expected a Windows window")),
    }
}

struct Meter;

impl Render for Meter {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .justify_center()
            .px_4()
            .gap_1()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(div().text_sm().font_semibold().child(text("nav.dps")))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(text("dps.empty")),
            )
    }
}

pub fn open(main: &Window, cx: &mut App) -> Result<AnyWindowHandle> {
    let prefs = Preferences::load()?;
    let screen =
        Screen::from_monitor(unsafe { MonitorFromWindow(hwnd(main)?, MONITOR_DEFAULTTONEAREST) })?;
    let [x, y, w, h] = screen.placement(&prefs);
    let mut native = Err(io::Error::other("No DPS window handle"));
    let handle = cx.open_window(
        WindowOptions {
            titlebar: None,
            kind: WindowKind::PopUp,
            focus: false,
            show: false,
            is_resizable: false,
            is_minimizable: false,
            window_decorations: Some(WindowDecorations::Client),
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(0.), px(0.)),
                size(px(WIDTH), px(HEIGHT)),
            ))),
            ..Default::default()
        },
        |window, cx| {
            window.set_window_title("Noia DNF · DPS");
            native = hwnd(window);
            cx.new(|_| Meter)
        },
    )?;
    let shown = native.and_then(|hwnd| unsafe {
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        SetWindowLongW(
            hwnd,
            GWL_STYLE,
            ((style & !(WS_CAPTION | WS_THICKFRAME)) | WS_POPUP) as i32,
        );
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        SetWindowLongW(hwnd, GWL_EXSTYLE, (ex | WS_EX_NOACTIVATE) as i32);
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR as u32,
            (&DWMWA_COLOR_NONE as *const u32).cast(),
            size_of::<u32>() as u32,
        );
        watch_foreground(hwnd)?;
        if SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            x,
            y,
            w,
            h,
            SWP_NOACTIVATE | SWP_FRAMECHANGED,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        apply_foreground(hwnd, GetForegroundWindow())
    });
    if let Err(error) = shown {
        handle.update(cx, |_, window, _| window.remove_window())?;
        return Err(error.into());
    }
    Ok(handle.into())
}
