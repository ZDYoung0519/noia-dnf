#![cfg(windows)]
use anyhow::{Context, Result, bail, ensure};
use reqwest::blocking::Client;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs,
    io::{Read, Write},
    os::windows::{ffi::OsStringExt, fs::OpenOptionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use windows_sys::Win32::{
    System::Com::CoTaskMemFree,
    UI::Shell::{FOLDERID_ProgramFilesX64, SHGetKnownFolderPath},
};

const MAX_MANIFEST: u64 = 1024 * 1024;
const MAX_INSTALLER: u64 = 256 * 1024 * 1024;

#[derive(Deserialize)]
struct Manifest {
    schema_version: u32,
    version: Version,
    #[serde(default)]
    changelog: Option<Changelog>,
    assets: BTreeMap<String, Asset>,
}

#[derive(Clone, Deserialize)]
pub struct Changelog {
    pub version: String,
    pub items: Vec<BTreeMap<String, String>>,
}

#[derive(Clone, Deserialize)]
struct Asset {
    url: String,
    size: u64,
    sha256: String,
}

#[derive(Clone)]
pub struct Release {
    pub version: Version,
    asset: Asset,
}

pub struct Check {
    pub update: Option<Release>,
    pub changelog: Option<Changelog>,
}

fn filename(version: &Version) -> String {
    format!("Noia-DNF-{version}-windows-x64-setup.exe")
}

fn parse_manifest(bytes: &[u8], current: &Version, releases: &str) -> Result<Check> {
    let mut manifest: Manifest = serde_json::from_slice(bytes)?;
    ensure!(manifest.schema_version == 1, "Unsupported update manifest");
    let changelog = manifest.changelog.filter(|log| {
        log.version == manifest.version.to_string()
            && !log.items.is_empty()
            && manifest.version.cmp_precedence(current).is_ge()
    });
    if manifest.version.cmp_precedence(current).is_le() {
        return Ok(Check {
            update: None,
            changelog,
        });
    }
    let version = manifest.version;
    let asset = manifest
        .assets
        .remove("windows-x86_64")
        .context("Missing Windows x64 installer")?;
    let expected = format!("{releases}/download/v{version}/{}", filename(&version));
    ensure!(asset.url == expected, "Unexpected release asset URL");
    ensure!(
        (1..=MAX_INSTALLER).contains(&asset.size),
        "Invalid installer size"
    );
    ensure!(
        asset.sha256.len() == 64 && asset.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid SHA-256"
    );
    Ok(Check {
        update: Some(Release { version, asset }),
        changelog,
    })
}

fn client(timeout: Duration) -> Result<Client> {
    Ok(Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout)
        .user_agent(concat!("Noia-DNF/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

pub fn check(releases: &str, current: &str) -> Result<Check> {
    let current = Version::parse(current)?;
    if let Ok(dir) = cache_dir() {
        remove_old_installers(&dir, &current);
    }
    let response = client(Duration::from_secs(20))?
        .get(format!("{releases}/latest/download/latest.json"))
        .send()?
        .error_for_status()?;
    let mut bytes = Vec::new();
    response.take(MAX_MANIFEST + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_MANIFEST,
        "Update manifest too large"
    );
    parse_manifest(&bytes, &current, releases)
}

// The app launches the installer elevated. Stage it under the system Program Files
// directory so ordinary processes cannot change the verified file or plant DLLs beside it.
fn cache_dir() -> Result<PathBuf> {
    let mut ptr = std::ptr::null_mut();
    let hr = unsafe {
        SHGetKnownFolderPath(&FOLDERID_ProgramFilesX64, 0, std::ptr::null_mut(), &mut ptr)
    };
    ensure!(hr >= 0, "Could not locate Program Files ({hr:#x})");
    let path = unsafe {
        let mut len = 0;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        let path = OsString::from_wide(std::slice::from_raw_parts(ptr, len));
        CoTaskMemFree(ptr.cast());
        PathBuf::from(path)
    };
    Ok(path.join("Noia DNF Update Cache"))
}

fn remove_old_installers(dir: &Path, current: &Version) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let version = name
            .to_str()
            .and_then(|s| s.strip_prefix("Noia-DNF-"))
            .and_then(|s| s.strip_suffix("-windows-x64-setup.exe"))
            .and_then(|s| Version::parse(s).ok());
        if version.is_some_and(|v| v.cmp_precedence(current).is_le()) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn copy_verified(
    mut input: impl Read,
    mut output: impl Write,
    asset: &Asset,
    mut progress: impl FnMut(u8),
) -> Result<()> {
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut last = 0;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        ensure!(size <= asset.size, "Installer exceeds declared size");
        hash.update(&buf[..n]);
        output.write_all(&buf[..n])?;
        let percent = (size * 100 / asset.size) as u8;
        if percent != last {
            progress(percent);
            last = percent;
        }
    }
    ensure!(size == asset.size, "Incomplete installer");
    ensure!(
        format!("{:x}", hash.finalize()).eq_ignore_ascii_case(&asset.sha256),
        "Installer SHA-256 mismatch"
    );
    Ok(())
}

pub struct Installer {
    path: PathBuf,
    _verified_file: fs::File,
}

impl Installer {
    fn open(path: &Path, asset: &Asset) -> Result<Self> {
        // Keep this read-only share open until CreateProcess has mapped the same file.
        let file = fs::File::options().read(true).share_mode(1).open(path)?;
        copy_verified(&file, std::io::sink(), asset, |_| {})?;
        Ok(Self {
            path: path.to_owned(),
            _verified_file: file,
        })
    }
}

pub fn download(release: Release, progress: impl FnMut(u8)) -> Result<Installer> {
    let dir = cache_dir()?;
    fs::create_dir_all(&dir)?;
    let path = dir.join(filename(&release.version));
    if let Ok(cached) = Installer::open(&path, &release.asset) {
        return Ok(cached);
    }
    let mut file = tempfile::NamedTempFile::new_in(&dir)?;
    let response = client(Duration::from_secs(600))?
        .get(&release.asset.url)
        .send()?
        .error_for_status()?;
    copy_verified(response, &mut file, &release.asset, progress)?;
    file.as_file().sync_all()?;
    drop(file.persist(&path)?);
    Installer::open(&path, &release.asset)
}

pub fn launch(prepared: &Installer) -> Result<()> {
    let installer = &prepared.path;
    let exe = std::env::current_exe()?;
    if !exe
        .file_name()
        .is_some_and(|s| s.eq_ignore_ascii_case(OsStr::new("noia-dnf.exe")))
    {
        bail!("Expected the executable to be named noia-dnf.exe");
    }
    // NSIS requires /D= to be the final argument, unquoted even for paths with spaces.
    let mut directory = OsString::from("/D=");
    directory.push(exe.parent().context("Missing application directory")?);
    Command::new(installer)
        .current_dir(installer.parent().context("Missing installer directory")?)
        .arg(format!("/NOIAUPDATE={}", std::process::id()))
        .raw_arg(directory)
        .spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const RELEASES: &str = "https://github.com/example/noia/releases";

    fn manifest(version: &str) -> serde_json::Value {
        serde_json::json!({
            "schema_version": 1, "version": version,
            "assets": {"windows-x86_64": {
                "url": format!("{RELEASES}/download/v{version}/Noia-DNF-{version}-windows-x64-setup.exe"),
                "size": 3, "sha256": format!("{:x}", Sha256::digest(b"abc"))
            }}
        })
    }

    #[test]
    fn accepts_only_newer_versions_and_the_expected_release_asset() {
        let current = Version::new(0, 2, 0);
        let parse = |value: &serde_json::Value| {
            parse_manifest(&serde_json::to_vec(value).unwrap(), &current, RELEASES)
        };
        for version in ["0.1.99", "0.2.0", "0.2.0+rebuild"] {
            assert!(parse(&manifest(version)).unwrap().update.is_none());
        }
        let mut value = manifest("0.10.0");
        assert!(parse(&value).unwrap().update.is_some());
        value["assets"]["windows-x86_64"]["url"] = "https://example.com/other.exe".into();
        assert!(parse(&value).is_err());
    }

    #[test]
    fn verifies_actual_bytes_before_install() {
        let release = parse_manifest(
            &serde_json::to_vec(&manifest("1.0.0")).unwrap(),
            &Version::new(0, 1, 0),
            RELEASES,
        )
        .unwrap()
        .update
        .unwrap();
        let mut progress = Vec::new();
        assert!(
            copy_verified(&b"abc"[..], Vec::new(), &release.asset, |p| progress
                .push(p))
            .is_ok()
        );
        assert_eq!(progress, [100]);
        for invalid in [&b"ab"[..], &b"abd"[..], &b"abcd"[..]] {
            assert!(copy_verified(invalid, Vec::new(), &release.asset, |_| {}).is_err());
        }
    }
}
