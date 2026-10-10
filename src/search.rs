//! Search is delegated to yt-dlp. It is already needed for playback, and it is
//! updated within days whenever YouTube changes its internals. Results are
//! read as they arrive (see `ytdlp`), so the first ones show within a second
//! or two.

use crate::model::{is_video_id, Video};
use crate::ytdlp::{self, Stream, Update};
use serde::Deserialize;

#[derive(Deserialize)]
struct Entry {
    id: Option<String>,
    title: Option<String>,
    url: Option<String>,
    ie_key: Option<String>,
    channel: Option<String>,
    uploader: Option<String>,
    channel_id: Option<String>,
    // Numbers are read as f64 so one odd value can't sink the whole response.
    duration: Option<f64>,
    view_count: Option<f64>,
    timestamp: Option<f64>,
}

/// Starts a search for up to `limit` videos. Searches go without the
/// browser sign-in, which would add them to that account's search history,
/// unless YouTube refused to answer without one (`signed_in`).
pub fn start(query: &str, limit: usize, signed_in: bool) -> Result<(Stream, async_channel::Receiver<Update>), String> {
    ytdlp::start(&[], &format!("ytsearch{limit}:{query}"), ("", ""), None, signed_in)
}

/// One line of yt-dlp's `--flat-playlist --dump-json` output, as a video of
/// the given channel (for entries that don't name theirs).
pub fn parse_line(line: &str, channel_id: &str, channel_name: &str) -> Option<Video> {
    let entry: Entry = serde_json::from_str(line).ok()?;
    let mut video = to_video(entry)?;
    if video.channel_id.is_empty() {
        video.channel_id = channel_id.to_string();
    }
    if video.channel_name.is_empty() {
        video.channel_name = channel_name.to_string();
    }
    Some(video)
}

fn to_video(e: Entry) -> Option<Video> {
    // Searches can also return channels and playlists; keep videos only.
    if e.ie_key.as_deref().is_some_and(|k| k != "Youtube") {
        return None;
    }
    let id = e.id.filter(|id| is_video_id(id))?;
    let url = e.url.unwrap_or_default();
    Some(Video {
        title: e.title.unwrap_or_default(),
        channel_name: e.channel.or(e.uploader).unwrap_or_default(),
        channel_id: e.channel_id.unwrap_or_default(),
        published: e.timestamp.map(|t| t as i64),
        views: e.view_count.map(|v| v.max(0.0) as u64),
        duration: e.duration.map(|d| d.round() as u32).filter(|d| *d > 0),
        is_short: url.contains("/shorts/"),
        id,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_line;

    #[test]
    fn parses_flat_entries() {
        let video = parse_line(
            r#"{"_type":"url","ie_key":"Youtube","id":"dQw4w9WgXcQ","url":"https://www.youtube.com/watch?v=dQw4w9WgXcQ",
               "title":"Lathe build","channel":"Tom","channel_id":"UCabcdefghijklmnopqrstuv","duration":754.0,"view_count":1234}"#,
            "",
            "",
        )
        .unwrap();
        assert_eq!(video.duration, Some(754));
        assert_eq!(video.views, Some(1234));
        assert_eq!(video.channel_name, "Tom");

        // Searches also return channels and playlists; those aren't videos.
        assert!(parse_line(r#"{"_type":"url","ie_key":"YoutubeTab","id":"UCabcdefghijklmnopqrstuv","title":"Tom"}"#, "", "").is_none());

        // A channel's own listing leaves the channel out; the caller supplies it.
        let short = parse_line(
            r#"{"_type":"url","ie_key":"Youtube","id":"abcdefghijk","url":"https://www.youtube.com/shorts/abcdefghijk",
               "title":"Short one","duration":null,"view_count":null}"#,
            "UCabcdefghijklmnopqrstuv",
            "Ann",
        )
        .unwrap();
        assert_eq!(short.channel_name, "Ann");
        assert_eq!(short.channel_id, "UCabcdefghijklmnopqrstuv");
        assert!(short.is_short);
        assert!(parse_line("not json", "", "").is_none());
    }
}
