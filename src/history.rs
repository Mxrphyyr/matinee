//! A channel's whole upload history. RSS only lists a channel's 15 newest
//! videos, so the channel page asks yt-dlp for the channel's Videos tab.
//!
//! Entries are read only as you scroll toward them. While Matinee isn't
//! reading, yt-dlp's output pipe fills and it stops fetching too, and it
//! pauses between the pages it does fetch. So a channel costs YouTube a few
//! requests rather than hundreds: bursts of requests are what make YouTube
//! start asking whether you're a bot.

use crate::model::Video;
use crate::{search, tools};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Uploads read per request for more (about two of YouTube's pages).
const PAGE: usize = 60;
/// Enough for nearly every channel's back catalogue.
const LIMIT: usize = 5000;

pub struct Stream {
    child: Child,
    more: mpsc::Sender<()>,
}

impl Stream {
    /// Reads the next page of older uploads.
    pub fn more(&self) {
        let _ = self.more.send(());
    }

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
        // A pause between page requests, so reading a channel is never a burst.
        .args(["--sleep-requests", "1"])
        .args(["--playlist-end", &limit])
        .args(tools::yt_dlp_args())
        .arg(format!("https://www.youtube.com/channel/{channel_id}/videos"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Couldn't run yt-dlp for older videos: {e}"))?;
    let stdout = child.stdout.take().ok_or("yt-dlp gave no output")?;
    let (tx, rx) = async_channel::unbounded();
    let (more_tx, more_rx) = mpsc::channel();
    let (id, name) = (channel_id.to_string(), channel_name.to_string());
    std::thread::spawn(move || {
        let mut batch = Vec::new();
        let mut sent = Instant::now();
        let (mut read, mut allowed) = (0, PAGE);
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Some(video) = search::parse_line(&line, &id, &name) {
                batch.push(video);
            }
            read += 1;
            let page_done = read >= allowed;
            let due = batch.len() >= 30 || page_done || sent.elapsed() > Duration::from_millis(400);
            if due && !batch.is_empty() {
                if tx.send_blocking(std::mem::take(&mut batch)).is_err() {
                    return;
                }
                sent = Instant::now();
            }
            if page_done {
                // Wait until the viewer scrolls near the end; meanwhile yt-dlp
                // fills the pipe and pauses. The stream ending stops us here.
                if more_rx.recv().is_err() {
                    return;
                }
                allowed += PAGE;
            }
        }
        if !batch.is_empty() {
            let _ = tx.send_blocking(batch);
        }
    });
    Ok((Stream { child, more: more_tx }, rx))
}
