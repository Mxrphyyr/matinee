//! A channel's whole upload history. RSS only lists a channel's 15 newest
//! videos, so the channel page asks yt-dlp for the channel's Videos tab.
//!
//! Entries are read only as you scroll toward them. While Matinee isn't
//! reading, yt-dlp's output pipe fills and it stops fetching too, and it
//! pauses between the pages it does fetch. So a channel costs YouTube a few
//! requests rather than hundreds: bursts of requests are what make YouTube
//! start asking whether you're a bot.

use crate::model::Video;
use crate::{search, tools, util};
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Uploads read per request for more (about two of YouTube's pages).
const PAGE: usize = 60;
/// Enough for nearly every channel's back catalogue.
const LIMIT: usize = 5000;

pub enum Update {
    /// The next older uploads.
    Videos(Vec<Video>),
    /// yt-dlp stopped with this error; uploads that arrived before it stay.
    Failed(String),
}

pub struct Stream {
    child: Child,
    more: mpsc::Sender<()>,
    /// yt-dlp was given the browser sign-in.
    pub signed_in: bool,
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

/// Starts reading a channel's older uploads. The browser sign-in is used
/// only when asked for, after YouTube refused to answer without it.
pub fn start(
    channel_id: &str,
    channel_name: &str,
    signed_in: bool,
) -> Result<(Stream, async_channel::Receiver<Update>), String> {
    let limit = LIMIT.to_string();
    let mut command = Command::new(tools::yt_dlp());
    command
        .args(["--flat-playlist", "--lazy-playlist", "--dump-json", "--no-warnings", "--ignore-errors"])
        // Turns "3 weeks ago" into a date, so older videos still show their age.
        .args(["--extractor-args", "youtubetab:approximate_date"])
        // A pause between page requests, so reading a channel is never a burst.
        .args(["--sleep-requests", "1"])
        .args(["--playlist-end", &limit]);
    if signed_in {
        command.args(tools::sign_in_args());
    }
    let mut child = command
        .arg(format!("https://www.youtube.com/channel/{channel_id}/videos"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Couldn't run yt-dlp for older videos: {e}"))?;
    let stdout = child.stdout.take().ok_or("yt-dlp gave no output")?;
    // yt-dlp's explanation, should it fail. Read alongside the uploads so
    // that pipe never fills up either.
    let stderr = child.stderr.take();
    let errors = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut stderr) = stderr {
            let _ = stderr.read_to_end(&mut bytes);
        }
        let text = String::from_utf8_lossy(&bytes);
        text.lines().rev().find(|line| line.contains("ERROR:")).map(util::tidy_error)
    });
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
                if tx.send_blocking(Update::Videos(std::mem::take(&mut batch))).is_err() {
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
            let _ = tx.send_blocking(Update::Videos(batch));
        }
        if let Ok(Some(reason)) = errors.join() {
            let _ = tx.send_blocking(Update::Failed(reason));
        }
    });
    Ok((Stream { child, more: more_tx, signed_in }, rx))
}
