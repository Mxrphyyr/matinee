//! Small formatting helpers: timestamps, relative times, view counts,
//! durations, and plain words for yt-dlp's errors.

use crate::tools::Browser;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Parses the RFC 3339 timestamps YouTube puts in its feeds
/// (`2024-05-01T15:00:06+00:00`) into Unix seconds.
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let s = s.trim();
    let num = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, se) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let mut rest = s.get(19..)?;
    if let Some(frac) = rest.strip_prefix('.') {
        rest = frac.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let offset = match rest.as_bytes().first() {
        None | Some(b'Z') | Some(b'z') => 0,
        Some(&sign @ (b'+' | b'-')) => {
            let hh: i64 = rest.get(1..3)?.parse().ok()?;
            let mm: i64 = rest.get(4..6)?.parse().ok()?;
            let off = hh * 3600 + mm * 60;
            if sign == b'+' {
                off
            } else {
                -off
            }
        }
        _ => return None,
    };
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se - offset)
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn time_ago(ts: i64, now: i64) -> String {
    let secs = (now - ts).max(0);
    let (mins, hours, days) = (secs / 60, secs / 3600, secs / 86_400);
    if secs < 60 {
        "Just now".into()
    } else if mins < 60 {
        ago(mins, "minute")
    } else if hours < 24 {
        ago(hours, "hour")
    } else if days == 1 {
        "Yesterday".into()
    } else if days < 7 {
        ago(days, "day")
    } else if days < 30 {
        ago(days / 7, "week")
    } else if days < 365 {
        ago(days / 30, "month")
    } else {
        ago(days / 365, "year")
    }
}

fn ago(n: i64, unit: &str) -> String {
    if n == 1 {
        format!("1 {unit} ago")
    } else {
        format!("{n} {unit}s ago")
    }
}

pub fn views(n: u64) -> String {
    fn short(v: f64, suffix: &str) -> String {
        if v < 9.95 {
            let s = format!("{v:.1}");
            format!("{}{suffix}", s.trim_end_matches(".0"))
        } else {
            format!("{v:.0}{suffix}")
        }
    }
    match n {
        1 => "1 view".into(),
        0..=999 => format!("{n} views"),
        1_000..=999_499 => format!("{} views", short(n as f64 / 1e3, "K")),
        999_500..=999_499_999 => format!("{} views", short(n as f64 / 1e6, "M")),
        _ => format!("{} views", short(n as f64 / 1e9, "B")),
    }
}

pub fn duration(secs: u32) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

pub fn greeting(hour: i32) -> &'static str {
    match hour {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        18..=22 => "Good evening",
        _ => "Late show",
    }
}

pub fn count(n: usize, singular: &str, plural: &str) -> String {
    if n == 1 {
        format!("1 {singular}")
    } else {
        format!("{n} {plural}")
    }
}

pub fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00+00:00"), Some(0));
        assert_eq!(parse_rfc3339("2024-05-01T15:00:06+00:00"), Some(1_714_575_606));
        assert_eq!(parse_rfc3339("2024-05-01T17:00:06+02:00"), Some(1_714_575_606));
        assert_eq!(parse_rfc3339("2024-05-01T15:00:06.123Z"), Some(1_714_575_606));
        assert_eq!(parse_rfc3339("2000-02-29T00:00:00Z"), Some(951_782_400));
        assert_eq!(parse_rfc3339("nonsense"), None);
    }

    #[test]
    fn relative_times() {
        let now = 1_000_000;
        assert_eq!(time_ago(now - 30, now), "Just now");
        assert_eq!(time_ago(now - 60, now), "1 minute ago");
        assert_eq!(time_ago(now - 3 * 3600, now), "3 hours ago");
        assert_eq!(time_ago(now - 86_400 - 5, now), "Yesterday");
        assert_eq!(time_ago(now - 15 * 86_400, now), "2 weeks ago");
    }

    #[test]
    fn view_counts() {
        assert_eq!(views(1), "1 view");
        assert_eq!(views(532), "532 views");
        assert_eq!(views(1_000), "1K views");
        assert_eq!(views(1_234), "1.2K views");
        assert_eq!(views(56_789), "57K views");
        assert_eq!(views(999_999), "1M views");
        assert_eq!(views(2_450_000), "2.5M views");
        assert_eq!(views(3_000_000_000), "3B views");
    }

    #[test]
    fn durations() {
        assert_eq!(duration(59), "0:59");
        assert_eq!(duration(754), "12:34");
        assert_eq!(duration(3723), "1:02:03");
    }
}

/// "ERROR: [youtube] dQw4w9WgXcQ: Sign in to confirm…" becomes "Sign in to confirm…".
pub fn tidy_error(text: &str) -> String {
    let text = text.rsplit("ERROR:").next().unwrap_or(text).trim();
    let text = match text.strip_prefix('[') {
        Some(rest) => rest.split_once("] ").map_or(rest, |(_, after)| after),
        None => text,
    };
    match text.split_once(": ") {
        Some((id, reason)) if crate::model::is_video_id(id) => reason.to_string(),
        _ => text.to_string(),
    }
}

/// A YouTube refusal in plain words.
pub struct Explained {
    pub title: String,
    pub detail: String,
}

fn plain(message: &str) -> String {
    message.to_lowercase().replace('\u{2019}', "'")
}

/// YouTube won't play anything for this internet connection without a sign-in.
pub fn is_bot_check(message: &str) -> bool {
    plain(message).contains("not a bot")
}

