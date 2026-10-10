# How Matinee got here

A condensed history of the claude.ai conversation in which Matinee was
designed and built (October 2026). `CLAUDE.md` has the rules; this file has
the reasons.

## 0.1: the first version

- The owner wanted a private, ad-free way to follow YouTube channels on
  AerynOS and KDE Plasma, without a Google account.
- The feed comes from each channel's RSS (Atom) feed, fetched in parallel.
  Subscriptions are imported once (Google Takeout CSV, NewPipe, FreeTube,
  OPML) or added by link or @handle.
- The first build failed on AerynOS, which has no GNU `ar`. `install.sh` now
  maps cc, ar and ld to clang, llvm-ar and lld when the GNU tools are missing.

## The embedded player

- libmpv's OpenGL render API inside a `GtkGLArea` (own FFI in `mpv.rs`, with
  EGL/GLX proc lookup), Matinee's own controls, resume where you left off
  (more than 20 s watched, more than 30 s left, under 95% done), and "Open in
  mpv" as a fallback.

## The performance pass

- The feed moved from a FlowBox to a virtualized GridView with recycled
  cards. Thumbnails load only near the screen and are dropped far away. All
  CSS blur was removed (the main cause of stutter), thumbnail effects are
  drawn by a custom Paintable, wheel scrolling glides, saves are batched.
- Benchmark (2,100-video feed, 600 wheel notches, same machine): scrolling
  CPU time went from 35.0 s to 20.6 s, and memory growth from +296 MB
  (unbounded) to +96 MB (then flat).

## 0.2: full screen and picture in picture

- Full-screen video stuttered after 10 to 15 seconds. Causes fixed: a
  blurred full-size poster still drawn behind the video, the clock and seek
  bar updating on every frame (each one a relayout), and frames copied from
  OpenGL to GTK's Vulkan renderer (Matinee now defaults to `GSK_RENDERER=ngl`).
  The screen is kept awake while playing.
- The slide into the player stuttered because mpv, OpenGL and the renderer
  all started as it began. They now start after it lands, and mpv is warmed
  up at launch.
- Picture in picture: a small window; the single mpv renderer moves between
  surfaces. Plasma doesn't let apps keep themselves on top, so the README
  explains a KWin window rule instead.
- Zero-copy hardware decoding was tried and made opt-in
  (`MATINEE_HWDEC_INTEROP=1`) after it gave artifacts in testing.

## 0.3.0: browsing, quality, Trending, releases

- The channel list became a drawer (F9) with a filter and a foldable
  Channels section, and "By Mxrphyy" appeared under the name.
- Channel pages show a channel's full history (RSS lists only 15 videos).
- A quality menu was added; the default matches the screen, so 4K screens
  get 4K (it was capped at 1080p before).
- Scrolling sometimes jumped back up. Cause: cards of different heights made
  the grid re-estimate rows. Fix: every card is the same height.
- Trending: YouTube retired its Trending page in 2025, so Matinee ranks
  subscription uploads by how far each beats its channel's norm.
- A Preferences dialog.
- GitHub: publish and sync scripts, then the release workflow, `release.sh`
  and a prebuilt-binary mode in `install.sh`. v0.3.0 was the first automatic
  release.
- Lessons: the owner edits README.md on GitHub, so updates never include it.
  The sync script pulls (rebase) before pushing, and runs as one function
  because a pull can rewrite the script while it runs. A stuck local clone
  (detached HEAD) was fixed by cloning afresh.

## 0.3.1: YouTube's bot check

- After 0.3.0, playback failed with "Sign in to confirm you're not a bot".
  Most likely cause: reading whole channel histories at once, hundreds of
  requests per channel. Channel history now reads 60 uploads per scroll
  request with `--sleep-requests 1`; errors are explained in plain words; an
  optional browser sign-in (cookies) was added; Matinee prefers
  `~/.local/bin/yt-dlp`.
- The owner was given steps to install the standalone yt-dlp and Deno in
  ~/.local/bin (current yt-dlp needs a JavaScript runtime for YouTube).
- Development then moved to Claude Code, with `CLAUDE.md` and this file
  carrying the context over.

## 0.3.2: the bot check, part two (Claude Code)

- Playback still failed with "Sign in to confirm you're not a bot". Tested
  from the owner's machine: every player type yt-dlp can ask for (visionos,
  web, web_safari, tv, tv_simply, mweb, web_embedded, android_vr) answered
  LOGIN_REQUIRED or UNPLAYABLE, even for a 2005 video. So no setting or
  yt-dlp update gets around it (2026.08.19 was the latest release). Search
  and channel listings still worked without a sign-in.
- Likely contributor: the owner follows 271 channels, and Matinee checked
  every feed at launch and every 20 minutes, eight at once: about 800
  requests an hour while open. Now checks run only while the feed is on
  screen, the last check time survives restarts, and checks after the
  first load go four at a time with pauses.
- The 0.3.1 sign-in only knew where regular installs keep their profiles,
  and most of the owner's browsers are Flatpaks. Matinee now finds regular,
  Flatpak and Snap installs and Firefox forks, lists only those, and points
  yt-dlp at the right folder. Firefox is recommended: Chromium browsers
  encrypt sign-ins with the desktop's password store.
- The sign-in is used for playback. Search and channel pages ask without
  it first, because a signed-in search lands in the account's history.
- Refusals are explained with a "Choose a browser…" button (player and
  toasts), including failures in mpv's own window.
- Bugs fixed: resume points and watched marks of videos outside the feed
  were erased at every refresh; a failed channel-history read counted as
  complete, so it never retried; the separate mpv window ignored the quality
  setting and marked videos watched even when they never played; "Match my
  screen" overestimated with fractional scaling; a damaged cached thumbnail
  never reloaded; GTK 4.18+ warned that `ngl` is now called `gl`.
