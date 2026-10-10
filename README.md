# Matinee

*By Mxrphyy*

A private, ad-free desktop app for following YouTube channels on Linux.
Written in Rust with GTK 4 and libadwaita.

- **No account, no ads, no tracking.** Matinee never signs in to Google. New
  videos come from YouTube's public per-channel feeds, and videos play right
  in the window through libmpv, the engine inside mpv.
- **Your subscriptions, brought over once.** Import from Google Takeout,
  NewPipe, FreeTube or OPML, or add channels by link or `@handle`.
- **Made to feel alive.** Feeds arrive in a diagonal wave and thumbnails
  fade up out of the dark, and the mouse wheel glides instead of jumping. Cards sink when pressed and
  spring back. Opening a video dims its thumbnail while the stream loads,
  then fades the picture up. New uploads are offered with a button instead of reshuffling
  the grid while you browse. All motion switches off when your desktop asks
  for reduced animation.

---

## Contents

1. [What you need](#1-what-you-need)
2. [Install on AerynOS](#2-install-on-aerynos)
3. [Notes for KDE Plasma](#3-notes-for-kde-plasma)
4. [Bring over your subscriptions](#4-bring-over-your-subscriptions)
5. [Using Matinee](#5-using-matinee)
6. [What Matinee sends over the network](#6-what-matinee-sends-over-the-network)
7. [Keeping playback working](#7-keeping-playback-working)
8. [Settings and environment variables](#8-settings-and-environment-variables)
9. [Troubleshooting](#9-troubleshooting)
10. [Updating and uninstalling](#10-updating-and-uninstalling)
11. [Other distributions](#11-other-distributions)
12. [For developers](#12-for-developers)

---

## 1. What you need

| What | Why | Minimum |
|---|---|---|
| Rust (`cargo`, `rustc`) | builds Matinee | 1.85 |
| A C compiler (`clang` or `gcc`) | the TLS library and the final link step | any recent |
| An archiver (`llvm-ar` or `ar`) | bundles the TLS library's compiled C code | any recent |
| `pkg-config` (`pkgconf`) | finds GTK and libadwaita | any |
| GTK 4, with development files | the user interface | 4.14 |
| libadwaita, with development files | adaptive layout, dialogs, animations | 1.5 |
| libmpv, with development files | plays video inside the window | 2.0 (mpv 0.35) |
| mpv | only for "Open in mpv" | any recent |
| yt-dlp | lets mpv stream from YouTube, and powers search | keep it current |

Matinee itself never touches `/usr`. The install script puts everything under
your home folder (`~/.local`), which suits AerynOS's moss-managed system.

---

## 2. Install on AerynOS

> I couldn't test on AerynOS itself. Package names below follow moss's
> provider syntax, which asks for *what a package provides* rather than its
> exact name. If moss rejects a name, step 2b shows how to find the right one,
> and `install.sh` tells you exactly what is still missing.

### 2a. Update the system

```sh
sudo moss sync -u
```

### 2b. Install the build tools and libraries

```sh
sudo moss install 'binary(cargo)' 'binary(clang)' 'binary(llvm-ar)' 'binary(pkg-config)' \
                  'pkgconfig(gtk4)' 'pkgconfig(libadwaita-1)' 'pkgconfig(mpv)'
```

The `pkgconfig(...)` entries pull in GTK's and libadwaita's development files,
along with everything they depend on (GLib, Pango, Cairo and so on).

If moss doesn't accept that form, install by name instead. The development
files usually live in a `-devel` package:

```sh
sudo moss install rust clang llvm pkgconf gtk4-devel libadwaita-devel mpv-devel
```

Still not found? Search, then install whatever name moss shows:

```sh
moss search gtk4
moss search adwaita
moss search cargo
```

**Rust version check.** Run `cargo --version`. If it is older than 1.85, use
rustup instead of the system package. It installs into your home folder, so it
doesn't conflict with moss:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

### 2c. Install mpv and yt-dlp

```sh
sudo moss install 'binary(mpv)' 'binary(yt-dlp)'
```

If yt-dlp isn't packaged, use its official standalone build. It lives in your
home folder and can update itself:

```sh
mkdir -p ~/.local/bin
curl -L https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux \
     -o ~/.local/bin/yt-dlp
chmod +x ~/.local/bin/yt-dlp
```

Matinee looks for yt-dlp in `~/.local/bin` even when that folder isn't on
your `PATH`, and passes its location to mpv.

### 2d. Build and install Matinee

Unpack the project and run the installer from inside its folder:

```sh
tar xf matinee.tar.gz
cd matinee
./install.sh
```

(If your file manager unpacked it and `./install.sh` says *permission denied*,
run `bash install.sh` instead.)

To check requirements without building anything, run `./install.sh --check`.

The script:

1. checks every requirement above and names anything missing;
2. maps the GNU tool names Rust's build helpers expect (`cc`, `ar`, `ld`) to
   LLVM's (`clang`, `llvm-ar`, `lld`) when they're missing, as they can be on
   AerynOS;
3. builds an optimized binary with the exact dependency versions in
   `Cargo.lock`. This takes a few minutes the first time;
4. installs:
   - `~/.local/bin/matinee`
   - `~/.local/share/applications/io.github.matinee.Matinee.desktop`
   - `~/.local/share/icons/hicolor/scalable/apps/io.github.matinee.Matinee.svg`
5. refreshes Plasma's application menu so Matinee shows up right away.

Then open Matinee from the application launcher (it's in **Multimedia**), or
run `~/.local/bin/matinee`.

---

## 3. Notes for KDE Plasma

Matinee runs natively on Plasma 6, on Wayland and on X11. A few things are
different from GNOME:

- **Title bar.** libadwaita apps draw their own title bar with built-in
  window buttons, so Matinee's buttons look slightly different from Breeze's.
  Plasma's button order is respected.
- **Light or dark.** Matinee follows the color scheme Plasma reports. With a
  dark scheme such as Breeze Dark you get the dark aubergine-and-gold look;
  with a light scheme you get the light version. To keep it dark regardless,
  turn on **Always use dark style** in Matinee's main menu.
- **Icons.** Matinee uses your Breeze icons. For any icon Breeze doesn't have,
  it falls back to small built-in icons, so you won't see broken images.
- **File picker.** The import dialog is GTK's. For Plasma's own file dialog,
  start Matinee with `GDK_DEBUG=portals` (see
  [section 8](#8-settings-and-environment-variables) for making that
  permanent).
- **Reduced motion.** Plasma's *Animation speed* slider, set to *Instant*
  (System Settings, General Behavior), also turns off Matinee's animations.
- **Default browser.** "Open in browser" uses the browser chosen in System
  Settings, Default Applications.
- **Volume mixer.** While a video plays, Plasma's volume applet lists it as
  *Matinee*.
- **Fractional scaling** (125%, 150%) is supported. Thumbnails are fetched at
  a resolution that stays sharp at those scales.

---

## 4. Bring over your subscriptions

You only do this once. Matinee keeps its own list from then on.

### From YouTube (Google Takeout)

1. Go to **https://takeout.google.com** and sign in.
2. Click **Deselect all**.
3. Scroll to **YouTube and YouTube Music** and tick it.
4. Click **All YouTube data included**, then **Deselect all**, tick only
   **subscriptions**, and click **OK**.
5. Click **Next step**, keep *Export once* and *.zip*, and click
   **Create export**.
6. Google emails you when it's ready, usually within minutes. Download and
   unzip it.
7. In Matinee, open the main menu (☰ at the top of the sidebar) and choose
   **Import subscriptions…**. Pick:
   `Takeout/YouTube and YouTube Music/subscriptions/subscriptions.csv`

Your Google account isn't connected to Matinee in any way. The CSV is just a
list of channel IDs and names.

### From NewPipe or FreeTube

- **NewPipe:** Subscriptions tab, ⋮ menu, **Export to**, then copy the `.json`
  file to your computer.
- **FreeTube:** Settings, Data settings, **Export subscriptions**. Either the
  FreeTube (`.db`) or NewPipe (`.json`) format works.

Then use **Import subscriptions…** as above. OPML files from RSS readers work
too.

### One channel at a time

Click **+** at the top of the sidebar (or press **Ctrl+N**) and paste any of:

- a channel link: `https://www.youtube.com/@somechannel`
- a handle: `@somechannel`
- a channel ID: `UCxxxxxxxxxxxxxxxxxxxxxx`
- a link to **any video** from that channel

You can also subscribe from search results: right-click a video and choose
**Subscribe to *channel name***.

---

## 5. Using Matinee

### The window

- **Sidebar:** *All videos*, then your channels A–Z. A gold number shows how
  many new uploads a channel has.
- **Header:** a greeting with how many videos are new since your last visit,
  or the channel name and an **Unsubscribe** button when viewing one channel.
- **Filters:** *All*, *Unwatched* or *This week*, right under the greeting.
- **Grid:** the newest videos first, all the way down. Only the cards near
  the screen use memory, so even thousands of videos scroll smoothly.
- **Search bar:** searches all of YouTube, not just your channels.

### Mouse and touch

| Action | Result |
|---|---|
| Click a video | Plays it in the window and marks it watched |
| Right-click, or long-press on a touchscreen | Play, Play in separate window, Mark as watched/unwatched, Copy link, Open in browser, Go to channel or Subscribe |
| Gold **Show N new videos** button | Appears when a background check finds uploads; click to see them |
| Gold arrow, bottom-right | Glides back to the top |

### Keyboard

| Keys | Action |
|---|---|
| Ctrl+F | Search |
| Enter (in search) | Run the search |
| Esc (in search) | Clear the search and return to your feed |
| Ctrl+R or F5 | Refresh now |
| Ctrl+N | Add a channel |
| Arrow keys, then Enter | Move between videos and play one |
| Ctrl+W / Ctrl+Q | Close the window / quit |

### The player

Videos open over the browser, which keeps its place for when you come back.
The thumbnail waits, dimmed, while the stream loads; then the picture fades
up. Controls fade away after a few seconds of playback and come back when you
move the mouse.

| Keys or action | Result |
|---|---|
| Space or K, or click the picture | Play / pause |
| ← / → | Back / forward 5 seconds (presses add up) |
| J / L | Back / forward 10 seconds |
| ↑ / ↓ | Volume |
| M | Mute |
| P | Picture in picture |
| F, or double-click the picture | Full screen |
| Esc | Leave full screen, then back to your feed |
| Alt+← or the mouse's back button | Back to your feed |

The seek bar shows what has already loaded in a lighter tone. The speed
button cycles through 1×, 1.25×, 1.5×, 1.75× and 2×. **Open in mpv** (top
right) continues the same video, from the same moment, in a separate mpv
window. Volume is remembered between sessions, and the player is listed as
*Matinee* in your desktop's volume mixer.

Leave a video partway and it resumes there next time; a **Start over**
button appears beside the time for the first few seconds.

**Picture in picture** (the button beside full screen, or P) moves the video
into a small window and takes you back to your feed while it keeps playing.
Drag the picture to move the window and drag its edges to resize it. Hover
over it for play/pause, **Back to the player** and **Close**; double-click
returns to the full player too. Picking another video while it's open plays
that one in the small window.

To keep the small window above other windows on Plasma, press **Alt+F3** on it
and choose **More Actions → Keep Above Others**. To make that automatic, add a
rule in System Settings → Window Management → Window Rules: match the window
title **Picture in picture** (and class `io.github.matinee.Matinee`), then add
the property **Keep above other windows: Force, Yes**.

If a video can't play, the player says why, using yt-dlp's own explanation,
and offers **Try again** and **Open in mpv**.

### Labels on thumbnails

- **New:** uploaded since you last looked. Glows three times when it first
  appears, then stays still.
- **Watched:** played from Matinee. The thumbnail dims so unwatched videos
  stand out.
- **Gold bar along the bottom:** how far you got in a video you left partway.
  Playing it again picks up from there.
- **Bottom-left:** when it was uploaded. **Bottom-right** (search results
  only): its length. Channel feeds don't include lengths.

### Main menu

- **Import subscriptions…**
- **Hide Shorts:** removes Shorts from the feed, channel pages and search.
- **Always use dark style:** ignores the system's light/dark setting.
- **Always play in a separate window:** opens videos in mpv's own window
  instead of inside Matinee.
- **About Matinee**

Matinee checks for new uploads when it starts and every 20 minutes while open.

---

## 6. What Matinee sends over the network

Matinee sends no account details, API keys or analytics. Here is every kind of
request it makes:

| Request | When | Goes to |
|---|---|---|
| A channel's public feed (`/feeds/videos.xml`) | at start, every 20 minutes, and on Refresh | youtube.com |
| Thumbnails | when a video first appears; cached on disk for 30 days | i.ytimg.com |
| A channel's public page | only when you add a channel by link or `@handle` | youtube.com |
| Search | when you search, through yt-dlp | youtube.com |
| Video streams | when you play, through libmpv and yt-dlp | googlevideo.com |

The only cookie sent is `SOCS=CAI`, which *declines* YouTube's cookie-consent
prompt in the EU. Videos never load YouTube's web player, so there are no ads
and no watch-time reporting.

What this can't hide: YouTube's servers still see your IP address, as with any
website. Use a VPN or Tor if that matters to you.

Everything Matinee remembers stays on your computer:

| File | Contents |
|---|---|
| `~/.local/share/matinee/state.json` | your channels, the last feed, watched and seen videos, resume positions, window size, settings |
| `~/.cache/matinee/thumbs/` | thumbnails; anything older than 30 days is deleted automatically |

---

## 7. Keeping playback working

YouTube changes its internals often. When it does, yt-dlp (and therefore
playback and search) can break until yt-dlp is updated, which usually happens
within days. If a video won't play, the player says so and shows yt-dlp's
own explanation. Update yt-dlp:

```sh
sudo moss sync -u        # if yt-dlp came from moss
yt-dlp -U                # if you installed the standalone build in ~/.local/bin
```

---

## 8. Settings and environment variables

Everything day-to-day is in the main menu. These variables are for special
setups:

| Variable | Effect |
|---|---|
| `MATINEE_PLAYER` | Use another player, e.g. `celluloid`, `mpv --fs`, or `flatpak run io.mpv.Mpv`. The video link is added at the end. |
| `GDK_DEBUG=portals` | Use Plasma's own file dialog for importing. |
| `GSK_RENDERER` | Matinee uses GTK's OpenGL renderer (`ngl`) because video is drawn with OpenGL; set `vulkan` to use GTK's default instead. |
| `MATINEE_HWDEC_INTEROP=1` | Experimental: zero-copy hardware decoding, which hands decoded frames straight to the GPU. |
| `MATINEE_FEED_BASE`, `MATINEE_THUMB_BASE` | Fetch feeds or thumbnails through a mirror or proxy that copies YouTube's paths. Mainly for testing. |

To make a variable permanent for the launcher entry, edit
`~/.local/share/applications/io.github.matinee.Matinee.desktop` and change the
`Exec=` line, for example:

```ini
Exec=env GDK_DEBUG=portals MATINEE_PLAYER="mpv --fs" /home/you/.local/bin/matinee
```

---

## 9. Troubleshooting

**`install.sh` says something is missing.**
It names the missing piece. Install it with moss (section 2), then run the
script again.

**`error: linker 'cc' not found`** (when building by hand).
AerynOS builds with clang and may not have `cc`. Either use `./install.sh`,
which handles this, or:

```sh
export CC=clang AR=llvm-ar CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=clang
cargo build --release --locked
```

**`failed to find tool "ar"`** (while compiling `ring`).
The build bundles some C code with an archiver, which LLVM-based systems such
as AerynOS call `llvm-ar`. `install.sh` finds it automatically. To run the
older script or build by hand, set it yourself:

```sh
AR=llvm-ar ./install.sh
```

If `llvm-ar` isn't installed either: `sudo moss install 'binary(llvm-ar)'`.

**`Executable "ld" doesn't exist`** (at the final link step).
There's no GNU linker. `install.sh` switches to LLVM's `lld` when it finds
`ld.lld`; install it if it's missing:

```sh
sudo moss install 'binary(ld.lld)'
```

**`The system library 'gtk4' required by crate 'gdk4-sys' was not found`**
(or the same for `libadwaita-1`).
The development files are missing. Install `pkgconfig(gtk4)` and
`pkgconfig(libadwaita-1)` (section 2b). Check with:

```sh
pkg-config --modversion gtk4 libadwaita-1
```

**`package ... requires rustc 1.85` or newer.**
Your Rust is too old. Run `sudo moss sync -u`, or install rustup (section 2b).

**Videos don't play.**
First update yt-dlp (section 7). Then try a video directly in a terminal; the
output usually explains the problem:

```sh
mpv 'https://www.youtube.com/watch?v=jNQXAC9IVRw'
```

**Full-screen video stutters.**
Matinee draws video with OpenGL and uses GTK's OpenGL renderer so frames
aren't copied between graphics systems. If you set `GSK_RENDERER` yourself,
try removing it or setting `GSK_RENDERER=ngl`. You can also try
`MATINEE_HWDEC_INTEROP=1` (see section 8). If it still stutters, run
`~/.local/bin/matinee` from Konsole and send the output.

**The player says "Video can't be shown here".**
Matinee draws video with OpenGL, and it isn't working on this system, which
can happen with some older or virtual graphics drivers. Turn on **Always play
in a separate window** in the main menu to use mpv's own window instead.

**The build stops with "Matinee needs libmpv".**
Install libmpv's development files: `sudo moss install 'pkgconfig(mpv)'`.

**"Search needs yt-dlp."
yt-dlp isn't installed or can't be found. Install it (section 2c).

**"Couldn't refresh *channel name*."**
That channel's feed didn't load. YouTube's feeds occasionally fail for a few
minutes; try Refresh later. If it keeps happening, the channel may have been
deleted. Open it in the sidebar and choose **Unsubscribe**.

**"Couldn't reach YouTube."**
No channel loaded at all. Check your connection, VPN or firewall.

**Matinee doesn't appear in the launcher.**
Run `kbuildsycoca6` (rebuilds Plasma's menu), or log out and back in.

**Matinee looks light but I wanted dark.**
Turn on **Always use dark style** in the main menu, or pick a dark color
scheme in Plasma.

**Thumbnails look wrong after a channel changes them.**
Delete `~/.cache/matinee/thumbs/`. They will download again.

**Start over completely.**
Quit Matinee, then delete `~/.local/share/matinee/` and `~/.cache/matinee/`.
This removes your channel list, so export or re-import it afterwards.

**See error messages.**
Run `~/.local/bin/matinee` from a terminal (Konsole) and watch its output.

---

## 10. Updating and uninstalling

**Update:** replace the project folder with the new version and run
`./install.sh` again. Your channels and settings are kept.

**Uninstall:**

```sh
./install.sh --uninstall
```

That removes the program, launcher entry and icon. To also delete your channel
list and cache:

```sh
rm -rf ~/.local/share/matinee ~/.cache/matinee
```

---

## 11. Other distributions

`install.sh` works anywhere the requirements are met. Equivalent packages:

| Distribution | Command |
|---|---|
| Fedora 40+ | `sudo dnf install cargo gcc pkgconf-pkg-config gtk4-devel libadwaita-devel mpv-devel mpv yt-dlp` |
| Arch | `sudo pacman -S rust gcc pkgconf gtk4 libadwaita mpv yt-dlp` |
| Debian 13 / Ubuntu 24.04+ | `sudo apt install cargo gcc pkg-config libgtk-4-dev libadwaita-1-dev libmpv-dev mpv yt-dlp` |
| openSUSE Tumbleweed | `sudo zypper install cargo gcc pkg-config gtk4-devel libadwaita-devel mpv-devel mpv yt-dlp` |

On Ubuntu 24.04 the packaged `cargo` is older than 1.85. Install
`cargo-1.85` (or newer) from the archive, or use rustup.

---

## 12. For developers

```sh
cargo run            # debug build
cargo test           # parser and formatting tests
```

| Path | What it does |
|---|---|
| `src/main.rs` | app startup and keyboard shortcuts |
| `src/window.rs` | the window, views, refresh, search, subscriptions, playback |
| `src/card.rs` | the video card and its animation states |
| `src/thumb.rs` | thumbnail drawing: fade, zoom, dimming, resume bar |
| `src/smooth.rs` | smooth mouse-wheel scrolling |
| `src/watch.rs` | the embedded player, its controls and picture in picture |
| `src/history.rs` | streams a channel's full upload history from yt-dlp |
| `src/trending.rs` | the Trending ranking |
| `src/mpv.rs` | the libmpv binding: playback engine and OpenGL drawing |
| `data/style.css` | all styling and motion |
| `src/style.rs` | the light and dark palettes |
| `src/feed.rs` | fetching and parsing channel feeds |
| `src/search.rs` | search through yt-dlp |
| `src/resolve.rs` | turning links, handles and exports into channel IDs |
| `src/thumbs.rs` | thumbnail downloading, decoding and caching |
| `src/icons.rs` | fallback icons for themes that lack some |
| `src/player.rs` | launching mpv in a separate window |
| `src/store.rs` | the local state file |
| `src/tools.rs` | finding yt-dlp outside `PATH` |

The motion design is documented at the top of `data/style.css`.

## License

GPL-3.0-or-later.


## Channels, Trending and preferences

**The channel list** on the left is a drawer. Hide or show it with the button
at the top left of the feed or with **F9**; Matinee remembers your choice. On a
narrow window it slides over the feed instead. Type in **Filter channels** to
find a channel by name, and click the **Channels** heading to fold the list
away.

**Channel pages show the whole channel.** A channel's RSS feed only lists its
15 newest videos, so when you open a channel Matinee asks yt-dlp for the
channel's full Videos tab and adds older uploads as they arrive: the header
counts them ("132 videos so far, loading older ones"). Dates on older videos
are approximate, as YouTube only says "5 months ago". Lists are kept for the
session, so going back to a channel is instant.

**Trending** ranks your subscriptions' uploads from the last two weeks by how
unusually well each is doing *for its own channel*: a video with ten times its
channel's usual views at the same age ranks high, so a small channel's
breakout can sit above a huge channel's ordinary upload. It only uses view
counts Matinee already has, so it sends nothing anywhere. (YouTube retired its
public Trending page in 2025, so there is no global list to show.)

**Video quality.** The button beside the speed shows what's playing (720p,
1080p, 4K) and lets you pick another; the video reloads from the same moment.
**Match my screen**, the default, streams up to your monitor's resolution, so
a 4K monitor gets 4K. 4K streams are VP9 or AV1, which want hardware decoding
for smooth playback.

**Preferences** (main menu, or **Ctrl+,**): dark style, card size, Hide
Shorts, how often to check for new videos, default quality, resuming, playing
in a separate window, smooth scrolling, and the channel list.
