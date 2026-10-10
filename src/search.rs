//! Search is delegated to yt-dlp. It is already needed for playback, and it is
//! updated within days whenever YouTube changes its internals.

use crate::model::{is_video_id, Video};
use crate::{tools, util};
use serde::Deserialize;
use std::process::{Command, Output};

#[derive(Deserialize)]
struct Playlist {
    #[serde(default)]
    entries: Vec<Entry>,
}

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

pub fn search(query: &str, limit: usize) -> Result<Vec<Video>, String> {
    // Searches go without the browser sign-in, which would add them to that
    // account's search history, unless YouTube won't answer without one.
    let mut output = run(query, limit, false)?;
    let sign_in = tools::sign_in();
    if !output.status.success() && sign_in.is_some() && util::is_bot_check(&error(&output)) {
        output = run(query, limit, true)?;
    }
    if !output.status.success() {
        let reason = error(&output);
        if let Some(explained) = util::explain_ytdlp(&reason, sign_in.as_ref()) {
            return Err(format!("{}. {}", explained.title, explained.detail));
        }
        return Err(format!("yt-dlp couldn't search: {reason}"));
    }
    parse(&output.stdout)
}

fn run(query: &str, limit: usize, signed_in: bool) -> Result<Output, String> {
    let mut command = Command::new(tools::yt_dlp());
    command.args(["--flat-playlist", "--dump-single-json", "--no-warnings", "--quiet"]);
    if signed_in {
        command.args(tools::sign_in_args());
    }
    command.arg(format!("ytsearch{limit}:{query}")).output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "Search needs yt-dlp. Install it from your package manager, then search again.".to_string()
        } else {
            format!("Couldn't run yt-dlp: {e}")
        }
    })
}

/// yt-dlp's explanation: the last thing it said.
fn error(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let last = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no error message");
    util::tidy_error(last)
}

pub fn parse(json: &[u8]) -> Result<Vec<Video>, String> {
    let list: Playlist =
        serde_json::from_slice(json).map_err(|e| format!("Unexpected output from yt-dlp: {e}"))?;
    Ok(list.entries.into_iter().filter_map(to_video).collect())
}

/// One line of yt-dlp's `--flat-playlist --dump-json` output, as a video of
/// the given channel.
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
    #[test]
    fn parses_flat_search() {
        let json = br#"{"_type":"playlist","entries":[
          {"_type":"url","ie_key":"Youtube","id":"dQw4w9WgXcQ","url":"https://www.youtube.com/watch?v=dQw4w9WgXcQ",
           "title":"Lathe build","channel":"Tom","channel_id":"UCabcdefghijklmnopqrstuv","duration":754.0,"view_count":1234},
          {"_type":"url","ie_key":"YoutubeTab","id":"UCabcdefghijklmnopqrstuv","title":"Tom"},
          {"_type":"url","ie_key":"Youtube","id":"abcdefghijk","url":"https://www.youtube.com/shorts/abcdefghijk",
           "title":"Short one","uploader":"Ann","duration":null,"view_count":null}
        ]}"#;
        let v = super::parse(json).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].duration, Some(754));
        assert_eq!(v[0].views, Some(1234));
        assert_eq!(v[1].channel_name, "Ann");
        assert!(v[1].is_short);
    }
}
