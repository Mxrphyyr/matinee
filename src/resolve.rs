//! Turning whatever the user pastes or imports into channel ids.

use crate::model::{is_channel_id, is_id_byte, Channel};
use crate::{feed, net};
use std::process::Command;

/// Every `UC…` channel id embedded in arbitrary text, in order of appearance.
pub fn find_channel_ids(text: &str) -> Vec<String> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 24 <= b.len() {
        let starts = b[i] == b'U' && b[i + 1] == b'C' && (i == 0 || !is_id_byte(b[i - 1]));
        let ends = i + 24 == b.len() || !is_id_byte(b[i + 24]);
        if starts && ends && b[i..i + 24].iter().all(|&c| is_id_byte(c)) {
            out.push(String::from_utf8_lossy(&b[i..i + 24]).into_owned());
            i += 24;
        } else {
            i += 1;
        }
    }
    out
}

pub fn find_channel_id(text: &str) -> Option<String> {
    find_channel_ids(text).into_iter().next()
}

/// Resolves a channel link, `@handle`, bare channel id, or a link to one of
/// the channel's videos. Blocking.
pub fn resolve(input: &str) -> Result<Channel, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Enter a channel link, @handle or channel ID".into());
    }
    let id = match find_channel_id(input) {
        Some(id) => id,
        None => {
            let url = page_url(input)?;
            match from_page(&url) {
                Ok(id) => id,
                Err(e) => from_ytdlp(&url).ok_or(e)?,
            }
        }
    };
    let feed = feed::fetch_channel(&id)
        .map_err(|e| format!("Found the channel, but its videos didn't load ({e})"))?;
    let name = if feed.channel.name.is_empty() { id.clone() } else { feed.channel.name };
    Ok(Channel { id, name })
}

fn page_url(input: &str) -> Result<String, String> {
    if input.starts_with("https://") || input.starts_with("http://") {
        return Ok(input.to_string());
    }
    if input.contains("youtube.com/") || input.contains("youtu.be/") {
        return Ok(format!("https://{input}"));
    }
    let handle = input.trim_start_matches('@');
    if handle.is_empty() || handle.chars().any(|c| c.is_whitespace() || c == '/') {
        return Err("That doesn't look like a channel link or @handle".into());
    }
    Ok(format!("https://www.youtube.com/@{handle}"))
}

fn from_page(url: &str) -> Result<String, String> {
    let html = net::get_text(url).map_err(|e| format!("Couldn't open that page ({e})"))?;
    const MARKERS: [&str; 4] = [
        "<link rel=\"canonical\" href=\"https://www.youtube.com/channel/",
        "\"externalId\":\"",
        "itemprop=\"identifier\" content=\"",
        "\"channelId\":\"",
    ];
    MARKERS
        .iter()
        .find_map(|marker| {
            let start = html.find(marker)? + marker.len();
            html.get(start..start + 24).filter(|id| is_channel_id(id)).map(str::to_string)
        })
        .ok_or_else(|| "Couldn't find a channel at that address".into())
}

/// Fallback for pages the scraper can't read.
fn from_ytdlp(url: &str) -> Option<String> {
    let out = Command::new(crate::tools::yt_dlp())
        .args(["--flat-playlist", "--playlist-items", "0", "--dump-single-json", "--no-warnings"])
        .arg(url)
        .output()
        .ok()?;
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    ["channel_id", "id"]
        .iter()
        .filter_map(|k| json.get(k)?.as_str())
        .find(|id| is_channel_id(id))
        .map(str::to_string)
}

