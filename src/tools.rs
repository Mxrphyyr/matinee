//! Finding helper programs, and the browser sign-in yt-dlp may borrow. Apps
//! opened from a desktop launcher often get a shorter PATH than a terminal,
//! so ~/.local/bin is always checked too.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// The browser whose YouTube sign-in yt-dlp may borrow, when that's turned on.
static SIGN_IN: RwLock<Option<Browser>> = RwLock::new(None);

/// Browsers yt-dlp can read a YouTube sign-in from: the key Preferences
/// stores, the name people know, the family yt-dlp reads it as, the profile
/// folders (under the home folder) yt-dlp searches by itself, and the ones
/// it has to be pointed at: Flatpak and Snap installs, and Firefox's forks.
type Known = (&'static str, &'static str, &'static str, &'static [&'static str], &'static [&'static str]);

const BROWSERS: &[Known] = &[
    (
        "firefox",
        "Firefox",
        "firefox",
        &[
            ".config/mozilla/firefox",
            ".mozilla/firefox",
            ".var/app/org.mozilla.firefox/config/mozilla/firefox",
            ".var/app/org.mozilla.firefox/.mozilla/firefox",
            "snap/firefox/common/.mozilla/firefox",
        ],
        &[],
    ),
    (
        "librewolf",
        "LibreWolf",
        "firefox",
        &[],
        &[
            ".librewolf",
            ".config/librewolf/librewolf",
            ".var/app/io.gitlab.librewolf-community/.librewolf",
            ".var/app/io.gitlab.librewolf-community/config/librewolf/librewolf",
        ],
    ),
    ("waterfox", "Waterfox", "firefox", &[], &[".waterfox", ".var/app/net.waterfox.waterfox/.waterfox"]),
    ("zen", "Zen", "firefox", &[], &[".zen", ".var/app/app.zen_browser.zen/.zen"]),
    ("floorp", "Floorp", "firefox", &[], &[".floorp", ".var/app/one.ablaze.floorp/.floorp"]),
    (
        "chrome",
        "Google Chrome",
        "chrome",
        &[".config/google-chrome"],
        &[".var/app/com.google.Chrome/config/google-chrome"],
    ),
    (
        "chrome-dev",
        "Google Chrome Dev",
        "chrome",
        &[],
        &[".config/google-chrome-unstable", ".var/app/com.google.ChromeDev/config/google-chrome-unstable"],
    ),
    (
        "chromium",
        "Chromium",
        "chromium",
        &[".config/chromium"],
        &[".var/app/org.chromium.Chromium/config/chromium", "snap/chromium/common/chromium"],
    ),
    (
        "ungoogled-chromium",
        "Ungoogled Chromium",
        "chromium",
        &[],
        &[".var/app/io.github.ungoogled_software.ungoogled_chromium/config/chromium"],
    ),
    (
        "brave",
        "Brave",
        "brave",
        &[".config/BraveSoftware/Brave-Browser"],
        &[".var/app/com.brave.Browser/config/BraveSoftware/Brave-Browser"],
    ),
    (
        "edge",
        "Microsoft Edge",
        "edge",
        &[".config/microsoft-edge"],
        &[".var/app/com.microsoft.Edge/config/microsoft-edge"],
    ),
    ("vivaldi", "Vivaldi", "vivaldi", &[".config/vivaldi"], &[".var/app/com.vivaldi.Vivaldi/config/vivaldi"]),
    ("opera", "Opera", "opera", &[".config/opera"], &[".var/app/com.opera.Opera/config/opera"]),
];

/// A browser yt-dlp can borrow a YouTube sign-in from.
#[derive(Clone, Debug)]
pub struct Browser {
    /// What Preferences stores, such as "firefox".
    pub key: String,
    pub name: String,
    /// yt-dlp's `--cookies-from-browser` value: the family it reads, plus
    /// the profile folder when yt-dlp wouldn't look there by itself.
    pub source: String,
}

impl Browser {
    /// Firefox and its forks keep sign-ins unencrypted, so reading them never
    /// depends on the desktop's password store.
    pub fn is_firefox_based(&self) -> bool {
        self.source.starts_with("firefox")
    }
}

pub fn find(program: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(home) = home() {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join("bin"));
    }
    dirs.push(PathBuf::from("/usr/local/bin"));
    dirs.into_iter()
        .map(|dir| dir.join(program))
        .find(|path| is_executable(path))
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// yt-dlp, preferring a copy in ~/.local/bin: that's the one people keep up
/// to date themselves (`yt-dlp -U`), and YouTube breaks old versions often.
pub fn find_yt_dlp() -> Option<PathBuf> {
    home()
        .map(|home| home.join(".local/bin/yt-dlp"))
        .filter(|path| is_executable(path))
        .or_else(|| find("yt-dlp"))
}

/// yt-dlp's full path, or its bare name so error messages still read well.
pub fn yt_dlp() -> PathBuf {
    find_yt_dlp().unwrap_or_else(|| PathBuf::from("yt-dlp"))
}

/// yt-dlp's choice of streams for playing a video at most `height` pixels tall.
pub fn ytdl_format(height: u32) -> String {
    format!("bestvideo[height<=?{height}]+bestaudio/best[height<=?{height}]/best")
}

/// The browsers set up for this account, in the order Preferences lists them.
pub fn browsers() -> Vec<Browser> {
    match home() {
        Some(home) => BROWSERS.iter().filter_map(|known| locate(&home, known)).collect(),
        None => Vec::new(),
    }
}

/// The browser Preferences calls `key`. One that isn't set up (any more) is
/// still returned, to be looked for where yt-dlp looks by default, so the
/// problem it causes gets explained instead of the sign-in quietly stopping.
pub fn browser(key: &str) -> Browser {
    let known = BROWSERS.iter().find(|known| known.0 == key);
    if let (Some(home), Some(known)) = (home(), known) {
        if let Some(found) = locate(&home, known) {
            return found;
        }
    }
    let (name, family) = known.map_or((key, key), |&(_, name, family, _, _)| (name, family));
    Browser { key: key.to_string(), name: name.to_string(), source: family.to_string() }
}

/// Where `known` keeps its profiles, if it's set up for this account.
fn locate(home: &Path, &(key, name, family, usual, other): &Known) -> Option<Browser> {
    // Every Firefox profile folder holds a profiles.ini, every Chromium one a "Local State".
    let marker = if family == "firefox" { "profiles.ini" } else { "Local State" };
    let set_up = |dir: &str| home.join(dir).join(marker).is_file();
    let source = if usual.iter().any(|dir| set_up(dir)) {
        family.to_string()
    } else {
        let dir = other.iter().find(|dir| set_up(dir))?;
        format!("{family}:{}", home.join(dir).display())
    };
    Some(Browser { key: key.to_string(), name: name.to_string(), source })
}

/// Turns the browser sign-in on, for the browser Preferences calls `key`, or off.
pub fn set_sign_in(key: Option<&str>) {
    if let Ok(mut slot) = SIGN_IN.write() {
        *slot = key.map(browser);
    }
}

/// The browser whose sign-in is in use, if that's turned on.
pub fn sign_in() -> Option<Browser> {
    SIGN_IN.read().ok().and_then(|slot| slot.clone())
}

/// yt-dlp arguments that borrow the browser's sign-in, when it's on.
pub fn sign_in_args() -> Vec<String> {
    sign_in()
        .map(|browser| vec!["--cookies-from-browser".to_string(), browser.source])
        .unwrap_or_default()
}

/// The same, in the form mpv's `ytdl-raw-options` expects.
pub fn mpv_sign_in() -> String {
    sign_in().map(|browser| mpv_cookies_option(&browser.source)).unwrap_or_default()
}

/// The value is quoted with its length (`%n%`), so nothing in a folder's
/// name can be misread as the start of another option.
fn mpv_cookies_option(source: &str) -> String {
    format!("cookies-from-browser=%{}%{source}", source.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A home folder holding the given profile folders, each with its marker file.
    fn fake_home(name: &str, profiles: &[(&str, &str)]) -> PathBuf {
        let home = std::env::temp_dir().join(format!("matinee-test-{}-{name}", std::process::id()));
        for (dir, marker) in profiles {
            std::fs::create_dir_all(home.join(dir)).unwrap();
            std::fs::write(home.join(dir).join(marker), "").unwrap();
        }
        home
    }

    fn located(home: &Path, key: &str) -> Option<String> {
        let known = BROWSERS.iter().find(|known| known.0 == key).unwrap();
        locate(home, known).map(|browser| browser.source)
    }

    #[test]
    fn finds_regular_flatpak_and_forked_browsers() {
        let home = fake_home(
            "browsers",
            &[
                (".var/app/org.mozilla.firefox/.mozilla/firefox", "profiles.ini"),
                (".var/app/com.google.Chrome/config/google-chrome", "Local State"),
                (".config/chromium", "Local State"),
                (".var/app/io.gitlab.librewolf-community/.librewolf", "profiles.ini"),
                // A leftover folder without a profile isn't a browser that's set up.
                (".config/vivaldi", "Bookmarks"),
            ],
        );
        // yt-dlp finds Firefox in all its usual places by itself.
        assert_eq!(located(&home, "firefox").as_deref(), Some("firefox"));
        assert_eq!(located(&home, "chromium").as_deref(), Some("chromium"));
        let flatpak_chrome = home.join(".var/app/com.google.Chrome/config/google-chrome");
        assert_eq!(located(&home, "chrome"), Some(format!("chrome:{}", flatpak_chrome.display())));
        let librewolf = home.join(".var/app/io.gitlab.librewolf-community/.librewolf");
        assert_eq!(located(&home, "librewolf"), Some(format!("firefox:{}", librewolf.display())));
        assert_eq!(located(&home, "vivaldi"), None);
        assert_eq!(located(&home, "brave"), None);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn quotes_folders_for_mpv() {
        assert_eq!(mpv_cookies_option("firefox"), "cookies-from-browser=%7%firefox");
        assert_eq!(mpv_cookies_option("chrome:/a,b"), "cookies-from-browser=%11%chrome:/a,b");
    }

    #[test]
    fn firefox_forks_count_as_firefox() {
        let browser = |source: &str| Browser { key: String::new(), name: String::new(), source: source.into() };
        assert!(browser("firefox:/home/me/.librewolf").is_firefox_based());
        assert!(!browser("chrome").is_firefox_based());
    }
}
