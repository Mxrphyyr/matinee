//! Small formatting helpers: timestamps, relative times, view counts, durations.

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
