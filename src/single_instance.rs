//! Windows single-instance gate, following Zed's application-layer named-mutex
//! approach. A named event suffices here because there are no CLI payloads.
use std::{
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
};

use windows::{
    Win32::{
        Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_OBJECT_0},
        System::Threading::{CreateEventW, CreateMutexW, INFINITE, SetEvent, WaitForSingleObject},
    },
    core::HSTRING,
};

// Local scopes the application to the current interactive Windows session.
const APP_ID: &str = "Local\\Noia.DNF.8EA050E3-27CF-4B7D-9BD3-AF8AB047652F";

pub struct SingleInstance {
    _mutex: OwnedHandle,
    activate: OwnedHandle,
}

impl SingleInstance {
    /// Manual launches activate the existing window. Logon launches yield silently.
    pub fn acquire(activate_existing: bool) -> windows::core::Result<Option<Self>> {
        Self::acquire_named(APP_ID, activate_existing)
    }

    fn acquire_named(name: &str, activate_existing: bool) -> windows::core::Result<Option<Self>> {
        // Create/open the event before election so another launch can signal
        // immediately, even while the first process is still initializing GPUI.
        let activate = unsafe {
            let handle = CreateEventW(
                None,
                false,
                false,
                &HSTRING::from(format!("{name}.Activate")),
            )?;
            OwnedHandle::from_raw_handle(handle.0)
        };
        let (mutex, already_running) = unsafe {
            let handle = CreateMutexW(None, false, &HSTRING::from(format!("{name}.Instance")))?;
            // Read last-error immediately after CreateMutexW.
            let exists = GetLastError() == ERROR_ALREADY_EXISTS;
            (OwnedHandle::from_raw_handle(handle.0), exists)
        };
        if already_running {
            // SAFETY: activate owns a valid event handle throughout the call.
            if activate_existing {
                unsafe { SetEvent(HANDLE(activate.as_raw_handle()))? };
            }
            return Ok(None);
        }
        Ok(Some(Self {
            _mutex: mutex,
            activate,
        }))
    }

    pub fn listen(&self) -> io::Result<async_channel::Receiver<()>> {
        let activate = self.activate.try_clone()?;
        let (sender, receiver) = async_channel::bounded(1);
        std::thread::Builder::new()
            .name("noia-instance-activation".into())
            .spawn(move || {
                loop {
                    // SAFETY: the thread owns this event handle. The OS releases it
                    // when the process exits; no polling is needed while idle.
                    let result =
                        unsafe { WaitForSingleObject(HANDLE(activate.as_raw_handle()), INFINITE) };
                    if result != WAIT_OBJECT_0 {
                        eprintln!("Single-instance event wait failed: {:?}", unsafe {
                            GetLastError()
                        });
                        break;
                    }
                    match sender.try_send(()) {
                        Ok(()) | Err(async_channel::TrySendError::Full(())) => {}
                        Err(async_channel::TrySendError::Closed(())) => break,
                    }
                }
            })?;
        Ok(receiver)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn secondary_process() {
        let Ok(name) = std::env::var("NOIA_TEST_INSTANCE_NAME") else {
            return;
        };
        assert!(
            SingleInstance::acquire_named(&name, true)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn duplicate_launch_signals_primary_and_exit_releases_gate() {
        let name = format!("Local\\Noia.Test.{}", std::process::id());
        let primary = SingleInstance::acquire_named(&name, true).unwrap().unwrap();
        assert!(
            SingleInstance::acquire_named(&name, false)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            unsafe { WaitForSingleObject(HANDLE(primary.activate.as_raw_handle()), 0) },
            windows::Win32::Foundation::WAIT_TIMEOUT
        );
        // Signal before any listener exists, covering the startup race.
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "single_instance::tests::secondary_process",
                "--nocapture",
            ])
            .env("NOIA_TEST_INSTANCE_NAME", &name)
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(
            unsafe { WaitForSingleObject(HANDLE(primary.activate.as_raw_handle()), 1000) },
            WAIT_OBJECT_0
        );
        drop(primary);
        assert!(
            SingleInstance::acquire_named(&name, true)
                .unwrap()
                .is_some()
        );
    }
}
