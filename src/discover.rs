//! "Discover" picks a few dozen videos at random from your own channels,
//! leaning toward the channels you watch most and uploads doing well for
//! their channel, at most two per channel, leaving out what you've watched.
//! It works only from what Matinee already has: no requests, no account,
//! nothing sent anywhere. Shuffling picks again.

use crate::model::Video;
use std::collections::{HashMap, HashSet};

pub const SHOWN: usize = 40;
const PER_CHANNEL: usize = 2;

pub fn pick(videos: &[Video], watched: &HashSet<String>, now: i64, seed: u64) -> Vec<Video> {
    // Which channels you watch most, from the watched marks.
    let mut watched_per_channel: HashMap<&str, usize> = HashMap::new();
    for video in videos.iter().filter(|v| watched.contains(&v.id)) {
        *watched_per_channel.entry(video.channel_id.as_str()).or_default() += 1;
    }
    let most_watched = watched_per_channel.values().copied().max().unwrap_or(0).max(1) as f64;
    // Each channel's typical views, to tell an upload doing well.
    let mut views_per_channel: HashMap<&str, Vec<f64>> = HashMap::new();
    for video in videos {
        if let Some(views) = video.views {
            views_per_channel.entry(video.channel_id.as_str()).or_default().push(views as f64);
        }
    }
    let typical: HashMap<&str, f64> = views_per_channel.into_iter().map(|(channel, views)| (channel, median(views))).collect();

    let mut seen = HashSet::new();
    let mut candidates: Vec<(f64, &Video)> = videos
        .iter()
        .filter(|v| !watched.contains(&v.id) && seen.insert(v.id.as_str()))
        .map(|video| {
            let affinity = watched_per_channel.get(video.channel_id.as_str()).copied().unwrap_or(0) as f64 / most_watched;
            let typical = typical.get(video.channel_id.as_str()).copied();
            (weight(video, affinity, typical, now), video)
        })
        .collect();

    let mut rng = Rng::new(seed);
    let mut per_channel: HashMap<&str, usize> = HashMap::new();
    let mut picks = Vec::with_capacity(SHOWN);
    while picks.len() < SHOWN && !candidates.is_empty() {
        let total: f64 = candidates.iter().map(|(w, _)| w).sum();
        let mut roll = rng.float() * total;
        let mut index = candidates.len() - 1;
        for (i, (w, _)) in candidates.iter().enumerate() {
            roll -= w;
            if roll <= 0.0 {
                index = i;
                break;
            }
        }
        let (_, video) = candidates.swap_remove(index);
        let count = per_channel.entry(video.channel_id.as_str()).or_default();
        if *count < PER_CHANNEL {
            *count += 1;
            picks.push(video.clone());
        }
    }
    picks
}

/// How likely a video is to be picked: fresher uploads, channels you watch
/// more, and uploads with more views than usual for their channel.
fn weight(video: &Video, affinity: f64, typical_views: Option<f64>, now: i64) -> f64 {
    let days = video.published.map_or(60.0, |published| (now - published).max(0) as f64 / 86_400.0);
    let freshness = 1.0 / (1.0 + days / 30.0);
    let liking = 1.0 + 2.0 * affinity;
    let doing_well = match (video.views, typical_views) {
        (Some(views), Some(typical)) => 1.0 + ((views as f64 + 10.0) / (typical + 10.0)).ln().clamp(0.0, 2.0),
        _ => 1.0,
    };
    freshness * liking * doing_well
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

/// A small, fast random source (xorshift), seeded so a set can be repeated.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Evenly spread in 0 up to (not including) 1.
    fn float(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 10_000_000;

    fn video(id: &str, channel: &str, views: u64, days_old: f64) -> Video {
        Video {
            id: id.into(),
            title: id.into(),
            channel_id: channel.into(),
            channel_name: channel.into(),
            published: Some(NOW - (days_old * 86_400.0) as i64),
            views: Some(views),
            ..Default::default()
        }
    }

    #[test]
    fn leaves_out_watched_and_limits_each_channel() {
        let mut videos = Vec::new();
        for channel in ["a", "b", "c"] {
            for i in 0..10 {
                videos.push(video(&format!("{channel}{i}"), channel, 1000, i as f64));
            }
        }
        let watched: HashSet<String> = ["a0", "b0"].iter().map(|s| s.to_string()).collect();
        let picks = pick(&videos, &watched, NOW, 7);
        assert!(!picks.is_empty() && picks.len() <= 3 * PER_CHANNEL);
        assert!(picks.iter().all(|v| !watched.contains(&v.id)));
        for channel in ["a", "b", "c"] {
            assert!(picks.iter().filter(|v| v.channel_id == channel).count() <= PER_CHANNEL);
        }
        // The same seed gives the same set again.
        let ids = |picks: &[Video]| picks.iter().map(|v| v.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&pick(&videos, &watched, NOW, 7)), ids(&picks));
    }

    #[test]
    fn weights_favour_fresh_liked_and_doing_well() {
        let fresh = weight(&video("x", "a", 1000, 1.0), 0.0, Some(1000.0), NOW);
        let stale = weight(&video("x", "a", 1000, 90.0), 0.0, Some(1000.0), NOW);
        assert!(fresh > stale * 2.0);
        let liked = weight(&video("x", "a", 1000, 1.0), 1.0, Some(1000.0), NOW);
        assert!(liked > fresh * 2.5);
        let hit = weight(&video("x", "a", 20_000, 1.0), 0.0, Some(1000.0), NOW);
        assert!(hit > fresh * 2.0);
        // No view counts: still eligible, just unweighted on that score.
        let mut no_views = video("x", "a", 0, 1.0);
        no_views.views = None;
        assert!(weight(&no_views, 0.0, None, NOW) > 0.0);
    }

    #[test]
    fn same_video_counted_once() {
        let twice = vec![video("v", "a", 10, 1.0), video("v", "a", 10, 1.0)];
        assert_eq!(pick(&twice, &HashSet::new(), NOW, 3).len(), 1);
    }
}
