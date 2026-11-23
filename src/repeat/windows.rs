use super::{Settings, WorkerStatus};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr::{null, null_mut},
    sync::{Arc, atomic::Ordering, mpsc},
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{ERROR_INVALID_PARAMETER, HWND, LPARAM, LRESULT, WAIT_FAILED, WPARAM},
    System::{LibraryLoader::GetModuleHandleW, Threading::*},
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

const EDGE: u32 = WM_APP + 1;
// Match the reference client's vkFF + physical scan code for repeat pulses.
// KEYEVENTF_SCANCODE would make Windows ignore this virtual key.
const REPEAT_VK: u16 = 0xff;

pub struct Worker {
    id: u32,
    thread: Option<thread::JoinHandle<()>>,
}

impl Worker {
    pub fn start(config: Settings, status: Arc<WorkerStatus>) -> io::Result<Self> {
        let (ready, started) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("noia-key-repeat".into())
            .spawn(move || {
                if let Err(err) = run(config, &status, &ready) {
                    status.report_failure();
                    let _ = ready.send(Err(io::Error::other(err.to_string())));
                }
            })?;
        let id = started.recv().map_err(io::Error::other)??;
        Ok(Self {
            id,
            thread: Some(thread),
        })
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        unsafe {
            PostThreadMessageW(self.id, WM_QUIT, 0, 0);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Default)]
struct Physical {
    selected: BTreeSet<u16>,
    down: BTreeSet<u16>,
}
thread_local! { static PHYSICAL: RefCell<Physical> = RefCell::default(); }

fn hardware_key(event: &KBDLLHOOKSTRUCT) -> Option<u16> {
    if event.flags & LLKHF_INJECTED != 0 || event.scanCode > 0x7f {
        return None;
    }
    Some(
        event.scanCode as u16
            | if event.flags & LLKHF_EXTENDED != 0 {
                0x100
            } else {
                0
            },
    )
}

unsafe extern "system" fn key_hook(code: i32, msg: WPARAM, data: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // Windows owns this structure for the duration of the hook callback.
        let event = unsafe { &*(data as *const KBDLLHOOKSTRUCT) };
        if let Some(key) = hardware_key(event) {
            PHYSICAL.with_borrow_mut(|physical| {
                if !physical.selected.contains(&key) {
                    return;
                }
                if event.flags & LLKHF_UP != 0 {
                    if physical.down.remove(&key) {
                        unsafe {
                            PostThreadMessageW(GetCurrentThreadId(), EDGE, key as usize, 0);
                        }
                    }
                } else if physical.down.insert(key) {
                    unsafe {
                        PostThreadMessageW(
                            GetCurrentThreadId(),
                            EDGE,
                            key as usize,
                            GetForegroundWindow() as isize,
                        );
                    }
                }
            });
        }
    }
    // Let Windows handle the original key, including its normal typing repeat.
    unsafe { CallNextHookEx(null_mut(), code, msg, data) }
}

fn is_game(hwnd: HWND, own_integrity: Option<u32>, status: &WorkerStatus) -> bool {
    let Some(process) = crate::game_window::dnf_process(hwnd) else {
        return false;
    };
    let blocked = own_integrity
        .zip(crate::privileges::integrity(process.as_raw_handle()).ok())
        .is_some_and(|(own, target)| target > own);
    if status.higher_integrity.swap(blocked, Ordering::Relaxed) != blocked {
        let _ = status.changed.try_send(());
    }
    !blocked
}

fn repeat_input(key: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: REPEAT_VK,
                wScan: key & 0xff,
                dwFlags: if key & 0x100 != 0 {
                    KEYEVENTF_EXTENDEDKEY
                } else {
                    0
                } | if up { KEYEVENTF_KEYUP } else { 0 },
                ..Default::default()
            },
        },
    }
}

