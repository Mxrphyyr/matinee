# Matinee: notes for Claude

Read this at the start of every session. It carries over what was decided
while Matinee was built with Claude in a claude.ai chat (October 2026). The
reasons behind past decisions are in `docs/HISTORY.md`; read it before
changing anything it describes.

## The project and its owner

Matinee is a private, ad-free YouTube client for Linux: follow channels
without a Google account, browse a feed of their uploads, search, and watch
in an embedded player. Rust, GTK 4, libadwaita and libmpv; yt-dlp does all
YouTube extraction.

- Owner: **Mxrphyyr** on GitHub, shown in the app as "By Mxrphyy".
  Repository: https://github.com/Mxrphyyr/matinee (public, branch `main`).
  The local clone lives at `~/Downloads/matinee`.
- The owner runs **AerynOS** (moss package manager, LLVM toolchain) with
  **KDE Plasma on Wayland** and a **4K monitor**.
- The owner is not a developer. Explain in plain language, say what you're
  about to change and why, give copy-paste commands when they must act, and
  do the git work for them. You can't see the screen: ask them to check
  anything visual.

## Ground rules

- **Never change `README.md` without asking.** The owner edits it on the
  GitHub website (it has their screenshots). Maintainer docs live in
  `MAINTAINING.md`. Run `git pull --rebase` before starting work.
- Install only into `~/.local`, never `/usr`; moss manages the system.
- **Be gentle with YouTube.** Bursts of requests make YouTube answer "Sign in
  to confirm you're not a bot" for the owner's whole connection (this
  happened with 0.3.0 and again before 0.3.2; the owner follows 271
  channels). Never fetch more than the user is about to see; keep
  `--sleep-requests` on bulk reads and the pauses in feed checks.
- **No CSS `filter` or blur anywhere.** Blur caused every past stutter (feed
  scrolling, full-screen video, the page slide).
- UI text is plain, friendly and in sentence case. Explain problems in
  words, not command-line flags (see `util::explain_ytdlp`).
- Keep `Cargo.lock` in step with `Cargo.toml` (CI builds with `--locked`).

## Commands

- `cargo build`, `cargo test` (26 unit tests of pure logic; no display needed).
  AerynOS has no GNU `ar`: plain cargo needs `AR=llvm-ar` (install.sh sets it).
- `cargo run` opens the app on the owner's desktop
- `MATINEE_PERF=1 ~/.local/bin/matinee` prints timing: start-up stages, a
  frame report every 5 s (late frames and the longest UI work), and what the
  player decodes and drops. Ask the owner to run it via `!` and send the
  output; it's the only way to measure their 4K 240 Hz desktop.
- `./install.sh`: release build, installed into ~/.local (maps cc/ar/ld to
  clang/llvm-ar/lld when GNU tools are missing). `--check`, `--uninstall`.
- `./sync-to-github.sh "message"`: commit everything, pull (rebase), push.
  This is how changes reach GitHub.
- `./release.sh X.Y.Z`: bump the version in Cargo.toml and Cargo.lock, then
  sync; GitHub Actions publishes the release by itself.
- `scripts/package.sh`: the release archive, built locally into `dist/`.
- `shellcheck *.sh scripts/*.sh` for scripts; `actionlint` for the workflow.
- CI: `gh run list`, `gh run view <id> --log-failed`, `gh release list`.

## Releases

`.github/workflows/release.yml` runs on every push and pull request on
ubuntu-24.04, whose GTK 4.14, libadwaita 1.5 and libmpv 2.2 match the crate
features (`gtk` v4_12, `adw` v1_5). It tests, packages
`matinee-X.Y.Z-x86_64-linux.tar.gz` plus `.sha256`, and when the version in
Cargo.toml has no release yet, publishes one with the notes in
`.github/release-notes.md` plus GitHub's generated notes. Published so far:
v0.3.0, v0.3.1, v0.3.2 and v0.4.0. The owner's gh login has the `workflow` scope needed to
push workflow changes. A release archive installs with its own `./install.sh`,
which detects the prebuilt binary and checks libraries before installing.

## Code map (src/)

