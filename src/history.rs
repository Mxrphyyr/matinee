//! A channel's whole upload history. RSS only lists a channel's 15 newest
//! videos, so the channel page asks yt-dlp to walk the channel's Videos tab
//! and streams entries as they're read: the first ones appear within a second
//! or two while older ones keep arriving.

use crate::model::Video;
use crate::{search, tools};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Enough for nearly every channel's back catalogue.
const LIMIT: usize = 5000;

pub struct Stream {
    child: Child,
}

impl Stream {
    /// Stops reading (if still running) and reaps the process.
    pub fn stop(mut self) {
        let _ = self.child.kill();
        std::thread::spawn(move || {
            let _ = self.child.wait();
        });
    }
}

pub fn start(channel_id: &str, channel_name: &str) -> Result<(Stream, async_channel::Receiver<Vec<Video>>), String> {
    let limit = LIMIT.to_string();
    let mut child = Command::new(tools::yt_dlp())
        .args(["--flat-playlist", "--lazy-playlist", "--dump-json", "--no-warnings", "--ignore-errors"])
        // Turns "3 weeks ago" into a date, so older videos still show their age.
        .args(["--extractor-args", "youtubetab:approximate_date"])
        .args(["--playlist-end", &limit])
        .arg(format!("https://www.youtube.com/channel/{channel_id}/videos"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Couldn't run yt-dlp for older videos: {e}"))?;
    let stdout = child.stdout.take().ok_or("yt-dlp gave no output")?;
    let (tx, rx) = async_channel::unbounded();
    let (id, name) = (channel_id.to_string(), channel_name.to_string());
    std::thread::spawn(move || {
        let mut batch = Vec::new();
        let mut sent = Instant::now();
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Some(video) = search::parse_line(&line, &id, &name) {
                batch.push(video);
            }
            if batch.len() >= 60 || (!batch.is_empty() && sent.elapsed() > Duration::from_millis(400)) {
                if tx.send_blocking(std::mem::take(&mut batch)).is_err() {
                    return;
                }
                sent = Instant::now();
            }
        }
        if !batch.is_empty() {
            let _ = tx.send_blocking(batch);
        }
    });
    Ok((Stream { child }, rx))
}