- Claude tested the UI in an invisible KWin session with a throwaway home
  folder (see CLAUDE.md, Testing). It did not read the owner's browser
  cookie stores; the owner checks the sign-in by playing a video.
- The owner chose Firefox and playback worked again, while YouTube still
  refused the same connection without a sign-in. No prompt appears: yt-dlp
  reads Firefox's saved sign-in each time.
- The README gained the bot-check steps (section 7) and Deno, which current
  yt-dlp needs for YouTube.

## 0.4.0: speed and stability

- The owner reported slow start-up, stutter when scrolling and switching
  views, player stutter with artefacts in some videos, and slow searches,
  and chose features for later rounds (see CLAUDE.md's roadmap note in
  memory): 0.5.0 player features, 0.6.0 browsing features.
- Measured on the owner's real data (3,842 videos, 271 channels) in the
  hidden test desktop: loading and saving the 1 MB state file 1–2 ms,
  Trending 0.3 ms, the thumbnail pass under 1 ms, the channel list 38 ms,
  filling the grid 20–33 ms (GTK binding ~80 visible cards is the floor),
  GTK's own init 1.3 s and the first frame 300 ms (shader compile without
  a cache). The screen is 4K at 240 Hz with 145% scaling, so a frame is
  4 ms there.
- Changes: the window shows before the feed and channel list are built; the
  grid fills in slices; search results stream in (first page in about
  1.5 s instead of all 40 after 2 s); a feed fetch is retried once; a crash
  log is written and noted at the next start; `MATINEE_PERF=1` diagnostics
  so the owner can report real-desktop numbers; About links to GitHub;
  `install.sh --check` warns about missing Deno.
- Player: the machine has an Intel Arrow Lake chip and an RTX 5070 Ti.
  mpv's `auto-copy-safe` chose `vulkan-copy`; a local 4K benchmark (H.264,
  VP9, AV1 test clips) showed `nvdec-copy` 30–40% faster and `vaapi-copy`
  slowest, so the decoder order is now nvdec-copy, vaapi-copy, then auto.
  yt-dlp's default sort prefers HDR at 4K, which needs tone mapping and
  can look wrong: streams are now chosen SDR first. Zero-copy `nvdec` works
  with NVIDIA's GL in the hidden desktop; whether it should be the default
  depends on the owner's test.
- The owner found start-up "really fast" but resume doing nothing. Cause:
  Ctrl+Q called `app.quit()`, which ends the program without the window's
  close-request, so the resume spot, the window size and any unsaved change
  were lost. Ctrl+Q now closes the main window. A spot is kept from 5 s
  watched (was 20 s) while more than 20 s remain.
- The owner's first `MATINEE_PERF=1` run (4K, 240 Hz, 145% scaling):
  window built at 108 ms, first frame 190 ms, feed on screen 230 ms; while
  scrolling, 1,200 frames per 5 s with the worst at 8–9 ms; a 1080p60 AV1
  video decoded by `nvdec-copy` with 1 frame dropped at the start, none
  late, never waiting for data. Two hitches: 54 ms when the channel list was
  built during the feed's entrance (now 40 rows per idle moment), and
  270 ms when the video opened (the player's first frame compiles shaders;
  now `gpu-shader-cache-dir` keeps them on disk, and the first draw is
  timed).
- The second run played 4K60 VP9 through `nvdec-copy`: 1 frame dropped at
  the start, none late, never waiting for data, steady frames within
  8–12 ms. Opening a video still stalled twice: 180 ms creating mpv's
  renderer after the slide, and 260 ms compiling shaders at the first real
  frame (the cache was empty on that first run). The renderer is now created
  at the 2 s warm-up on the unmapped page and kept between videos, which
  changes the 0.2-era rule "OpenGL starts after the slide lands": the GL
  context exists before the slide, but no video work happens during it.
- A regression caught by the hidden desktop's screenshots: calling
  `app.register()` before `connect_startup` (for an "already running"
  notice) fired `startup` with no handlers, so the CSS and fallback icons
  were never installed and the app ran in plain GTK styling for three test
  builds. The notice now prints after `run()` returns, when `is_remote()`
  is known. Lesson: GApplication emits `startup` inside `register`.
- The owner confirmed the look was back and playback started cleanly, and
  0.4.0 was released. Next: 0.5.0 (player features), then 0.6.0 (browsing).

## 0.5.0: the player

- Media keys and Plasma's media controls through MPRIS, written against
  gio's D-Bus API rather than a new crate: the window keeps the list a video
  was played from, so Next and Previous (and autoplay) have somewhere to go.
- Chapters as marks on the seek bar with the current chapter's name beside
  the clock; Page Up and Page Down jump. mpv's YouTube hook already sets
  `chapter-list` from yt-dlp's chapters.
- A subtitles button. The hook only adds the subtitle tracks yt-dlp was
  asked for, so Matinee asks for the desktop's language and English,
  auto-generated included, and selects the best track itself (subtitles
  start hidden).
- Autoplay next, off by default, with an "up next" card and eight seconds
  to say no. In picture in picture it plays on without the card.
- Tested in the hidden desktop on a local clip (`MATINEE_PLAY_FILE`) with
  chapters and an SRT track, driving the controls with `gdbus`. A thumbnail
  already in memory arrived before the card knew which video it was for;
  fixed by recording the video first.
- The owner tried all four on the real desktop ("everything works") and
  0.5.0 was released. Next: 0.6.0 (browsing features).
