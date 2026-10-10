//! Everything Matinee remembers lives in one JSON file on this machine:
//! ~/.local/share/matinee/state.json. Nothing is synced anywhere.

use crate::model::{Channel, Group, Video};
use gtk::glib;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Default, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    pub channels: Vec<Channel>,
    /// The last fetched feed, so the window fills instantly on launch.
    #[serde(default)]
    pub feed: Vec<Video>,
    #[serde(default)]
    pub watched: HashSet<String>,
    /// Videos that have been on screen at least once; anything else is "New".
    #[serde(default)]
    pub seen: HashSet<String>,
    #[serde(default)]
    pub hide_shorts: bool,
    /// Dark even when the desktop uses a light color scheme.
    #[serde(default)]
    pub always_dark: bool,
    /// Play in mpv's own window instead of inside Matinee.
    #[serde(default)]
    pub external_player: bool,
    /// Player volume, 0–100, remembered between sessions.
    #[serde(default)]
    pub volume: Option<f64>,
    /// Where unfinished videos were left: video id → [position, duration] in seconds.
    #[serde(default)]
    pub progress: HashMap<String, [f64; 2]>,
    #[serde(default)]
    pub window: Option<WindowState>,
    /// Tallest video to stream, or None to match the screen.
    #[serde(default)]
    pub quality: Option<u32>,
    #[serde(default)]
    pub hide_sidebar: bool,
    #[serde(default)]
    pub channels_collapsed: bool,
    /// 0 small, 1 medium (the default), 2 large.
    #[serde(default)]
    pub card_size: Option<u8>,
    /// Minutes between automatic checks; Some(0) means only on request.
    #[serde(default)]
    pub refresh_minutes: Option<u32>,
    #[serde(default)]
    pub no_smooth_scroll: bool,
    #[serde(default)]
    pub no_resume: bool,
    /// The browser whose YouTube sign-in yt-dlp may borrow (a key from
    /// `tools`, such as "firefox"); None means never.
    #[serde(default)]
    pub cookies_browser: Option<String>,
    /// When the feed was last checked for new uploads, in Unix seconds.
    #[serde(default)]
    pub last_refresh: Option<i64>,
    /// After a video ends, play the next one in the list it came from.
    #[serde(default)]
    pub autoplay: bool,
    /// Show subtitles when a video has them.
    #[serde(default)]
    pub subtitles: bool,
    /// Videos saved to watch later, newest saved first.
    #[serde(default)]
    pub watch_later: Vec<Video>,
    /// Named sets of channels, each with a feed of its own.
    #[serde(default)]
    pub groups: Vec<Group>,
    /// The feed filter in use: "unwatched", "week", or nothing for all.
    #[serde(default)]
    pub filter: Option<String>,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct WindowState {
    pub width: i32,
    pub height: i32,
    pub maximized: bool,
}

impl Store {
    fn path() -> PathBuf {
        glib::user_data_dir().join("matinee").join("state.json")
    }

    pub fn load() -> Store {
        let path = Self::path();
        let Ok(bytes) = std::fs::read(&path) else { return Store::default() };
        match serde_json::from_slice(&bytes) {
            Ok(store) => store,
            Err(e) => {
                // Keep the unreadable file instead of silently overwriting someone's subscriptions.
                let backup = path.with_extension(format!("broken-{}.json", crate::util::now()));
                let _ = std::fs::rename(&path, &backup);
                eprintln!("matinee: state file was unreadable ({e}); moved it to {}", backup.display());
                Store::default()
            }
        }
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("json.tmp");
        let result = serde_json::to_vec(self)
            .map_err(|e| e.to_string())
            .and_then(|bytes| std::fs::write(&tmp, bytes).map_err(|e| e.to_string()))
            .and_then(|_| std::fs::rename(&tmp, &path).map_err(|e| e.to_string()));
        if let Err(e) = result {
            eprintln!("matinee: couldn't save {}: {e}", path.display());
        }
    }

    /// Forgets the watched marks and resume points of videos that are in the
    /// feed now but not in `next`. Those of videos that were never in the
    /// feed (search results, a channel's older uploads) stay.
    pub fn forget_leaving(&mut self, next: &[Video]) {
        let staying: HashSet<&str> = next.iter().map(|v| v.id.as_str()).collect();
        let leaving: HashSet<String> = self
            .feed
            .iter()
            .filter(|v| !staying.contains(v.id.as_str()))
            .map(|v| v.id.clone())
            .collect();
        self.watched.retain(|id| !leaving.contains(id));
        self.progress.retain(|id, _| !leaving.contains(id));
    }

    pub fn is_subscribed(&self, channel_id: &str) -> bool {
        self.channels.iter().any(|c| c.id == channel_id)
    }

    pub fn channel_name(&self, channel_id: &str) -> Option<String> {
        self.channels.iter().find(|c| c.id == channel_id).map(|c| c.name.clone())
    }

    pub fn is_saved(&self, video_id: &str) -> bool {
        self.watch_later.iter().any(|v| v.id == video_id)
    }

    /// Saves a video to watch later, or removes it if it's there. Returns
    /// whether it's saved now.
    pub fn toggle_saved(&mut self, video: &Video) -> bool {
        if let Some(index) = self.watch_later.iter().position(|v| v.id == video.id) {
            self.watch_later.remove(index);
            false
        } else {
            self.watch_later.insert(0, video.clone());
            true
        }
    }

    pub fn group(&self, id: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    pub fn group_mut(&mut self, id: &str) -> Option<&mut Group> {
        self.groups.iter_mut().find(|g| g.id == id)
    }

    /// Makes a group with this name and returns its id.
    pub fn add_group(&mut self, name: &str) -> String {
        let id = format!("g{}-{}", crate::util::now(), self.groups.len());
        self.groups.push(Group { id: id.clone(), name: name.trim().to_string(), channels: Vec::new() });
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(id: &str) -> Video {
        Video { id: id.into(), ..Default::default() }
    }

    #[test]
    fn keeps_resume_points_of_videos_outside_the_feed() {
        let mut store = Store { feed: vec![video("stays"), video("leaves")], ..Default::default() };
        for id in ["stays", "leaves", "searched"] {
            store.watched.insert(id.to_string());
            store.progress.insert(id.to_string(), [60.0, 600.0]);
        }
        store.forget_leaving(&[video("stays"), video("arrives")]);
        let mut kept: Vec<&String> = store.progress.keys().collect();
        kept.sort();
        assert_eq!(kept, ["searched", "stays"]);
        assert!(store.watched.contains("searched") && !store.watched.contains("leaves"));
    }

    #[test]
    fn older_state_files_still_load() {
        let store: Store = serde_json::from_str(r#"{"channels":[],"cookies_browser":"chrome"}"#).unwrap();
        assert_eq!(store.cookies_browser.as_deref(), Some("chrome"));
        assert_eq!(store.last_refresh, None);
    }
}
