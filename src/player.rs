//! Playback happens in mpv, which streams through yt-dlp. Neither loads
//! YouTube's web player, so there are no ads and no watch-time reporting.
//! Set MATINEE_PLAYER (e.g. "celluloid" or "mpv --fs") to use something else;
//! the video URL is appended as the last argument.

use crate::model::Video;
use std::process::{Child, Command, Stdio};

fn custom() -> Option<String> {
    std::env::var("MATINEE_PLAYER").ok().filter(|c| !c.trim().is_empty())
}

pub fn name() -> String {
    custom()
        .and_then(|c| c.split_whitespace().next().map(|p| p.rsplit('/').next().unwrap_or(p).to_string()))
        .unwrap_or_else(|| "mpv".into())
}

pub fn spawn(video: &Video, start: Option<f64>) -> Result<Child, String> {
    let mut cmd = match custom() {
        Some(line) => {
            let mut parts = line.split_whitespace();
            let mut cmd = Command::new(parts.next().unwrap_or("mpv"));
            cmd.args(parts);
            cmd
        }
        None => {
            let mut cmd = Command::new(crate::tools::find("mpv").unwrap_or_else(|| "mpv".into()));
            cmd.arg("--force-window=immediate")
                // mpv expands ${…} in titles; double any $ so titles show literally.
                .arg(format!("--title={}", video.title.replace('$', "$$")))
                .arg("--ytdl-format=bestvideo[height<=?1080]+bestaudio/best");
            if let Some(start) = start.filter(|s| *s > 1.0) {
                cmd.arg(format!("--start={start:.1}"));
            }
            // Launchers can leave ~/.local/bin off PATH; tell mpv where yt-dlp is.
            if let Some(path) = crate::tools::find("yt-dlp") {
                cmd.arg(format!("--script-opts=ytdl_hook-ytdl_path={}", path.display()));
            }
            cmd
        }
    };
    cmd.arg(video.watch_url())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("{} isn't installed. Install it to play videos.", name())
        } else {
            format!("Couldn't start {}: {e}", name())
        }
    })
}

/// Blocks until the player exits. mpv exits with 1–3 when it couldn't play.
pub fn wait(mut child: Child) -> Result<(), String> {
    let status = child.wait().map_err(|e| e.to_string())?;
    if custom().is_none() && matches!(status.code(), Some(1..=3)) {
        return Err("mpv couldn't play that video. Updating yt-dlp usually fixes this.".into());
    }
    Ok(())
}
