use std::{
    ffi::OsString,
    io,
    os::windows::{ffi::OsStringExt, process::CommandExt},
    path::PathBuf,
    process::Command,
    ptr,
};
use windows_sys::Win32::System::{
    RemoteDesktop::*, SystemInformation::GetSystemDirectoryW, Threading::CREATE_NO_WINDOW,
};

fn session_field(field: WTS_INFO_CLASS) -> io::Result<String> {
    let mut data = ptr::null_mut();
    let mut bytes = 0;
    unsafe {
        if WTSQuerySessionInformationW(
            WTS_CURRENT_SERVER_HANDLE,
            WTS_CURRENT_SESSION,
            field,
            &mut data,
            &mut bytes,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let value = if data.is_null() || bytes < 2 {
            String::new()
        } else {
            String::from_utf16_lossy(std::slice::from_raw_parts(data, bytes as usize / 2))
                .trim_end_matches('\0')
                .to_owned()
        };
        WTSFreeMemory(data.cast());
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "Reads the interactive Windows account's Task Scheduler state"]
    fn reads_task_for_windows_session() {
        super::configure(None).unwrap();
    }
}

/// None reads the actual scheduled task. Only explicit settings changes write it.
pub fn configure(enabled: Option<bool>) -> io::Result<bool> {
    let user = session_field(WTSUserName)?;
    let domain = session_field(WTSDomainName)?;
    if user.is_empty() || domain.is_empty() {
        return Err(io::Error::other("Windows session account unavailable"));
    }
    let mut directory = [0; 260];
    let length =
        unsafe { GetSystemDirectoryW(directory.as_mut_ptr(), directory.len() as u32) } as usize;
    if length == 0 || length >= directory.len() {
        return Err(io::Error::last_os_error());
    }
    let ps_dir =
        PathBuf::from(OsString::from_wide(&directory[..length])).join(r"WindowsPowerShell\v1.0");
    let output = Command::new(ps_dir.join("powershell.exe"))
        .current_dir(&ps_dir)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            include_str!("autostart.ps1"),
        ])
        .env("PSModulePath", ps_dir.join("Modules"))
        .env("NOIA_SESSION_ACCOUNT", format!(r"{domain}\{user}"))
        .env("NOIA_STARTUP_EXE", std::env::current_exe()?)
        .env(
            "NOIA_STARTUP_OP",
            match enabled {
                Some(true) => "enable",
                Some(false) => "disable",
                None => "status",
            },
        )
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    match output.stdout.trim_ascii() {
        b"enabled" => Ok(true),
        b"disabled" => Ok(false),
        _ => Err(io::Error::other("Unexpected Task Scheduler response")),
    }
}
