//! The only network code in the app. Requests carry no account, no API key and
//! no tracking parameters: just the public feed, page or image being fetched.

use std::sync::OnceLock;
use std::time::Duration;

const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0";

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .user_agent(USER_AGENT)
            .build()
            .into()
    })
}

/// Base URL for channel feeds. Overridable so the app can be pointed at a
/// local mirror or test server.
pub fn feed_base() -> String {
    env_or("MATINEE_FEED_BASE", "https://www.youtube.com")
}

pub fn thumb_base() -> String {
    env_or("MATINEE_THUMB_BASE", "https://i.ytimg.com")
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .unwrap_or_else(|| default.to_string())
}

pub fn get_text(url: &str) -> Result<String, String> {
    agent()
        .get(url)
        // Declines YouTube's cookie-consent interstitial (shown in the EU)
        // instead of accepting tracking cookies.
        .header("Cookie", "SOCS=CAI")
        .header("Accept-Language", "en-US,en;q=0.8")
        .call()
        .map_err(describe)?
        .body_mut()
        .read_to_string()
        .map_err(describe)
}

pub fn get_bytes(url: &str) -> Result<Vec<u8>, String> {
    agent()
        .get(url)
        .call()
        .map_err(describe)?
        .body_mut()
        .read_to_vec()
        .map_err(describe)
}

fn describe(e: ureq::Error) -> String {
    match e {
        ureq::Error::StatusCode(code) => format!("HTTP {code}"),
        ureq::Error::Timeout(_) => "timed out".into(),
        ureq::Error::HostNotFound => "no connection".into(),
        other => other.to_string(),
    }
}
