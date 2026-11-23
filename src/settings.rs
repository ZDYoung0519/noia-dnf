use serde::{Serialize, de::DeserializeOwned};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

pub fn path(name: &str) -> io::Result<PathBuf> {
    let exe = env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or_else(|| io::Error::other("Executable has no parent"))?;
    Ok(dir.join("settings").join(name))
}

pub fn legacy(name: &str) -> io::Result<Vec<u8>> {
    let dir = dirs::config_dir().ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
    fs::read(dir.join("Noia DNF").join(name))
}

pub fn read_or_create<T: DeserializeOwned + Serialize>(
    path: &Path,
    initial: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    match fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let value = initial()?;
            save(path, &value)?;
            Ok(value)
        }
        Err(e) => Err(e),
    }
}

pub fn save(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let dir = path.parent().expect("settings file has a directory");
    fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer_pretty(file.as_file_mut(), value)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}
