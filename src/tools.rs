//! Finding helper programs, and the options every yt-dlp run shares. Apps
//! opened from a desktop launcher often get a shorter PATH than a terminal,
//! so ~/.local/bin is always checked too.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// The browser whose YouTube sign-in yt-dlp may borrow, when that's turned on.
static COOKIES_FROM: RwLock<Option<String>> = RwLock::new(None);

pub fn find(program: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(home) = home() {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join("bin"));
    }
    dirs.push(PathBuf::from("/usr/local/bin"));
    dirs.into_iter()
        .map(|dir| dir.join(program))
        .find(|path| is_executable(path))
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// yt-dlp, preferring a copy in ~/.local/bin: that's the one people keep up
/// to date themselves (`yt-dlp -U`), and YouTube breaks old versions often.
pub fn find_yt_dlp() -> Option<PathBuf> {
    home()
        .map(|home| home.join(".local/bin/yt-dlp"))
        .filter(|path| is_executable(path))
        .or_else(|| find("yt-dlp"))
}

/// yt-dlp's full path, or its bare name so error messages still read well.
pub fn yt_dlp() -> PathBuf {
    find_yt_dlp().unwrap_or_else(|| PathBuf::from("yt-dlp"))
}

pub fn set_cookies_browser(browser: Option<String>) {
    if let Ok(mut slot) = COOKIES_FROM.write() {
        *slot = browser;
    }
}

fn cookies_browser() -> Option<String> {
    COOKIES_FROM.read().ok().and_then(|slot| slot.clone())
}

/// Extra arguments for every yt-dlp run: the browser sign-in, when it's on.
pub fn yt_dlp_args() -> Vec<String> {
    match cookies_browser() {
        Some(browser) => vec!["--cookies-from-browser".to_string(), browser],
        None => Vec::new(),
    }
}

/// The same, in the form mpv's `ytdl-raw-options` expects.
pub fn mpv_raw_options() -> String {
    cookies_browser().map(|browser| format!("cookies-from-browser={browser}")).unwrap_or_default()
}