// The physical key passes through. Track the extra pulse independently so that
// even a physical key-up halfway through a pulse closes its synthetic down.
struct Hold {
    hwnd: HWND,
    due: Instant,
    synthetic_down: bool,
}

impl Hold {
    fn tick(
        &mut self,
        now: Instant,
        half_period: Duration,
        foreground: HWND,
        down: bool,
        mut emit: impl FnMut(bool) -> bool,
        status: &WorkerStatus,
    ) -> bool {
        if !down || foreground != self.hwnd {
            return false;
        }
        if now < self.due {
            return true;
        }
        if !emit(self.synthetic_down) {
            status.report_failure();
            return false;
        }
        self.synthetic_down = !self.synthetic_down;
        self.due = if self.due + half_period > now {
            self.due + half_period
        } else {
            now + half_period
        };
        true
    }

    fn finish(&self, key: u16, status: &WorkerStatus) {
        if self.synthetic_down && !send(key, true) {
            status.report_failure();
        }
    }
}

fn send(key: u16, up: bool) -> bool {
    let event = repeat_input(key, up);
    unsafe { SendInput(1, &event, size_of::<INPUT>() as i32) == 1 }
}

fn repeat_timer() -> io::Result<OwnedHandle> {
    let create = |flags| unsafe {
        CreateWaitableTimerExW(
            null(),
            null(),
            flags,
            TIMER_MODIFY_STATE | SYNCHRONIZATION_SYNCHRONIZE,
        )
    };
    let mut handle = create(CREATE_WAITABLE_TIMER_HIGH_RESOLUTION);
    if handle.is_null()
        && io::Error::last_os_error().raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32)
    {
        handle = create(0);
    }
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

// Wait for the next half-cycle or keyboard message on the same hook thread.
// A negative due time is a relative delay in 100 ns units.
fn next_message(timer: &OwnedHandle, deadline: Option<Instant>) -> io::Result<Option<MSG>> {
    let mut msg = MSG::default();
    let Some(deadline) = deadline else {
        return match unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) } {
            -1 => Err(io::Error::last_os_error()),
            _ => Ok(Some(msg)),
        };
    };
    // Check for focus loss even when the configured interval is a full second.
    let delay = deadline
        .saturating_duration_since(Instant::now())
        .min(Duration::from_millis(10));
    let handle = timer.as_raw_handle();
    if !delay.is_zero() {
        let due = -(delay.as_nanos().div_ceil(100) as i64);
        if unsafe { SetWaitableTimer(handle, &due, 0, None, null(), 0) } == 0
            || unsafe {
                MsgWaitForMultipleObjectsEx(1, &handle, INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE)
            } == WAIT_FAILED
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok((unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) } != 0).then_some(msg))
}

