//! Finding helper programs. Apps opened from a desktop launcher often get a
//! shorter PATH than a terminal, so ~/.local/bin is always checked too.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub fn find(program: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join("bin"));
    }
    dirs.push(PathBuf::from("/usr/local/bin"));
    dirs.into_iter()
        .map(|dir| dir.join(program))
        .find(|path| is_executable(path))
}

fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// yt-dlp's full path, or its bare name so error messages still read well.
pub fn yt_dlp() -> PathBuf {
    find("yt-dlp").unwrap_or_else(|| PathBuf::from("yt-dlp"))
}
