use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io, path::Path};
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::Worker;

pub struct WorkerStatus {
    pub higher_integrity: std::sync::atomic::AtomicBool,
    pub failed: std::sync::atomic::AtomicBool,
    changed: async_channel::Sender<()>,
}
impl WorkerStatus {
    pub fn new(changed: async_channel::Sender<()>) -> Self {
        Self {
            higher_integrity: Default::default(),
            changed,
            failed: Default::default(),
        }
    }

    pub fn report_failure(&self) {
        if !self.failed.swap(true, std::sync::atomic::Ordering::Relaxed) {
            let _ = self.changed.try_send(());
        }
    }
}

#[cfg(not(windows))]
pub struct Worker;
#[cfg(not(windows))]
impl Worker {
    pub fn start(_: Settings, _: std::sync::Arc<WorkerStatus>) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Key repeat requires Windows",
        ))
    }
}

pub const MIN_INTERVAL_MS: u64 = 5;
pub const MAX_INTERVAL_MS: u64 = 1000;

// Set 1 make code, with bit 8 for the E0 prefix.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    pub keys: BTreeSet<u16>,
    #[serde(default = "default_interval")]
    pub interval_ms: u64,
}

fn default_interval() -> u64 {
    50
}

impl Settings {
    pub fn load() -> io::Result<Self> {
        Self::read(&crate::settings::path("rapid-fire.json")?)
    }
    fn read(path: &Path) -> io::Result<Self> {
        crate::settings::read_or_create(path, Self::default())?.validated()
    }
    fn validated(self) -> io::Result<Self> {
        if !(MIN_INTERVAL_MS..=MAX_INTERVAL_MS).contains(&self.interval_ms)
            || self
                .keys
                .iter()
                .any(|&key| !crate::keyboard::selectable(key))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid rapid-fire settings",
            ));
        }
        Ok(self)
    }
    pub fn save(&self) -> io::Result<()> {
        crate::settings::save(&crate::settings::path("rapid-fire.json")?, self)
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            keys: BTreeSet::new(),
            interval_ms: default_interval(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_round_trip_and_invalid_input() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("rapid-fire.json");
        let read = || Settings::read(&file);
        let defaults = read().unwrap();
        assert!(!defaults.enabled);
        assert_eq!(defaults.interval_ms, 50);
        assert!(defaults.keys.is_empty());
        assert_eq!(
            serde_json::from_slice::<Settings>(&std::fs::read(&file).unwrap()).unwrap(),
            defaults
        );
        std::fs::write(&file, b"{}").unwrap();
        assert_eq!(read().unwrap().interval_ms, 50);
        let mut config = Settings {
            enabled: true,
            interval_ms: 120,
            keys: [0x1f, 0x11c].into(),
        };
        for ms in [5, 10, 37, 50, 80, 120, 200, 999, 1000] {
            config.interval_ms = ms;
            crate::settings::save(&file, &config).unwrap();
            assert_eq!(read().unwrap(), config);
        }
        for ms in [0, 4, 1001] {
            config.interval_ms = ms;
            crate::settings::save(&file, &config).unwrap();
            assert!(read().is_err());
        }
    }
}
