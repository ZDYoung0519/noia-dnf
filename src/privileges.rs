use std::{
    io,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{ERROR_CANCELLED, ERROR_INSUFFICIENT_BUFFER, HANDLE},
    Security::{
        GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TOKEN_MANDATORY_LABEL,
        TOKEN_QUERY, TokenIntegrityLevel,
    },
    System::{
        SystemServices::SECURITY_MANDATORY_HIGH_RID,
        Threading::{GetCurrentProcess, OpenProcessToken},
    },
    UI::{
        Shell::{SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW},
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

pub fn integrity(process: HANDLE) -> io::Result<u32> {
    unsafe {
        let mut token = null_mut();
        if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = OwnedHandle::from_raw_handle(token);
        let mut bytes = 0;
        GetTokenInformation(
            token.as_raw_handle(),
            TokenIntegrityLevel,
            null_mut(),
            0,
            &mut bytes,
        );
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) || bytes == 0 {
            return Err(error);
        }
        // The header contains a pointer to the SID within this same aligned buffer.
        let mut buffer = vec![0usize; (bytes as usize).div_ceil(size_of::<usize>())];
        if GetTokenInformation(
            token.as_raw_handle(),
            TokenIntegrityLevel,
            buffer.as_mut_ptr().cast(),
            bytes,
            &mut bytes,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let sid = (*buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()).Label.Sid;
        let index = (*GetSidSubAuthorityCount(sid))
            .checked_sub(1)
            .ok_or_else(|| io::Error::other("Missing integrity RID"))?;
        Ok(*GetSidSubAuthority(sid, index.into()))
    }
}

/// Only the elevated instance proceeds to single-instance election and starts GPUI.
pub fn ensure_high_integrity(autostart: bool) -> io::Result<bool> {
    if integrity(unsafe { GetCurrentProcess() })? >= SECURITY_MANDATORY_HIGH_RID as u32 {
        return Ok(true);
    }
    let exe: Vec<u16> = std::env::current_exe()?
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect();
    let mut launch = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC,
        lpVerb: windows_sys::w!("runas"),
        lpFile: exe.as_ptr(),
        lpParameters: if autostart {
            windows_sys::w!("--autostart")
        } else {
            null()
        },
        nShow: SW_SHOWNORMAL,
        ..Default::default()
    };
    if unsafe { ShellExecuteExW(&mut launch) } == 0 {
        let err = io::Error::last_os_error();
        if err.raw_os_error() != Some(ERROR_CANCELLED as i32) {
            return Err(err);
        }
    }
    Ok(false)
}
