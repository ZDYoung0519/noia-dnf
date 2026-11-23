#![cfg(windows)]
mod routes;
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
    downloads: Vec<String>,
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
        update: Some(Release {
            version,
            downloads: Vec::new(),
            asset,
        }),
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
    routes::check(releases, &current)
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
    download_in(&dir, release, progress)
}

fn download_in(dir: &Path, release: Release, progress: impl FnMut(u8)) -> Result<Installer> {
    let client = client(Duration::from_secs(600))?;
    download_from(dir, release, progress, |url| {
        Ok(client
            .get(url)
            .header(reqwest::header::ACCEPT, "application/octet-stream")
            .send()?
            .error_for_status()?)
    })
}

fn download_from<R: Read>(
    dir: &Path,
    release: Release,
    mut progress: impl FnMut(u8),
    mut request: impl FnMut(&str) -> Result<R>,
) -> Result<Installer> {
    let path = dir.join(filename(&release.version));
    if let Ok(cached) = Installer::open(&path, &release.asset) {
        return Ok(cached);
    }
    let mut errors = Vec::new();
    for url in &release.downloads {
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        progress(0);
        if let Err(error) = request(url)
            .and_then(|response| copy_verified(response, &mut file, &release.asset, &mut progress))
        {
            errors.push(format!("{url}: {error:#}"));
            continue;
        }
        file.as_file().sync_all()?;
        drop(file.persist(&path)?);
        return Installer::open(&path, &release.asset);
    }
    bail!("All installer downloads failed: {}", errors.join(", "))
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

    #[test]
    fn retries_failed_or_corrupt_downloads_in_latency_order() {
        let mut release = parse_manifest(
            &serde_json::to_vec(&manifest("1.0.0")).unwrap(),
            &Version::new(0, 1, 0),
            RELEASES,
        )
        .unwrap()
        .update
        .unwrap();
        release.downloads = ["broken", "corrupt", "good"].map(String::from).to_vec();
        let dir = tempfile::tempdir().unwrap();
        let mut attempted = Vec::new();
        let installer = download_from(
            dir.path(),
            release,
            |_| {},
            |url| {
                attempted.push(url.to_string());
                match url {
                    "broken" => bail!("Timeout"),
                    "corrupt" => Ok(&b"bad"[..]),
                    _ => Ok(&b"abc"[..]),
                }
            },
        )
        .unwrap();
        assert_eq!(attempted, ["broken", "corrupt", "good"]);
        assert_eq!(fs::read(installer.path).unwrap(), b"abc");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    #[ignore = "Downloads the public installer into a temporary directory"]
    fn checks_and_downloads_published_release() -> Result<()> {
        let releases = std::env::var("NOIA_RELEASE_URL")?;
        let expected = Version::parse(&std::env::var("NOIA_TEST_RELEASE_VERSION")?)?;
        let check = routes::check(&releases, &Version::new(0, 0, 0))?;
        let release = check.update.context("Missing update")?;
        assert_eq!(release.version, expected);
        assert_eq!(check.changelog.unwrap().version, expected.to_string());
        eprintln!("Ranked download URLs: {:?}", release.downloads);
        let dir = tempfile::tempdir()?;
        let installer = download_in(dir.path(), release, |_| {})?;
        assert!(installer.path.is_file());
        Ok(())
    }
}