fn run(
    config: Settings,
    status: &WorkerStatus,
    ready: &mpsc::SyncSender<io::Result<u32>>,
) -> io::Result<()> {
    let interval = Duration::from_millis(config.interval_ms);
    let timer = repeat_timer()?;
    let own_integrity = crate::privileges::integrity(unsafe { GetCurrentProcess() }).ok();
    let mut msg = MSG::default();
    PHYSICAL.with_borrow_mut(|p| p.selected = config.keys);
    unsafe {
        PeekMessageW(&mut msg, null_mut(), 0, 0, PM_NOREMOVE);
    }
    let hook =
        unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_hook), GetModuleHandleW(null()), 0) };
    if hook.is_null() {
        return Err(io::Error::last_os_error());
    }
    let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));
    let mut held: BTreeMap<u16, Hold> = BTreeMap::new();
    let result = loop {
        let message = match next_message(&timer, held.values().map(|hold| hold.due).min()) {
            Ok(message) => message,
            Err(err) => break Err(err),
        };
        if let Some(msg) = message {
            if msg.message == WM_QUIT {
                break Ok(());
            }
            if msg.message != EDGE {
                continue;
            }
            let key = msg.wParam as u16;
            let hwnd = msg.lParam as HWND;
            if let Some(hold) = held.remove(&key) {
                hold.finish(key, status);
            }
            if !hwnd.is_null()
                && hwnd == unsafe { GetForegroundWindow() }
                && is_game(hwnd, own_integrity, status)
                && PHYSICAL.with_borrow(|p| p.down.contains(&key))
            {
                held.insert(
                    key,
                    Hold {
                        hwnd,
                        due: Instant::now() + interval,
                        synthetic_down: false,
                    },
                );
            }
        } else {
            let now = Instant::now();
            held.retain(|&key, hold| {
                let foreground = unsafe { GetForegroundWindow() };
                let down = PHYSICAL.with_borrow(|p| p.down.contains(&key));
                let keep = hold.tick(
                    now,
                    interval / 2,
                    foreground,
                    down,
                    |up| send(key, up),
                    status,
                );
                if !keep {
                    hold.finish(key, status);
                }
                keep
            });
        }
    };
    unsafe {
        for (key, hold) in held {
            hold.finish(key, status);
        }
        UnhookWindowsHookEx(hook);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_timer_wait_wakes_for_deadline_and_for_queued_release() {
        let timer = repeat_timer().unwrap();
        let deadline = Instant::now() + Duration::from_micros(2500);
        while Instant::now() < deadline {
            next_message(&timer, Some(deadline)).unwrap();
        }
        assert_ne!(
            unsafe { PostThreadMessageW(GetCurrentThreadId(), EDGE, 31, 0) },
            0
        );
        let msg = next_message(&timer, Some(Instant::now() + Duration::from_secs(1)))
            .unwrap()
            .unwrap();
        assert_eq!((msg.message, msg.wParam, msg.lParam), (EDGE, 31, 0));
    }

    #[test]
    fn held_key_produces_distinct_taps_and_stops_on_release_or_focus_loss() {
        let hwnd = std::ptr::dangling_mut();
        let period = Duration::from_millis(40);
        let start = Instant::now();
        let (changed, notifications) = async_channel::bounded(1);
        let status = WorkerStatus::new(changed);
        let mut hold = Hold {
            hwnd,
            due: start + period,
            synthetic_down: false,
        };
        let mut edges = Vec::new();
        for ms in [0, 39, 40, 79, 80, 120, 160, 200] {
            assert!(hold.tick(
                start + Duration::from_millis(ms),
                period,
                hwnd,
                true,
                |up| {
                    edges.push(up);
                    true
                },
                &status
            ));
        }
        assert_eq!(edges, [false, true, false, true, false]);
        for (foreground, down) in [(null_mut(), true), (hwnd, false)] {
            assert!(!hold.tick(
                start + period * 10,
                period,
                foreground,
                down,
                |_| panic!("unexpected output"),
                &status
            ));
        }
        assert_eq!(
            notifications.try_recv(),
            Err(async_channel::TryRecvError::Empty)
        );
        assert!(!hold.tick(start + period * 10, period, hwnd, true, |_| false, &status));
        assert!(status.failed.load(Ordering::Relaxed));
        assert_eq!(notifications.try_recv(), Ok(()));
        status.report_failure();
        assert_eq!(
            notifications.try_recv(),
            Err(async_channel::TryRecvError::Empty)
        );
    }

    #[test]
    fn keypad_enter_is_distinct_and_generated_edges_cannot_retrigger() {
        let mut event = KBDLLHOOKSTRUCT {
            scanCode: 0x1c,
            ..Default::default()
        };
        assert_eq!(hardware_key(&event), Some(0x1c));
        event.flags = LLKHF_EXTENDED;
        assert_eq!(hardware_key(&event), Some(0x11c));
        event.flags |= LLKHF_INJECTED;
        assert_eq!(hardware_key(&event), None);
        // INPUT_KEYBOARD is the active union member produced by repeat_input().
        let press = unsafe { repeat_input(0x1f, false).Anonymous.ki };
        assert_eq!((press.wVk, press.wScan, press.dwFlags), (0xff, 0x1f, 0));
        let release = unsafe { repeat_input(0x11c, true).Anonymous.ki };
        assert_eq!(release.wVk, 0xff);
        assert_eq!(release.wScan, 0x1c);
        assert_eq!(release.dwFlags, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP);
    }

    #[test]
    #[ignore = "Requires an interactive Windows desktop and briefly focuses a test edit control"]
    fn repeat_pulses_preserve_scan_code_without_adding_text() {
        use ::windows::Win32::{
            Foundation::HWND as WindowHandle,
            UI::Input::Ime::{HIMC, ImmAssociateContext},
        };
        unsafe {
            let edit = CreateWindowExW(
                0,
                windows_sys::w!("EDIT"),
                windows_sys::w!("Noia input test - click here"),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                320,
                100,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            assert!(!edit.is_null());
            // Test committed Latin text, independent of the user's active IME.
            let previous_ime = ImmAssociateContext(WindowHandle(edit), HIMC::default());
            ShowWindow(edit, SW_SHOW);
            SetForegroundWindow(edit);
            SetFocus(edit);
            let ready = Instant::now() + Duration::from_secs(45);
            while GetForegroundWindow() != edit && Instant::now() < ready {
                let mut msg = MSG::default();
                while PeekMessageW(&mut msg, edit, 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                thread::sleep(Duration::from_millis(20));
            }
            SetWindowTextW(edit, windows_sys::w!(""));
            // Use a plain S as the physical-key stand-in, bracketing four actual
            // production pulses. Submit one batch only to our focused window.
            let mut ordinary_down = repeat_input(0x1f, false);
            ordinary_down.Anonymous.ki.wVk = 0x53;
            let mut ordinary_up = ordinary_down;
            ordinary_up.Anonymous.ki.dwFlags = KEYEVENTF_KEYUP;
            let mut events = vec![ordinary_down];
            for _ in 0..4 {
                events.extend([repeat_input(0x1f, false), repeat_input(0x1f, true)]);
            }
            events.push(ordinary_up);
            assert_eq!(GetForegroundWindow(), edit);
            assert_eq!(GetFocus(), edit);
            assert_eq!(
                SendInput(
                    events.len() as u32,
                    events.as_ptr(),
                    size_of::<INPUT>() as i32
                ),
                events.len() as u32
            );

            let mut pulses = Vec::new();
            let mut saw_release = false;
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                let mut msg = MSG::default();
                while PeekMessageW(&mut msg, edit, 0, 0, PM_REMOVE) != 0 {
                    if matches!(msg.message, WM_KEYDOWN | WM_KEYUP) {
                        let scan = (msg.lParam >> 16) & 0xff;
                        if msg.wParam == REPEAT_VK as usize {
                            pulses.push((msg.message == WM_KEYUP, scan));
                        }
                        saw_release |= msg.message == WM_KEYUP && msg.wParam == 0x53;
                    }
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                if saw_release || Instant::now() >= deadline {
                    break;
                }
                thread::sleep(Duration::from_millis(2));
            }
            let mut text = [0u16; 32];
            let len = GetWindowTextW(edit, text.as_mut_ptr(), text.len() as i32);
            ImmAssociateContext(WindowHandle(edit), previous_ime);
            DestroyWindow(edit);
            assert!(saw_release);
            assert_eq!(pulses, [(false, 0x1f), (true, 0x1f)].repeat(4));
            assert_eq!(
                String::from_utf16_lossy(&text[..len as usize]).to_ascii_lowercase(),
                "s"
            );
        }
    }

    #[test]
    fn windows_hook_starts_and_shuts_down_without_any_selected_keys() {
        let worker = Worker::start(
            Settings::default(),
            Arc::new(WorkerStatus::new(async_channel::bounded(1).0)),
        )
        .unwrap();
        drop(worker);
    }
}
