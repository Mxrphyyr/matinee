//! "Trending" ranks your subscriptions' recent uploads by how unusually well
//! each is doing for its own channel, so a small channel's breakout can sit
//! above a big channel's ordinary upload. It works only from the view counts
//! already in the feed: no extra requests, no account, nothing sent anywhere.

use crate::model::Video;
use std::collections::HashMap;

/// Only uploads from the last two weeks can trend.
const WINDOW_HOURS: f64 = 14.0 * 24.0;
const SHOWN: usize = 60;

pub fn rank(feed: &[Video], now: i64) -> Vec<Video> {
    let age_hours = |published: i64| ((now - published) as f64 / 3600.0).max(0.5);

    // Each channel's typical views, from uploads old enough to have settled.
    let mut by_channel: HashMap<&str, Vec<(f64, f64)>> = HashMap::new();
    for video in feed {
        if let (Some(views), Some(published)) = (video.views, video.published) {
            by_channel
                .entry(video.channel_id.as_str())
                .or_default()
                .push((views as f64, age_hours(published)));
        }
    }
    let typical: HashMap<&str, f64> = by_channel
        .iter()
        .map(|(channel, uploads)| {
            let settled: Vec<f64> = uploads.iter().filter(|(_, age)| *age > 48.0).map(|(v, _)| *v).collect();
            let pool = if settled.len() >= 3 { settled } else { uploads.iter().map(|(v, _)| *v).collect() };
            (*channel, median(pool))
        })
        .collect();

    let mut scored: Vec<(f64, &Video)> = feed
        .iter()
        .filter_map(|video| {
            let views = video.views? as f64;
            let age = age_hours(video.published?);
            if age > WINDOW_HOURS {
                return None;
            }
            let usual = typical.get(video.channel_id.as_str()).copied().unwrap_or(views);
            // Views keep arriving for a couple of days, so compare a new upload
            // with what the channel usually has at the same age.
            let expected = usual * (1.0 - (-age / 36.0).exp());
            let outperformance = ((views + 10.0) / (expected + 10.0)).ln();
            // Gentle preference for fresh uploads, and for some absolute reach so a
            // lucky upload with a handful of views can't top the list.
            let freshness = -0.07 * age / 24.0;
            let reach = 0.12 * (views + 1.0).log10();
            Some((outperformance + freshness + reach, video))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.into_iter().take(SHOWN).map(|(_, video)| video.clone()).collect()
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    match n {
        0 => 0.0,
        n if n % 2 == 1 => values[n / 2],
        n => (values[n / 2 - 1] + values[n / 2]) / 2.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 10_000_000;
    const DAY: i64 = 86_400;

    fn video(id: &str, channel: &str, views: u64, days_old: f64) -> Video {
        Video {
            id: id.into(),
            title: id.into(),
            channel_id: channel.into(),
            channel_name: channel.into(),
            published: Some(NOW - (days_old * DAY as f64) as i64),
            views: Some(views),
            ..Default::default()
        }
    }

    #[test]
    fn breakouts_beat_ordinary_uploads() {
        let feed = vec![
            // A channel that usually gets ~1,000 views.
            video("old1", "a", 1_000, 10.0),
            video("old2", "a", 900, 9.0),
            video("old3", "a", 1_100, 8.0),
            video("hit", "a", 12_000, 2.0),
            video("usual", "a", 1_000, 2.0),
        ];
        let ranked: Vec<String> = rank(&feed, NOW).into_iter().map(|v| v.id).collect();
        assert_eq!(ranked[0], "hit");
        assert!(ranked.iter().position(|id| id == "hit") < ranked.iter().position(|id| id == "usual"));
    }

    #[test]
    fn small_channel_breakout_beats_big_channel_routine() {
        let feed = vec![
            video("s1", "small", 100, 6.0),
            video("s2", "small", 120, 5.0),
            video("s3", "small", 90, 4.0),
            video("small-hit", "small", 3_000, 1.0),
            video("b1", "big", 1_000_000, 6.0),
            video("b2", "big", 1_100_000, 5.0),
            video("b3", "big", 900_000, 4.0),
            video("big-routine", "big", 700_000, 1.0),
        ];
        let ranked: Vec<String> = rank(&feed, NOW).into_iter().map(|v| v.id).collect();
        let pos = |id: &str| ranked.iter().position(|r| r == id).unwrap();
        assert!(pos("small-hit") < pos("big-routine"));
    }

    #[test]
    fn old_uploads_do_not_trend() {
        let feed = vec![video("ancient", "a", 5_000_000, 30.0), video("fresh", "a", 10, 1.0)];
        let ranked: Vec<String> = rank(&feed, NOW).into_iter().map(|v| v.id).collect();
        assert_eq!(ranked, vec!["fresh".to_string()]);
    }
}
