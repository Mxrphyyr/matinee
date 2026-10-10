//! Running yt-dlp for a list of videos (a search, a channel's uploads) and
//! reading its entries as they arrive, so the first ones show while the rest
//! are still coming.
//!
//! A list can be read a page at a time: while Matinee isn't reading, yt-dlp's
//! output pipe fills and it stops fetching too, so a long channel costs
//! YouTube a few requests rather than hundreds. Bursts of requests are what
//! make YouTube start asking whether you're a bot.

use crate::model::Video;
use crate::{search, tools, util};
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

pub enum Update {
    /// The next videos.
    Videos(Vec<Video>),
    /// yt-dlp stopped with this error; videos that arrived before it stay.
    Failed(String),
}

pub struct Stream {
    child: Child,
    more: mpsc::Sender<()>,
    /// yt-dlp was given the browser sign-in.
    pub signed_in: bool,
}

impl Stream {
    /// Reads the next page.
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

/// Starts yt-dlp on `target` (a search term or a channel address) with the
/// extra `args`, reading its entries as videos of `channel` (id and name,
/// for entries that don't say). With a `page` size, reading pauses after
/// each page until `Stream::more` is called. The browser sign-in is used
/// only when asked for, after YouTube refused to answer without it.
pub fn start(
    args: &[&str],
    target: &str,
    channel: (&str, &str),
    page: Option<usize>,
    signed_in: bool,
) -> Result<(Stream, async_channel::Receiver<Update>), String> {
    let mut command = Command::new(tools::yt_dlp());
    command.args(["--flat-playlist", "--lazy-playlist", "--dump-json", "--no-warnings"]).args(args);
    if signed_in {
        command.args(tools::sign_in_args());
    }
    let mut child = command
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "This needs yt-dlp. Install it from your package manager, then try again.".to_string()
            } else {
                format!("Couldn't run yt-dlp: {e}")
            }
        })?;
    let stdout = child.stdout.take().ok_or("yt-dlp gave no output")?;
    // yt-dlp's explanation, should it fail. Read alongside the videos so
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
    // One thread reads lines, handing each over only when the next thread
    // is ready for it: while that one waits (for the viewer to scroll
    // nearer the end), the pipe fills and yt-dlp pauses.
    let (line_tx, line_rx) = mpsc::sync_channel::<String>(0);
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line_tx.send(line).is_err() {
                break;
            }
        }
    });
    let (tx, rx) = async_channel::unbounded();
    let (more_tx, more_rx) = mpsc::channel();
    let (id, name) = (channel.0.to_string(), channel.1.to_string());
    std::thread::spawn(move || {
        let mut batch = Vec::new();
        let (mut read, mut allowed) = (0, page.unwrap_or(usize::MAX));
        loop {
            // Entries that arrive together (a page from YouTube) go out
            // together; a lull sends whatever has come.
            match line_rx.recv_timeout(LULL) {
                Ok(line) => {
                    if let Some(video) = search::parse_line(&line, &id, &name) {
                        batch.push(video);
                    }
                    read += 1;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if !batch.is_empty() && tx.send_blocking(Update::Videos(std::mem::take(&mut batch))).is_err() {
                        return;
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            let page_done = read >= allowed;
            if !batch.is_empty()
                && (batch.len() >= 30 || page_done)
                && tx.send_blocking(Update::Videos(std::mem::take(&mut batch))).is_err()
            {
                return;
            }
            if page_done {
                // Wait until the viewer scrolls near the end. The stream
                // ending stops us here.
                if more_rx.recv().is_err() {
                    return;
                }
                allowed += page.unwrap_or(0);
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

/// How long without a new entry before the ones in hand are passed on.
const LULL: Duration = Duration::from_millis(80);
