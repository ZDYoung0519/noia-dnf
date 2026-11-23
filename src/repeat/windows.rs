use super::{Settings, Stats};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::Path,
    ptr::{null, null_mut},
    sync::{Arc, atomic::Ordering, mpsc},
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_INVALID_PARAMETER, HWND, LPARAM, LRESULT, WAIT_FAILED, WPARAM,
    },
    System::{LibraryLoader::GetModuleHandleW, Threading::*},
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

const EDGE: u32 = WM_APP + 1;

pub struct Worker {
    id: u32,
    thread: Option<thread::JoinHandle<()>>,
}

impl Worker {
    pub fn start(config: Settings, stats: Arc<Stats>) -> io::Result<Self> {
        let (ready, started) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("noia-key-repeat".into())
            .spawn(move || {
                if let Err(err) = run(config, &stats, &ready) {
                    stats.failed.store(true, Ordering::Relaxed);
                    let _ = stats.changed.try_send(());
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
    repeating: BTreeMap<u16, usize>,
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
            let suppress = PHYSICAL.with_borrow_mut(|physical| {
                if !physical.selected.contains(&key) {
                    return false;
                }
                let hwnd = unsafe { GetForegroundWindow() } as usize;
                if event.flags & LLKHF_UP != 0 {
                    physical.down.remove(&key);
                    physical.repeating.remove(&key);
                    unsafe {
                        PostThreadMessageW(GetCurrentThreadId(), EDGE, key as usize, 0);
                    }
                } else if physical.down.insert(key) {
                    unsafe {
                        PostThreadMessageW(GetCurrentThreadId(), EDGE, key as usize, hwnd as isize);
                    }
                } else {
                    return physical.repeating.get(&key) == Some(&hwnd);
                }
                false
            });
            if suppress {
                return 1;
            }
        }
    }
    unsafe { CallNextHookEx(null_mut(), code, msg, data) }
}

fn is_game(hwnd: HWND, own_integrity: Option<u32>, stats: &Stats) -> bool {
    if hwnd.is_null() {
        return false;
    }
    let mut pid = 0;
    let mut path = [0u16; 1024];
    let mut length = path.len() as u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let ok = QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length);
        let game = ok != 0
            && Path::new(&String::from_utf16_lossy(&path[..length as usize]))
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("DNF.exe"));
        let mut blocked = false;
        if game {
            blocked = own_integrity
                .zip(crate::privileges::integrity(process).ok())
                .is_some_and(|(own, target)| target > own);
            if stats.higher_integrity.swap(blocked, Ordering::Relaxed) != blocked {
                let _ = stats.changed.try_send(());
            }
        }
        CloseHandle(process);
        game && !blocked
    }
}

fn input(key: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wScan: key & 0xff,
                dwFlags: KEYEVENTF_SCANCODE
                    | if key & 0x100 != 0 {
                        KEYEVENTF_EXTENDEDKEY
                    } else {
                        0
                    }
                    | if up { KEYEVENTF_KEYUP } else { 0 },
                ..Default::default()
            },
        },
    }
}

// The physical down starts the first press. Space releases and presses apart so
// clients sampling keyboard state can observe both phases.
struct Hold {
    hwnd: HWND,
    due: Instant,
    release_next: bool,
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
        stats: &Stats,
    ) -> bool {
        if !down || foreground != self.hwnd {
            return false;
        }
        if now < self.due {
            return true;
        }
        if !emit(self.release_next) {
            stats.failed.store(true, Ordering::Relaxed);
            return false;
        }
        if self.release_next && self.synthetic_down {
            stats.sent.fetch_add(1, Ordering::Relaxed);
            let _ = stats.changed.try_send(());
        }
        self.synthetic_down = !self.release_next;
        self.release_next = !self.release_next;
        self.due = if self.due + half_period > now {
            self.due + half_period
        } else {
            now + half_period
        };
        true
    }
}

fn send(key: u16, up: bool) -> bool {
    let event = input(key, up);
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
    stats: &Stats,
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
            held.remove(&key);
            if !hwnd.is_null()
                && hwnd == unsafe { GetForegroundWindow() }
                && is_game(hwnd, own_integrity, stats)
                && PHYSICAL.with_borrow(|p| p.down.contains(&key))
            {
                held.insert(
                    key,
                    Hold {
                        hwnd,
                        due: Instant::now() + interval / 2,
                        release_next: true,
                        synthetic_down: false,
                    },
                );
                PHYSICAL.with_borrow_mut(|p| p.repeating.insert(key, hwnd as usize));
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
                    stats,
                );
                if !keep {
                    let _ = stats.changed.try_send(());
                    PHYSICAL.with_borrow_mut(|p| p.repeating.remove(&key));
                    if down && hold.synthetic_down {
                        send(key, true);
                    }
                }
                keep
            });
        }
    };
    unsafe {
        for (key, hold) in held {
            if hold.synthetic_down {
                send(key, true);
            }
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
        let stats = Stats::new(async_channel::bounded(1).0);
        let mut hold = Hold {
            hwnd,
            due: start + period,
            release_next: true,
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
                &stats
            ));
        }
        assert_eq!(edges, [true, false, true, false, true]);
        assert_eq!(stats.sent.load(Ordering::Relaxed), 2);
        for (foreground, down) in [(null_mut(), true), (hwnd, false)] {
            assert!(!hold.tick(
                start + period * 10,
                period,
                foreground,
                down,
                |_| panic!("unexpected output"),
                &stats
            ));
        }
        assert!(!hold.tick(start + period * 10, period, hwnd, true, |_| false, &stats));
        assert!(stats.failed.load(Ordering::Relaxed));
        assert_eq!(stats.sent.load(Ordering::Relaxed), 2);
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
        // INPUT_KEYBOARD is the active union member produced by input().
        let release = unsafe { input(0x11c, true).Anonymous.ki };
        assert_eq!(release.wScan, 0x1c);
        assert_eq!(
            release.dwFlags,
            KEYEVENTF_SCANCODE | KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP
        );
    }

    #[test]
    fn windows_hook_starts_and_shuts_down_without_any_selected_keys() {
        let worker = Worker::start(
            Settings::default(),
            Arc::new(Stats::new(async_channel::bounded(1).0)),
        )
        .unwrap();
        drop(worker);
    }
}
