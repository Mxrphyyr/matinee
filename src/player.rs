//! Playback happens in mpv, which streams through yt-dlp. Neither loads
//! YouTube's web player, so there are no ads and no watch-time reporting.
//! Set MATINEE_PLAYER (e.g. "celluloid" or "mpv --fs") to use something else;
//! the video URL is appended as the last argument.

use crate::model::Video;
use crate::{tools, util};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

fn custom() -> Option<String> {
    std::env::var("MATINEE_PLAYER").ok().filter(|c| !c.trim().is_empty())
}

pub fn name() -> String {
    custom()
        .and_then(|c| c.split_whitespace().next().map(|p| p.rsplit('/').next().unwrap_or(p).to_string()))
        .unwrap_or_else(|| "mpv".into())
}

/// Plays `video` in its own window, from `start` seconds in, streaming at
/// most `height` pixels tall.
pub fn spawn(video: &Video, start: Option<f64>, height: u32) -> Result<Child, String> {
    let mut cmd = match custom() {
        Some(line) => {
            let mut parts = line.split_whitespace();
            let mut cmd = Command::new(parts.next().unwrap_or("mpv"));
            cmd.args(parts).stdout(Stdio::null());
            cmd
        }
        None => {
            let mut cmd = Command::new(tools::find("mpv").unwrap_or_else(|| "mpv".into()));
            cmd.arg("--force-window=immediate")
                // mpv expands ${…} in titles; double any $ so titles show literally.
                .arg(format!("--title={}", video.title.replace('$', "$$")))
                .arg(format!("--ytdl-format={}", tools::ytdl_format(height)))
                // Only problems are worth reading (see `wait`).
                .arg("--msg-level=all=error");
            if let Some(start) = start.filter(|s| *s > 1.0) {
                cmd.arg(format!("--start={start:.1}"));
            }
            // Launchers can leave ~/.local/bin off PATH; tell mpv where yt-dlp is.
            if let Some(path) = tools::find_yt_dlp() {
                cmd.arg(format!("--script-opts=ytdl_hook-ytdl_path={}", path.display()));
            }
            // The browser sign-in, if it's turned on in Preferences, and the
            // subtitles to list.
            cmd.arg(format!("--ytdl-raw-options={}", tools::mpv_ytdl_options()));
            // mpv writes its messages here; `wait` reads them.
            cmd.stdout(Stdio::piped());
            cmd
        }
    };
    cmd.arg(video.watch_url()).stdin(Stdio::null()).stderr(Stdio::null());
    cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("{} isn't installed. Install it to play videos.", name())
        } else {
            format!("Couldn't start {}: {e}", name())
        }
    })
}

/// Blocks until the player exits. mpv exits with 1–3 when it couldn't play;
/// the error is then yt-dlp's explanation, or empty when there was none.
pub fn wait(mut child: Child) -> Result<(), String> {
    // Read mpv's messages as they come, so it never waits on a full pipe.
    let mut error = String::new();
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line.contains("ERROR:") {
                error = util::tidy_error(&line);
            }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if custom().is_none() && matches!(status.code(), Some(1..=3)) {
        return Err(error);
    }
    Ok(())
}
