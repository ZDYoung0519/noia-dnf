use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use windows_sys::Win32::{
    Foundation::HWND,
    System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    },
    UI::WindowsAndMessaging::GetWindowThreadProcessId,
};

pub fn dnf_process(hwnd: HWND) -> Option<OwnedHandle> {
    let mut pid = 0;
    unsafe {
        if GetWindowThreadProcessId(hwnd, &mut pid) == 0 {
            return None;
        }
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let process = OwnedHandle::from_raw_handle(handle);
        let mut path = [0u16; 1024];
        let mut len = path.len() as u32;
        if QueryFullProcessImageNameW(process.as_raw_handle(), 0, path.as_mut_ptr(), &mut len) == 0
        {
            return None;
        }
        let path = String::from_utf16_lossy(&path[..len as usize]);
        std::path::Path::new(&path)
            .file_name()?
            .eq_ignore_ascii_case(std::ffi::OsStr::new("DNF.exe"))
            .then_some(process)
    }
}
