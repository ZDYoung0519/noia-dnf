use super::{Check, MAX_MANIFEST, client, filename, parse_manifest};
use anyhow::{Context, Result, ensure};
use reqwest::{blocking::Client, header::ACCEPT};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    thread,
    time::{Duration, Instant},
};

// The empty prefix is the canonical GitHub release URL.
const PREFIXES: [&str; 7] = [
    "",
    "https://gh-proxy.com/",
    "https://gh-proxy.org/",
    "https://v4.gh-proxy.org/",
    "https://v6.gh-proxy.org/",
    "https://cdn.gh-proxy.org/",
    "https://axisnow.gh-proxy.org/",
];

#[derive(Deserialize)]
struct GithubRelease {
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    id: u64,
    digest: Option<String>,
}

impl GithubRelease {
    fn asset(&self, name: &str) -> Option<&GithubAsset> {
        self.assets.iter().find(|asset| asset.name == name)
    }
}

fn fetch(client: &Client, url: &str, accept: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    client
        .get(url)
        .header(ACCEPT, accept)
        .send()?
        .error_for_status()?
        .take(MAX_MANIFEST + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_MANIFEST,
        "Update metadata too large"
    );
    Ok(bytes)
}

struct Probe {
    prefix: &'static str,
    elapsed: Duration,
    bytes: Vec<u8>,
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn select(probes: &mut Vec<Probe>, github_digest: Option<&str>) -> Result<String> {
    let trusted = github_digest
        .map(str::to_owned)
        .or_else(|| {
            probes
                .iter()
                .find(|p| p.prefix.is_empty())
                .map(|p| digest(&p.bytes))
        })
        .context("Could not verify latest.json with GitHub")?;
    probes.retain(|probe| digest(&probe.bytes) == trusted);
    probes.sort_by_key(|probe| probe.elapsed);
    Ok(trusted)
}

pub(super) fn check(releases: &str, current: &Version) -> Result<Check> {
    let repository = releases
        .strip_prefix("https://github.com/")
        .and_then(|s| s.strip_suffix("/releases"))
        .context("Expected a GitHub releases URL")?;
    let api = format!("https://api.github.com/repos/{repository}/releases");
    let client = client(Duration::from_secs(10))?;
    let (metadata, mut probes) = thread::scope(|scope| {
        let metadata = scope.spawn(|| -> Result<GithubRelease> {
            let bytes = fetch(
                &client,
                &format!("{api}/latest"),
                "application/vnd.github+json",
            )?;
            Ok(serde_json::from_slice(&bytes)?)
        });
        let workers: Vec<_> = PREFIXES
            .into_iter()
            .map(|prefix| {
                let client = &client;
                scope.spawn(move || {
                    let url = format!("{prefix}{releases}/latest/download/latest.json");
                    let start = Instant::now();
                    fetch(client, &url, "application/octet-stream").map(|bytes| Probe {
                        prefix,
                        elapsed: start.elapsed(),
                        bytes,
                    })
                })
            })
            .collect();
        let probes = workers
            .into_iter()
            .filter_map(|w| w.join().ok()?.ok())
            .collect();
        (metadata.join().ok().and_then(Result::ok), probes)
    });
    let manifest_asset = metadata
        .as_ref()
        .and_then(|release| release.asset("latest.json"));
    let trusted = select(
        &mut probes,
        manifest_asset.and_then(|a| a.digest.as_deref()),
    )?;
    let api_bytes;
    let bytes = if let Some(probe) = probes.first() {
        &probe.bytes
    } else {
        let id = manifest_asset
            .context("No current release manifest available")?
            .id;
        api_bytes = fetch(
            &client,
            &format!("{api}/assets/{id}"),
            "application/octet-stream",
        )?;
        ensure!(
            digest(&api_bytes) == trusted,
            "GitHub manifest SHA-256 mismatch"
        );
        &api_bytes
    };
    let mut check = parse_manifest(bytes, current, releases)?;
    if let Some(update) = &mut check.update {
        update.downloads = probes
            .iter()
            .map(|p| format!("{}{}", p.prefix, update.asset.url))
            .collect();
        if let Some(asset) = metadata
            .as_ref()
            .and_then(|r| r.asset(&filename(&update.version)))
        {
            update.downloads.push(format!("{api}/assets/{}", asset.id));
        }
    }
    Ok(check)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_current_authenticated_channels_are_ranked_by_latency() {
        let probe = |prefix, ms, bytes: &[u8]| Probe {
            prefix,
            elapsed: Duration::from_millis(ms),
            bytes: bytes.to_vec(),
        };
        let mut probes = vec![
            probe(PREFIXES[1], 1, b"old"),
            probe(PREFIXES[2], 5, b"current"),
            probe(PREFIXES[3], 2, b"current"),
            probe(PREFIXES[4], 3, b"modified"),
        ];
        assert!(select(&mut probes, None).is_err());
        select(&mut probes, Some(&digest(b"current"))).unwrap();
        assert_eq!(
            probes.iter().map(|p| p.prefix).collect::<Vec<_>>(),
            [PREFIXES[3], PREFIXES[2]]
        );
        probes.push(probe("", 50, b"current"));
        select(&mut probes, None).unwrap();
        assert_eq!(probes.len(), 3);
    }
}