| File | What it does |
|---|---|
| `main.rs` | entry point; picks GTK's OpenGL renderer (`GSK_RENDERER=gl` on GTK 4.18+, `ngl` before) unless the user set one; crash log (`~/.cache/matinee/crash.log`, noted at the next start); shortcuts |
| `window.rs` | main window: sidebar drawer (filter, foldable Channels, Trending), feed grid, channel history, search, Preferences, saving |
| `card.rs`, `thumb.rs` | recyclable feed card; `ThumbPaintable` draws fade, zoom, dim and progress in `snapshot` (no relayout) |
| `thumbs.rs` | thumbnail loader: 6 workers, newest-first queue, cancellation, 120-image memory cache |
| `smooth.rs` | smooth wheel scrolling (exponential glide that follows the grid's own corrections) |
| `watch.rs` | embedded player: `Surface` (GLArea plus the one mpv renderer), picture in picture, quality menu, controls, errors |
| `mpv.rs` | libmpv FFI: client API and OpenGL render API, EGL/GLX proc lookup |
| `feed.rs` | channel RSS (Atom) feeds: 8 at once on a first load, later 4 at once with pauses; one retry |
| `ytdlp.rs` | runs yt-dlp for a list and streams its entries as they arrive; optional page gating (reading pauses until `more`) |
| `history.rs` | a channel's older uploads: `ytdlp` with 60 per page, `--sleep-requests 1` |
| `trending.rs` | Trending ranking (subscriptions only) |
| `search.rs` | yt-dlp search through `ytdlp` (results show as they come) and flat-entry parsing |
| `perf.rs` | the `MATINEE_PERF=1` diagnostics: marks, timed spans, frame report |
| `resolve.rs` | channel ids from links and @handles; Takeout, NewPipe, FreeTube and OPML import |
| `store.rs` | `~/.local/share/matinee/state.json`; every field has a serde default |
| `tools.rs` | finds yt-dlp (prefers ~/.local/bin) and mpv; browser detection and the sign-in arguments |
| `util.rs` | formatting helpers; `explain_ytdlp` turns yt-dlp errors into plain words |
| `player.rs` | external mpv ("Open in mpv"); reads mpv's output to explain failures |
| `style.rs`, `data/style.css` | light/dark palette and all CSS (embedded at build time) |
| `icons.rs` | fallback symbolic icons for themes that lack them |

## How things work, and why

- **Feed.** `gtk::GridView` over a `gio::ListStore` of `BoxedAnyObject<Video>`;
  only on-screen cards exist. Every card is exactly the same height (title
  `min-height: 2.5em`, meta `1.4em`): variable heights made the grid
  re-estimate rows and jump while scrolling. Thumbnails are requested for
  cards within one screen (that pass runs at most every 80 ms) and dropped
  beyond three screens. A view's list goes into the grid in slices
  (`fill_grid`: the first screens at once, then 200 per idle moment);
  appends (older uploads, search results) queue behind pending slices.
- **Start-up.** `Win::start` shows the loading page with the greeting, and
  fills the feed, then the channel list, in idles after the first frame
  (`fill_in`). Measured in the hidden test desktop: GTK's own init is 1.3 s
  and the first frame 300 ms (shader compile with no cache); Matinee's own
  work is under 60 ms. The owner's desktop numbers come from `MATINEE_PERF`.
- **Checking for new uploads.** Automatic checks happen only while the feed
  is on screen in the focused window (a 60 s tick, window focus, and leaving
  the player all call `refresh_if_due`), at most every `refresh_minutes`
  (default 20). `last_refresh` survives restarts, so reopening Matinee
  doesn't refetch every channel. Watched marks and resume points are pruned
  only for videos leaving the feed (`Store::forget_leaving`).
- **Player.** mpv renders into a `GtkGLArea` through the render API. libmpv
  allows one render context per mpv handle, so picture in picture moves it:
  set `vid=no`, free the old context with its GL context current, create the
  new one, set `vid=auto`. mpv and its renderer are created at the warm-up
  2 s after launch (on the unmapped page: creating the renderer took 180 ms
  on the owner's machine) and the renderer is kept between videos; loading
  starts only after the page's `shown` signal so the slide never waits.
  mpv's compiled shaders are cached in `~/.cache/matinee/shaders`.
  The waiting poster is a 32×18 downscale stretched by the GPU (looks
  blurred, costs nothing) and is hidden once video plays. The seek bar and
  clock refresh at most 4 times a second and only while visible. Screen idle
  is inhibited during playback; `audio-fallback-to-null=yes`. Hardware
  decoding is `hwdec=nvdec-copy,vaapi-copy,auto-copy-safe` (an older libmpv
  gets just the last): on the owner's Intel + RTX 5070 Ti machine mpv's own
  `auto-copy-safe` picked `vulkan-copy`, 30–40% slower than `nvdec-copy` in
  a local 4K benchmark and possibly on the Intel chip. Zero-copy interop
  (`nvdec,vaapi,auto-safe`) only with `MATINEE_HWDEC_INTEROP=1`; `nvdec`
  worked with NVIDIA's GL in the hidden desktop, the owner's screen decides.
- **Quality.** `ytdl-format` (`tools::ytdl_format`) is set per load, for
  the embedded player and the external mpv alike, and prefers SDR:
  `[dynamic_range=?SDR]` first, since yt-dlp's default sort would pick the
  HDR stream at 4K (heavier, and tone-mapped). "Match my screen" (the
  default) uses the monitor's physical height (logical height times the
  surface's fractional scale): at least 2000 px gives 2160, at least 1350
  gives 1440, otherwise 1080. Changing quality reloads from the current
  position.
- **yt-dlp.** Used for playback (mpv's ytdl_hook), "Open in mpv", search,
  channel lookup and channel history. Recommended setup on the owner's
  machine: the standalone `yt-dlp_linux` in ~/.local/bin (update with
  `~/.local/bin/yt-dlp -U`) plus Deno in ~/.local/bin, with
  `--js-runtimes deno:$HOME/.local/bin/deno` in `~/.config/yt-dlp/config`;
  current yt-dlp needs a JavaScript runtime for YouTube.
- **Browser sign-in.** When YouTube refuses a connection, every player type
  answers LOGIN_REQUIRED; only a signed-in request gets through. The sign-in
  is borrowed from a browser (Preferences, Playback, or "Choose a browser…"
  on the player's problem card and on toasts): `--cookies-from-browser` for
  playback, embedded and external. Search and channel history go without it
  and retry with it only after a bot check, since a signed-in search lands in
  the account's search history. `tools::BROWSERS` maps each browser to its
  profile folders; Flatpak and Snap installs and Firefox forks are passed as
  `family:/path`, and mpv gets the value length-quoted (`%n%`). Firefox is
  the reliable one: Chromium browsers encrypt sign-ins with the desktop's
  password store. The owner's browsers are mostly Flatpaks. Claude must not
  read browser cookie stores itself (the safety check blocks it, rightly);
  the owner verifies sign-in by playing a video.
- **Trending.** YouTube retired its public Trending page in 2025, so Trending
  ranks subscription uploads from the last 14 days by
  `ln((views+10)/(expected+10)) - 0.07*days + 0.12*log10(views+1)`, where
  expected is the channel's median views scaled by `1 - e^(-hours/36)`.
- **Search.** `win.search` (a string action, also used by the hidden-desktop
  tests) streams results through `ytdlp`; the first batch renders the page,
  later ones append. Entries that arrive together are sent together (80 ms
  lull); a bot check with no results retries with the sign-in.
- **State.** Saves are batched (700 ms) and forced on close. New settings use
  `Option` or negative booleans (`no_resume`) so older state files load.

## Testing

There are no automated UI tests. After UI changes, run the app and ask the
owner to try them; for video, check windowed, full screen (4K) and picture in
picture. During the original build Claude worked in a sandbox with Xvfb,
software OpenGL and a mock YouTube; software GL showed artifacts real GPUs
don't, so trust the owner's machine over sandbox screenshots.

To see the UI without disturbing the owner's desktop or data, run it in an
invisible KWin: `dbus-run-session -- kwin_wayland --virtual --socket NAME
--width 1600 --height 1400 --no-lockscreen --exit-with-session SCRIPT`, with
`HOME` and the `XDG_*` folders pointed at a throwaway directory holding a
hand-made `state.json` (set `last_refresh` to now so it fetches nothing).
Inside SCRIPT, start `target/debug/matinee`, trigger window actions with
`gdbus call --session --dest io.github.matinee.Matinee --object-path
/io/github/matinee/Matinee/window/1 --method org.gtk.Actions.Activate
play "[<'VIDEO_ID'>]" '{}'` (or `preferences`, `sign-in`, `show-channel`,
`search`), and capture with `spectacle -b -n -f -o shot.png`. That desktop
drew with the owner's NVIDIA GPU at 60 Hz, so GL and decoding checks are
meaningful there; frame rates at 4K 240 Hz are not. A copy of the owner's
real `state.json` (with `last_refresh` set to now) makes a realistic feed.

## Ideas not done yet

- An in-app "Update yt-dlp" button (download yt-dlp_linux into ~/.local/bin).
- A PO token provider (the bgutil yt-dlp plugin) might let playback through
  without a sign-in; untested, and another moving part to maintain.
- Link the About dialog to the repository; the app id could become
  `io.github.Mxrphyyr.Matinee`.
- ARM64 release builds; a Flatpak.
