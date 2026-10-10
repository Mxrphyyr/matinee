//! Keeping yt-dlp up to date from inside Matinee. YouTube changes often and
//! an old yt-dlp soon stops playing videos, so Preferences (and the player,
//! when a video won't play) offer an update.
//!
//! A standalone yt-dlp in ~/.local/bin updates itself (`yt-dlp -U`, which
//! checks the download against the release's published checksums). With
//! yt-dlp from the system's packages, or none at all, Matinee downloads
//! yt-dlp's standalone build into ~/.local/bin, checks it the same way, and
//! uses that copy from then on (see `tools::find_yt_dlp`). Nothing outside
//! ~/.local is touched, and the requests go to GitHub, never to YouTube.

use crate::{net, tools, util};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const RELEASES: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download";

pub enum Outcome {
    /// Already the newest version.
    UpToDate(String),
    Updated(String),
    /// A standalone copy was put in ~/.local/bin.
    Installed(String),
}

impl Outcome {
    /// yt-dlp changed, so whatever failed with the old one is worth another try.
    pub fn changed(&self) -> bool {
        !matches!(self, Outcome::UpToDate(_))
    }
}

/// The yt-dlp Matinee uses.
pub struct Found {
    pub version: String,
    pub path: PathBuf,
    /// In ~/.local/bin, where it can update itself.
    pub local: bool,
}

/// Which yt-dlp Matinee uses, and its version. Blocking (it runs yt-dlp).
pub fn found() -> Option<Found> {
    let path = tools::find_yt_dlp()?;
    let version = version_of(&path)?;
    let local = local_path().is_some_and(|local| local == path);
    Some(Found { version, path, local })
}

/// One line about the yt-dlp in use, for Preferences.
pub fn summary(found: Option<&Found>) -> String {
    match found {
        Some(found) if found.local => format!("Version {}, in ~/.local/bin", found.version),
        Some(found) => format!(
            "Version {}, from {}. Updating puts an up-to-date copy in ~/.local/bin, which Matinee then uses.",
            found.version,
            found.path.display()
        ),
        None => "Not installed. Installing puts it in ~/.local/bin.".to_string(),
    }
}

/// How an update went, in plain words.
pub fn describe(result: &Result<Outcome, String>) -> String {
    match result {
        Ok(Outcome::UpToDate(version)) => format!("yt-dlp is up to date ({version})"),
        Ok(Outcome::Updated(version)) => format!("Updated yt-dlp to {version}"),
        Ok(Outcome::Installed(version)) => {
            format!("Installed yt-dlp {version} in ~/.local/bin; Matinee uses it from now on")
        }
        Err(message) => message.clone(),
    }
}

/// Brings yt-dlp up to date. Blocking, and may download about 40 MB: run
/// it off the UI thread.
pub fn update() -> Result<Outcome, String> {
    let local = local_path().ok_or("Couldn't find your home folder")?;
    if tools::is_executable(&local) {
        self_update(&local)
    } else {
        install(&local)
    }
}

fn local_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/bin/yt-dlp"))
}

fn version_of(path: &Path) -> Option<String> {
    let output = Command::new(path).arg("--version").output().ok()?;
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !version.is_empty()).then_some(version)
}

