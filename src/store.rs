//! Everything Matinee remembers lives in one JSON file on this machine:
//! ~/.local/share/matinee/state.json. Nothing is synced anywhere.

use crate::model::{Channel, Video};
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
    /// The browser whose YouTube sign-in yt-dlp may borrow; None means never.
    #[serde(default)]
    pub cookies_browser: Option<String>,
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

    pub fn is_subscribed(&self, channel_id: &str) -> bool {
        self.channels.iter().any(|c| c.id == channel_id)
    }

    pub fn channel_name(&self, channel_id: &str) -> Option<String> {
        self.channels.iter().find(|c| c.id == channel_id).map(|c| c.name.clone())
    }
}