/// Reads a subscription export: Google Takeout's subscriptions.csv, NewPipe or
/// FreeTube JSON, or OPML. Anything else falls back to scanning for channel ids;
/// their names are filled in on the next refresh.
pub fn parse_import(text: &str) -> Vec<Channel> {
    let mut out = Vec::new();

    // NewPipe: one JSON document. FreeTube: one JSON profile per line.
    for doc in std::iter::once(text).chain(text.lines()) {
        let doc = doc.trim();
        if !doc.starts_with('{') {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(doc) else { continue };
        let Some(subs) = value.get("subscriptions").and_then(|s| s.as_array()) else { continue };
        for sub in subs {
            let name = sub.get("name").and_then(|n| n.as_str()).unwrap_or_default();
            let id = sub
                .get("id")
                .and_then(|i| i.as_str())
                .filter(|i| is_channel_id(i))
                .map(str::to_string)
                .or_else(|| sub.get("url").and_then(|u| u.as_str()).and_then(find_channel_id));
            if let Some(id) = id {
                push(&mut out, id, name);
            }
        }
    }

    if out.is_empty() && text.contains("<opml") {
        if let Ok(doc) = roxmltree::Document::parse(text) {
            for node in doc.descendants().filter(|n| n.tag_name().name() == "outline") {
                if let Some(id) = node.attribute("xmlUrl").and_then(find_channel_id) {
                    let name = node.attribute("title").or(node.attribute("text")).unwrap_or_default();
                    push(&mut out, id, name);
                }
            }
        }
    }

    // Takeout: "Channel Id,Channel Url,Channel Title" (header may be localized).
    if out.is_empty() {
        for line in text.lines() {
            let mut cols = line.splitn(3, ',');
            let id = cols.next().unwrap_or_default().trim();
            if is_channel_id(id) {
                let _url = cols.next();
                let title = cols.next().unwrap_or_default().trim();
                let title = title.strip_prefix('"').and_then(|t| t.strip_suffix('"')).unwrap_or(title);
                push(&mut out, id.to_string(), &title.replace("\"\"", "\""));
            }
        }
    }

    if out.is_empty() {
        for id in find_channel_ids(text) {
            push(&mut out, id, "");
        }
    }
    out
}

fn push(out: &mut Vec<Channel>, id: String, name: &str) {
    if is_channel_id(&id) && !out.iter().any(|c| c.id == id) {
        let name = if name.trim().is_empty() { id.clone() } else { name.trim().to_string() };
        out.push(Channel { id, name });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "UCabcdefghijklmnopqrstuv";

    #[test]
    fn finds_ids_in_urls() {
        assert_eq!(find_channel_id(&format!("https://www.youtube.com/channel/{ID}/videos")), Some(ID.into()));
        assert_eq!(find_channel_id(ID), Some(ID.into()));
        assert_eq!(find_channel_id(&format!("x{ID}")), None);
        assert_eq!(find_channel_id("@somebody"), None);
    }

    #[test]
    fn handles() {
        assert_eq!(page_url("@tom").unwrap(), "https://www.youtube.com/@tom");
        assert_eq!(page_url("tom").unwrap(), "https://www.youtube.com/@tom");
        assert_eq!(page_url("youtube.com/@tom").unwrap(), "https://youtube.com/@tom");
        assert!(page_url("two words").is_err());
    }

    #[test]
    fn takeout_csv() {
        let csv = format!(
            "Channel Id,Channel Url,Channel Title\n{ID},http://www.youtube.com/channel/{ID},\"Tom, \"\"the\"\" Builder\"\nUCzzzzzzzzzzzzzzzzzzzzzz,http://x,Ann\n"
        );
        let chans = parse_import(&csv);
        assert_eq!(chans.len(), 2);
        assert_eq!(chans[0].name, "Tom, \"the\" Builder");
        assert_eq!(chans[1].name, "Ann");
    }

    #[test]
    fn newpipe_and_freetube() {
        let newpipe = format!(
            r#"{{"app_version":"0.27","subscriptions":[{{"service_id":0,"url":"https://www.youtube.com/channel/{ID}","name":"Tom"}}]}}"#
        );
        assert_eq!(parse_import(&newpipe), vec![Channel { id: ID.into(), name: "Tom".into() }]);
        let freetube = format!(
            "{{\"name\":\"All Channels\",\"subscriptions\":[{{\"id\":\"{ID}\",\"name\":\"Tom\",\"thumbnail\":\"\"}}],\"_id\":\"allChannels\"}}\n{{\"name\":\"Fav\",\"subscriptions\":[{{\"id\":\"{ID}\",\"name\":\"Tom\"}}]}}"
        );
        assert_eq!(parse_import(&freetube).len(), 1);
    }

    #[test]
    fn opml() {
        let opml = format!(
            r#"<opml version="1.1"><body><outline text="YouTube"><outline text="Tom" title="Tom" type="rss" xmlUrl="https://www.youtube.com/feeds/videos.xml?channel_id={ID}"/></outline></body></opml>"#
        );
        assert_eq!(parse_import(&opml), vec![Channel { id: ID.into(), name: "Tom".into() }]);
    }
}
