use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    pub id: String,
    pub name: String,
}

/// A named set of channels with a feed of its own.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub channels: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub channel_id: String,
    pub channel_name: String,
    /// Unix seconds. Search results don't always carry one.
    #[serde(default)]
    pub published: Option<i64>,
    #[serde(default)]
    pub views: Option<u64>,
    /// Seconds. Channel feeds don't include durations; search results do.
    #[serde(default)]
    pub duration: Option<u32>,
    #[serde(default)]
    pub is_short: bool,
}

impl Video {
    pub fn watch_url(&self) -> String {
        format!("https://www.youtube.com/watch?v={}", self.id)
    }
}

pub fn is_id_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

pub fn is_channel_id(s: &str) -> bool {
    s.len() == 24 && s.starts_with("UC") && s.bytes().all(is_id_byte)
}

pub fn is_video_id(s: &str) -> bool {
    s.len() == 11 && s.bytes().all(is_id_byte)
}