/// `yt-dlp -U`: yt-dlp's own updater, for its standalone build.
fn self_update(path: &Path) -> Result<Outcome, String> {
    let before = version_of(path);
    let output = Command::new(path).arg("-U").output().map_err(|e| format!("Couldn't run yt-dlp: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr
            .lines()
            .rev()
            .find(|line| line.contains("ERROR:"))
            .map(util::tidy_error)
            .unwrap_or_else(|| "it stopped without saying why".to_string());
        return Err(explain_failure(&reason));
    }
    let after = version_of(path).ok_or("yt-dlp didn't start after updating. Try updating again.")?;
    if before.as_deref() == Some(after.as_str()) {
        Ok(Outcome::UpToDate(after))
    } else {
        Ok(Outcome::Updated(after))
    }
}

fn explain_failure(reason: &str) -> String {
    let lower = reason.to_lowercase();
    if ["package manager", "pip", "source code", "unofficial build", "unpackaged"].iter().any(|w| lower.contains(w)) {
        "The yt-dlp in ~/.local/bin was installed another way (with pip, say), so it can't update itself. \
         Update it the way it was installed, or delete it and update here to get yt-dlp's own build."
            .to_string()
    } else if ["unable to download", "unable to obtain", "unable to fetch", "network", "resolve", "timed out"]
        .iter()
        .any(|w| lower.contains(w))
    {
        "Couldn't reach GitHub to update yt-dlp. Check your connection, then try again.".to_string()
    } else {
        format!("yt-dlp couldn't update: {reason}")
    }
}

/// Downloads yt-dlp's standalone build into ~/.local/bin, after checking it
/// against the release's published checksums.
fn install(target: &Path) -> Result<Outcome, String> {
    let asset = match std::env::consts::ARCH {
        "x86_64" => "yt-dlp_linux",
        "aarch64" => "yt-dlp_linux_aarch64",
        other => {
            return Err(format!(
                "yt-dlp has no ready-made build for this kind of computer ({other}). Install it with your package manager."
            ))
        }
    };
    let offline = |e: String| format!("Couldn't reach GitHub to download yt-dlp ({e}). Check your connection, then try again.");
    let sums = net::fetch_text(&format!("{RELEASES}/SHA2-256SUMS")).map_err(offline)?;
    let expected = expected_sum(&sums, asset).ok_or("yt-dlp's latest release has no checksum for its Linux build. Try again later.")?;
    let dir = target.parent().ok_or("No folder for yt-dlp")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    let partial = dir.join(".yt-dlp.download");
    let actual = net::download_to(&format!("{RELEASES}/{asset}"), &partial).map_err(|e| {
        let _ = std::fs::remove_file(&partial);
        offline(e)
    })?;
    let fail = |message: &str| {
        let _ = std::fs::remove_file(&partial);
        message.to_string()
    };
    if actual != expected {
        return Err(fail("The downloaded yt-dlp didn't match its published checksum, so it wasn't installed. Try again later."));
    }
    std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| fail(&format!("Couldn't make yt-dlp runnable: {e}")))?;
    let version = version_of(&partial).ok_or_else(|| fail("The downloaded yt-dlp doesn't run on this computer."))?;
    std::fs::rename(&partial, target).map_err(|e| fail(&format!("Couldn't put yt-dlp in place: {e}")))?;
    Ok(Outcome::Installed(version))
}

/// The SHA-256 a `SHA2-256SUMS` file lists for `asset`.
fn expected_sum(sums: &str, asset: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let (sum, name) = (parts.next()?, parts.next()?);
        (name.trim_start_matches('*') == asset && sum.len() == 64 && sum.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| sum.to_lowercase())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_checksum_for_the_linux_build() {
        let a = "a".repeat(64);
        let b = "B".repeat(64);
        let sums = format!("{a}  yt-dlp\n{b}  yt-dlp_linux\n{a}  yt-dlp_linux_aarch64\nnonsense\n");
        assert_eq!(expected_sum(&sums, "yt-dlp_linux"), Some("b".repeat(64)));
        assert_eq!(expected_sum(&sums, "yt-dlp_linux_aarch64"), Some(a.clone()));
        assert_eq!(expected_sum(&format!("{a} *yt-dlp_linux"), "yt-dlp_linux"), Some(a));
        assert_eq!(expected_sum("short  yt-dlp_linux", "yt-dlp_linux"), None);
        assert_eq!(expected_sum(&sums, "yt-dlp_macos"), None);
    }

    #[test]
    fn explains_failures_in_plain_words() {
        assert!(explain_failure("You installed yt-dlp with pip or using the wheel from PyPi; Use that to update")
            .contains("can't update itself"));
        assert!(explain_failure("Unable to obtain version info (<urlopen error [Errno -3]>)").contains("GitHub"));
        assert!(explain_failure("Something new").starts_with("yt-dlp couldn't update: Something new"));
    }

    #[test]
    fn describes_outcomes() {
        assert_eq!(describe(&Ok(Outcome::UpToDate("2026.08.19".into()))), "yt-dlp is up to date (2026.08.19)");
        assert!(describe(&Ok(Outcome::Installed("2026.09.27".into()))).contains("~/.local/bin"));
        assert!(!Outcome::UpToDate(String::new()).changed() && Outcome::Updated(String::new()).changed());
    }
}
