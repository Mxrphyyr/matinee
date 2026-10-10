//! A channel's whole upload history. RSS only lists a channel's 15 newest
//! videos, so the channel page asks yt-dlp for the channel's Videos tab,
//! a page at a time as you scroll toward the end (see `ytdlp`).

use crate::ytdlp::{self, Stream, Update};

/// Uploads read per request for more (about two of YouTube's pages).
const PAGE: usize = 60;
/// Enough for nearly every channel's back catalogue.
const LIMIT: usize = 5000;

/// Starts reading a channel's older uploads. The browser sign-in is used
/// only when asked for, after YouTube refused to answer without it.
pub fn start(
    channel_id: &str,
    channel_name: &str,
    signed_in: bool,
) -> Result<(Stream, async_channel::Receiver<Update>), String> {
    let limit = LIMIT.to_string();
    ytdlp::start(
        &[
            "--ignore-errors",
            // Turns "3 weeks ago" into a date, so older videos still show their age.
            "--extractor-args",
            "youtubetab:approximate_date",
            // A pause between page requests, so reading a channel is never a burst.
            "--sleep-requests",
            "1",
            "--playlist-end",
            &limit,
        ],
        &format!("https://www.youtube.com/channel/{channel_id}/videos"),
        (channel_id, channel_name),
        Some(PAGE),
        signed_in,
    )
}
