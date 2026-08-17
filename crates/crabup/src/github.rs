//! Download prebuilt binaries from GitHub releases.

use std::io::Read;

use anyhow::{Context, Result, bail};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use crate::registry::Entry;

const REPO: &str = "crabtalk/crabtalk";

/// Detect the current platform in Makefile naming convention (e.g. `macos-arm64`).
fn detect_platform() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("macos-arm64"),
        ("macos", "x86_64") => Ok("macos-amd64"),
        ("linux", "aarch64") => Ok("linux-arm64"),
        ("linux", "x86_64") => Ok("linux-amd64"),
        ("windows", "x86_64") => Ok("windows-amd64"),
        (os, arch) => bail!("no prebuilt binary for {os}-{arch}"),
    }
}

/// Fetch the latest release tag from GitHub.
pub fn latest_version() -> Result<String> {
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;

    let url = format!("https://github.com/{REPO}/releases/latest");
    let resp = client.get(&url).send()?;

    if resp.status().is_redirection()
        && let Some(location) = resp.headers().get("location")
    {
        let location = location.to_str().context("non-UTF-8 redirect")?;
        if let Some(tag) = location.rsplit('/').next() {
            return Ok(tag.to_string());
        }
    }

    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let resp: serde_json::Value = reqwest::blocking::get(&url)?.json()?;
    resp["tag_name"]
        .as_str()
        .map(String::from)
        .context("could not determine latest version from GitHub")
}

/// Download a URL with a progress bar, return the bytes.
fn download(url: &str, prefix: &str, mp: &MultiProgress) -> Result<Vec<u8>> {
    let resp = reqwest::blocking::get(url).context("download failed")?;
    if !resp.status().is_success() {
        bail!("{prefix}: HTTP {}", resp.status());
    }

    let total = resp.content_length().unwrap_or(0);
    let style = ProgressStyle::with_template(
        "{prefix:>12} [{bar:20.cyan/dim}] {bytes:>10}/{total_bytes:<10} ({bytes_per_sec}, ETA: {eta})",
    )
    .unwrap()
    .progress_chars("=> ");

    let pb = mp.add(ProgressBar::new(total));
    pb.set_style(style);
    pb.set_prefix(prefix.to_string());

    let mut buf = Vec::with_capacity(total as usize);
    let mut reader = pb.wrap_read(resp);
    reader.read_to_end(&mut buf)?;

    pb.finish_with_message("done");
    Ok(buf)
}

/// Extract the named binary from a gzipped tarball into BIN_DIR.
fn extract_binary(tarball: &[u8], bin_name: &str) -> Result<()> {
    let bin_dir = &*wcore::paths::BIN_DIR;
    std::fs::create_dir_all(bin_dir)?;

    let gz = flate2::read::GzDecoder::new(tarball);
    let mut archive = tar::Archive::new(gz);

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if file_name == bin_name {
            let dest = bin_dir.join(bin_name);
            let mut file = std::fs::File::create(&dest)?;
            std::io::copy(&mut entry, &mut file)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
            }
            return Ok(());
        }
    }

    bail!("binary '{bin_name}' not found in tarball")
}

/// Install one or more entries from GitHub releases.
///
/// Uses a shared `MultiProgress` so all download bars render together,
/// like rustup's component download display.
pub fn install(entries: &[&Entry], version: Option<&str>) -> Result<()> {
    let platform = detect_platform()?;
    let version = match version {
        Some(v) => v.to_string(),
        None => {
            println!("info: checking latest version...");
            latest_version()?
        }
    };

    let mp = MultiProgress::new();
    println!("info: downloading {} components", entries.len());

    for entry in entries {
        let url = format!(
            "https://github.com/{REPO}/releases/download/{version}/{bin}-{version}-{platform}.tar.gz",
            bin = entry.bin,
        );
        let tarball = download(&url, entry.bin, &mp)?;
        extract_binary(&tarball, entry.bin)?;
        crate::manifest::record(entry.short, &version)?;
    }

    println!("info: installed to {}", wcore::paths::BIN_DIR.display());
    Ok(())
}
