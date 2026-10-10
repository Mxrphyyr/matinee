//! Channel feeds via YouTube's public Atom endpoint. No login, no API key,
//! and nothing in the response but the channel's 15 most recent uploads.

use crate::model::{is_video_id, Channel, Video};
use crate::{net, util};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

pub struct ChannelFeed {
    pub channel: Channel,
    pub videos: Vec<Video>,
}

#[derive(Default)]
pub struct Report {
    pub feeds: Vec<ChannelFeed>,
    pub failed: Vec<(Channel, String)>,
}

pub fn feed_url(channel_id: &str) -> String {
    format!("{}/feeds/videos.xml?channel_id={channel_id}", net::feed_base())
}

pub fn fetch_channel(channel_id: &str) -> Result<ChannelFeed, String> {
    parse(&net::get_text(&feed_url(channel_id))?)
}

/// Fetches every channel. Blocking: run it off the UI thread. A first load
/// goes eight channels at a time; `gentle` checks go four at a time with a
/// pause after each, so hundreds of subscriptions never reach YouTube as
/// one burst (bursts are what make it start asking whether you're a bot).
pub fn fetch_all(channels: &[Channel], gentle: bool) -> Report {
    let (workers, pause) = if gentle { (4, Duration::from_millis(250)) } else { (8, Duration::ZERO) };
    let next = AtomicUsize::new(0);
    let report = Mutex::new(Report::default());
    std::thread::scope(|scope| {
        for _ in 0..workers.min(channels.len()) {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(channel) = channels.get(i) else { break };
                let result = fetch_channel(&channel.id);
                {
                    let mut report = report.lock().unwrap();
                    match result {
                        Ok(mut feed) => {
                            // Key everything by the id we subscribed with.
                            feed.channel.id = channel.id.clone();
                            if feed.channel.name.is_empty() {
                                feed.channel.name = channel.name.clone();
                            }
                            for v in &mut feed.videos {
                                v.channel_id = channel.id.clone();
                                if v.channel_name.is_empty() {
                                    v.channel_name = feed.channel.name.clone();
                                }
                            }
                            report.feeds.push(feed);
                        }
                        Err(e) => report.failed.push((channel.clone(), e)),
                    }
                }
                std::thread::sleep(pause);
            });
        }
    });
    report.into_inner().unwrap()
}

pub fn parse(xml: &str) -> Result<ChannelFeed, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("unreadable feed: {e}"))?;
    let feed = doc.root_element();
    if feed.tag_name().name() != "feed" {
        return Err("not a channel feed".into());
    }
    let channel_name = child(feed, "author")
        .and_then(|a| child_text(a, "name"))
        .or_else(|| child_text(feed, "title"))
        .unwrap_or_default();
    let channel_id = child_text(feed, "channelId").unwrap_or_default();

    let videos = feed
        .children()
        .filter(|n| n.tag_name().name() == "entry")
        .filter_map(|entry| {
            let id = child_text(entry, "videoId").filter(|id| is_video_id(id))?;
            let link = child(entry, "link")
                .and_then(|n| n.attribute("href"))
                .unwrap_or_default();
            let views = entry
                .descendants()
                .find(|n| n.tag_name().name() == "statistics")
                .and_then(|n| n.attribute("views"))
                .and_then(|v| v.parse().ok());
            Some(Video {
                title: child_text(entry, "title").unwrap_or_default(),
                channel_id: child_text(entry, "channelId").unwrap_or_else(|| channel_id.clone()),
                channel_name: channel_name.clone(),
                published: child_text(entry, "published")
                    .as_deref()
                    .and_then(util::parse_rfc3339),
                views,
                duration: None,
                is_short: link.contains("/shorts/"),
                id,
            })
        })
        .collect();

    Ok(ChannelFeed {
        channel: Channel { id: channel_id, name: channel_name },
        videos,
    })
}

fn child<'a, 'i>(node: roxmltree::Node<'a, 'i>, name: &str) -> Option<roxmltree::Node<'a, 'i>> {
    node.children().find(|n| n.is_element() && n.tag_name().name() == name)
}

fn child_text(node: roxmltree::Node, name: &str) -> Option<String> {
    child(node, name)
        .and_then(|n| n.text())
        .map(|t| t.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015" xmlns:media="http://search.yahoo.com/mrss/" xmlns="http://www.w3.org/2005/Atom">
 <link rel="self" href="http://www.youtube.com/feeds/videos.xml?channel_id=UCabcdefghijklmnopqrstuv"/>
 <id>yt:channel:abcdefghijklmnopqrstuv</id>
 <yt:channelId>UCabcdefghijklmnopqrstuv</yt:channelId>
 <title>Tom &amp; Friends</title>
 <author><name>Tom &amp; Friends</name><uri>https://www.youtube.com/channel/UCabcdefghijklmnopqrstuv</uri></author>
 <entry>
  <id>yt:video:dQw4w9WgXcQ</id>
  <yt:videoId>dQw4w9WgXcQ</yt:videoId>
  <yt:channelId>UCabcdefghijklmnopqrstuv</yt:channelId>
  <title>Building a &quot;tiny&quot; lathe</title>
  <link rel="alternate" href="https://www.youtube.com/watch?v=dQw4w9WgXcQ"/>
  <published>2024-05-01T15:00:06+00:00</published>
  <media:group>
   <media:title>Building a "tiny" lathe</media:title>
   <media:thumbnail url="https://i4.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg" width="480" height="360"/>
   <media:community><media:starRating count="10" average="5.00" min="1" max="5"/><media:statistics views="56789"/></media:community>
  </media:group>
 </entry>
 <entry>
  <yt:videoId>abcdefghijk</yt:videoId>
  <title>Quick tip</title>
  <link rel="alternate" href="https://www.youtube.com/shorts/abcdefghijk"/>
  <published>2024-05-02T09:30:00+00:00</published>
 </entry>
</feed>"#;

    #[test]
    fn parses_youtube_feed() {
        let feed = parse(SAMPLE).unwrap();
        assert_eq!(feed.channel.id, "UCabcdefghijklmnopqrstuv");
        assert_eq!(feed.channel.name, "Tom & Friends");
        assert_eq!(feed.videos.len(), 2);
        let v = &feed.videos[0];
        assert_eq!(v.id, "dQw4w9WgXcQ");
        assert_eq!(v.title, "Building a \"tiny\" lathe");
        assert_eq!(v.views, Some(56_789));
        assert_eq!(v.published, Some(1_714_575_606));
        assert!(!v.is_short);
        assert!(feed.videos[1].is_short);
        assert_eq!(feed.videos[1].channel_id, "UCabcdefghijklmnopqrstuv");
    }

    #[test]
    fn rejects_html() {
        assert!(parse("<html><body>consent</body></html>").is_err());
    }
}