/// Plain words for the YouTube refusals people actually meet, in place of
/// yt-dlp's command-line advice. Each is one a browser sign-in (or another
/// browser's) gets past. `sign_in` is the browser whose YouTube sign-in
/// yt-dlp was given, if any.
pub fn explain_ytdlp(message: &str, sign_in: Option<&Browser>) -> Option<Explained> {
    let message = plain(message);
    let explained = |title: &str, detail: String| Some(Explained { title: title.to_string(), detail });
    let choose = "choose that browser in Preferences, under Playback";
    if message.contains("not a bot") {
        let Some(browser) = sign_in else {
            return explained(
                "YouTube wants to check you're not a bot",
                format!(
                    "YouTube asks this when lots of requests come from one internet connection, \
                     or from a VPN, and it stops by itself after a while.\n\n\
                     To watch now, sign in to YouTube in a web browser, then {choose}."
                ),
            );
        };
        let name = &browser.name;
        let mut detail = format!(
            "Matinee used your {name} sign-in, but YouTube still asks. Open youtube.com in {name} \
             and make sure you're signed in, then try again."
        );
        if !browser.is_firefox_based() {
            detail.push_str(&format!(
                "\n\nIf you are, {name} may be keeping its sign-in locked away. Firefox never does: \
                 sign in to YouTube there, then choose Firefox in Preferences."
            ));
        }
        explained("YouTube still wants to check you're not a bot", detail)
    } else if ["cookies database", "cookie database", "failed to load cookies", "failed to decrypt", "keyring"]
        .iter()
        .any(|words| message.contains(words))
    {
        let detail = match sign_in {
            Some(browser) if browser.is_firefox_based() => format!(
                "{name} hasn't saved a sign-in Matinee can find. Open {name}, sign in to YouTube, \
                 then try again. Or choose another browser in Preferences, under Playback.",
                name = browser.name
            ),
            _ => format!(
                "{} isn't set up on this computer, or it keeps its sign-in locked away. Open it, \
                 sign in to YouTube, then try again. Or choose another browser in Preferences, \
                 under Playback; Firefox works best.",
                sign_in.map_or("The browser", |browser| browser.name.as_str())
            ),
        };
        explained("Matinee couldn't read your browser's sign-in", detail)
    } else if message.contains("confirm your age") || message.contains("age-restricted") {
        let detail = match sign_in {
            None => format!(
                "YouTube only shows it to signed-in adults. To watch it here, sign in to YouTube \
                 in a web browser, then {choose}. Or open it in your browser."
            ),
            Some(browser) => format!(
                "YouTube only shows it to signed-in adults, and didn't accept your {} sign-in for \
                 it. Open it in your browser instead.",
                browser.name
            ),
        };
        explained("This video is age-restricted", detail)
    } else {
        None
    }
}

#[cfg(test)]
mod explain_tests {
    use super::{explain_ytdlp, is_bot_check, tidy_error};
    use crate::tools::Browser;

    const BOT_CHECK: &str = "Sign in to confirm you\u{2019}re not a bot. Use --cookies-from-browser or --cookies for the authentication.";

    fn browser(name: &str, source: &str) -> Browser {
        Browser { key: source.into(), name: name.into(), source: source.into() }
    }

    #[test]
    fn explains_the_bot_check() {
        assert!(is_bot_check(BOT_CHECK));
        let explained = explain_ytdlp(BOT_CHECK, None).expect("recognised");
        assert!(explained.title.contains("not a bot"));
        assert!(explained.detail.contains("Preferences") && !explained.detail.contains("--cookies"));
    }

    #[test]
    fn explains_the_bot_check_despite_a_sign_in() {
        let firefox = explain_ytdlp(BOT_CHECK, Some(&browser("Firefox", "firefox"))).unwrap();
        assert!(firefox.title.contains("still"));
        assert!(firefox.detail.contains("Firefox sign-in") && !firefox.detail.contains("locked"));
        let chrome = explain_ytdlp(BOT_CHECK, Some(&browser("Google Chrome", "chrome"))).unwrap();
        assert!(chrome.detail.contains("Google Chrome sign-in") && chrome.detail.contains("choose Firefox"));
    }

    #[test]
    fn explains_unreadable_sign_ins() {
        let raw = "could not find chrome cookies database in \"/home/me/.config/google-chrome\"";
        let explained = explain_ytdlp(raw, Some(&browser("Google Chrome", "chrome"))).unwrap();
        assert!(explained.title.contains("couldn't read"));
        assert!(explained.detail.starts_with("Google Chrome") && explained.detail.contains("Firefox works best"));
        let raw = "could not find firefox cookies database in '/home/me/.mozilla/firefox'";
        let firefox = explain_ytdlp(raw, Some(&browser("Firefox", "firefox"))).unwrap();
        assert!(firefox.detail.starts_with("Firefox hasn't saved") && !firefox.detail.contains("locked"));
    }

    #[test]
    fn explains_age_restrictions() {
        assert!(explain_ytdlp("Sign in to confirm your age. This video may be inappropriate", None).is_some());
    }

    #[test]
    fn leaves_other_errors_alone() {
        assert!(explain_ytdlp("Video unavailable. This video has been removed by the uploader", None).is_none());
        assert!(!is_bot_check("Video unavailable"));
    }

    #[test]
    fn tidies_yt_dlp_errors() {
        assert_eq!(
            tidy_error("[ytdl_hook] ERROR: [youtube] jNQXAC9IVRw: Sign in to confirm you're not a bot."),
            "Sign in to confirm you're not a bot."
        );
        assert_eq!(tidy_error("ERROR: [youtube:tab] Unable to download webpage"), "Unable to download webpage");
        assert_eq!(tidy_error("Failed to recognize file format."), "Failed to recognize file format.");
    }
}
