## Install

Download **matinee-{{VERSION}}-x86_64-linux.tar.gz** below, then:

```
tar xf matinee-{{VERSION}}-x86_64-linux.tar.gz
cd matinee-{{VERSION}}-x86_64-linux
./install.sh
```

Matinee installs for your user only (into `~/.local`) and appears in your app launcher. `./install.sh --uninstall` removes it again.

It needs GTK 4.14+, libadwaita 1.5+ and libmpv (mpv 0.35+), plus yt-dlp for search and playback. Keep yt-dlp up to date, since YouTube changes often. The installer names anything that's missing.

| System | Command |
|---|---|
| AerynOS | `sudo moss install 'binary(mpv)' 'binary(yt-dlp)' 'pkgconfig(libadwaita-1)' 'pkgconfig(mpv)'` |
| Arch | `sudo pacman -S gtk4 libadwaita mpv yt-dlp` |
| Fedora 40+ | `sudo dnf install gtk4 libadwaita mpv-libs yt-dlp` |
| Ubuntu 24.04+, Debian 13 | `sudo apt install libadwaita-1-0 libmpv2 yt-dlp` |

This build is made on Ubuntu 24.04. Older systems, such as Ubuntu 22.04 or Debian 12, can't run it.

If videos say YouTube wants to check you're not a bot, see Preferences, then Playback.

To verify the download: `sha256sum -c matinee-{{VERSION}}-x86_64-linux.tar.gz.sha256`
