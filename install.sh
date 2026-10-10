#!/usr/bin/env bash
# Installs Matinee for the current user only, under ~/.local. From the source
# code it builds Matinee first; a release download already holds the program.
# Nothing is written to /usr, which keeps AerynOS's moss-managed system clean.
#
#   ./install.sh               install (building first from source)
#   ./install.sh --check       only check that everything needed is installed
#   ./install.sh --uninstall   remove the program, launcher entry and icon

set -euo pipefail

APP_ID="io.github.matinee.Matinee"
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
BIN_DIR="$HOME/.local/bin"
APPS_DIR="$DATA_HOME/applications"
ICON_DIR="$DATA_HOME/icons/hicolor/scalable/apps"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MODE="${1:-install}"

say()  { printf '\033[1;33m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;35m  !\033[0m %s\n' "$*"; }
fail() { printf '\033[1;31mError:\033[0m %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

refresh_menus() {
    if have update-desktop-database; then update-desktop-database -q "$APPS_DIR" 2>/dev/null || true; fi
    # Plasma keeps its own menu cache; rebuilding it makes the entry appear immediately.
    if have kbuildsycoca6; then kbuildsycoca6 >/dev/null 2>&1 || true
    elif have kbuildsycoca5; then kbuildsycoca5 >/dev/null 2>&1 || true; fi
}

# Playback and search need these at run time.
check_tools() {
    local yt_dlp_found=false
    if have yt-dlp || [[ -x "$HOME/.local/bin/yt-dlp" ]]; then yt_dlp_found=true; fi
    # shellcheck disable=SC1111  # typographic quotes inside a message
    have mpv || warn "mpv isn't installed. Only “Open in mpv” needs it."
    $yt_dlp_found || warn "yt-dlp isn't installed. Search and playback need it (README section 2c)."
    return 0
}

# Copies the program, launcher entry and icon into ~/.local.
install_files() {
    say "Installing to $HOME/.local"
    mkdir -p "$BIN_DIR" "$APPS_DIR" "$ICON_DIR"
    cp -f "$1" "$BIN_DIR/matinee"
    chmod 755 "$BIN_DIR/matinee"
    cp -f "$HERE/data/$APP_ID.svg" "$ICON_DIR/$APP_ID.svg"
    # Use the full path: launchers often don't have ~/.local/bin on PATH.
    local exec_path="$BIN_DIR/matinee"
    if [[ "$exec_path" == *" "* ]]; then exec_path="\"$exec_path\""; fi
    sed "s|^Exec=.*|Exec=$exec_path|" "$HERE/data/$APP_ID.desktop" > "$APPS_DIR/$APP_ID.desktop"
    chmod 644 "$APPS_DIR/$APP_ID.desktop" "$ICON_DIR/$APP_ID.svg"
    refresh_menus
    say "Done. Open Matinee from your application launcher (Multimedia), or run:"
    echo "    $BIN_DIR/matinee"
    echo
    echo "    First time? Import your subscriptions: README section 4."
}

case "$MODE" in
    install|--check|--uninstall) ;;
    *) fail "Unknown option '$MODE'. Use --check or --uninstall, or no option to install." ;;
esac

if [[ "$MODE" == "--uninstall" ]]; then
    rm -f "$BIN_DIR/matinee" "$APPS_DIR/$APP_ID.desktop" "$ICON_DIR/$APP_ID.svg"
    refresh_menus
    say "Matinee is uninstalled."
    echo "    Your channel list is still in $DATA_HOME/matinee."
    echo "    To remove it too: rm -rf \"$DATA_HOME/matinee\" \"${XDG_CACHE_HOME:-$HOME/.cache}/matinee\""
    exit 0
fi

# ── Release downloads ──────────────────────────────────────────────────
# A release holds a ready-made program beside this script and no source
# code: check that it can run here, then install it. Nothing to build.
if [[ -x "$HERE/matinee" && ! -f "$HERE/Cargo.toml" ]]; then
    say "Checking that Matinee can run here"
    if have ldd; then
        # ldd fails outright on a program it cannot load; that case is explained below.
        missing_libs="$(ldd "$HERE/matinee" 2>/dev/null | awk '/=> not found/ {print $1}' | sort -u || true)"
        if [[ -n "$missing_libs" ]]; then
            printf '\033[1;31mMissing libraries:\033[0m\n'
            while read -r lib; do
                case "$lib" in
                    libgtk-4*)     hint="GTK 4.14 or newer" ;;
                    libadwaita-1*) hint="libadwaita 1.5 or newer" ;;
                    libmpv*)       hint="libmpv, from mpv 0.35 or newer" ;;
                    *)             hint="" ;;
                esac
                echo "    $lib${hint:+   ($hint)}"
            done <<< "$missing_libs"
            echo
            echo "    Install these with your package manager, then run ./install.sh again."
            echo "    The release page lists the command for common systems."
            exit 1
        fi
    fi
    # The libraries exist; make sure they (and the C library) are new enough.
    if ! problem="$("$HERE/matinee" --help 2>&1 >/dev/null)"; then
        printf '\033[1;31mThis build of Matinee can'\''t run on this system:\033[0m\n'
        echo "$problem" | head -n 5 | sed 's/^/    /'
        echo
        echo "    Release builds are made on Ubuntu 24.04. On older systems, build from source (README)."
        exit 1
    fi
    check_tools
    if [[ "$MODE" == "--check" ]]; then
        say "Everything Matinee needs is installed."
        exit 0
    fi
    install_files "$HERE/matinee"
    exit 0
fi

# ── 1. Requirements ────────────────────────────────────────────────────
say "Checking requirements"
missing=()

# rustup installs into ~/.cargo/bin, which a fresh shell may not have on PATH yet.
if ! have cargo && [[ -x "$HOME/.cargo/bin/cargo" ]]; then export PATH="$HOME/.cargo/bin:$PATH"; fi

if have cargo && have rustc; then
    rust_minor="$(rustc --version | sed -E 's/^rustc 1\.([0-9]+).*/\1/')"
    if [[ "$rust_minor" =~ ^[0-9]+$ ]] && (( rust_minor < 85 )); then
        fail "Rust 1.85 or newer is needed; you have $(rustc --version | cut -d' ' -f2).
       Update with: sudo moss sync -u    (or install rustup; see README section 2b)"
    fi
else
    missing+=("Rust (cargo):            sudo moss install 'binary(cargo)'")
fi

have pkg-config || missing+=("pkg-config:              sudo moss install 'binary(pkg-config)'")

if have pkg-config; then
    if ! pkg-config --exists gtk4; then
        missing+=("GTK 4 development files: sudo moss install 'pkgconfig(gtk4)'")
    elif ! pkg-config --atleast-version=4.14 gtk4; then
        fail "GTK 4.14 or newer is needed; you have $(pkg-config --modversion gtk4)."
    fi
    if ! pkg-config --exists libadwaita-1; then
        missing+=("libadwaita development:  sudo moss install 'pkgconfig(libadwaita-1)'")
    elif ! pkg-config --atleast-version=1.5 libadwaita-1; then
        fail "libadwaita 1.5 or newer is needed; you have $(pkg-config --modversion libadwaita-1)."
    fi
    if ! pkg-config --exists mpv; then
        missing+=("libmpv development:      sudo moss install 'pkgconfig(mpv)'")
    elif ! pkg-config --atleast-version=2.0 mpv; then
        fail "libmpv 2.0 or newer (mpv 0.35+) is needed; you have $(pkg-config --modversion mpv)."
    fi
fi

# ── 2. Toolchain ───────────────────────────────────────────────────────
# Rust's build helpers expect the GNU tool names cc, ar and ld. AerynOS builds
# with LLVM and may only have clang, llvm-ar and lld, so map them when missing.
if ! have cc; then
    if have clang; then compiler=clang
    elif have gcc; then compiler=gcc
    else compiler=""; missing+=("A C compiler (clang):    sudo moss install 'binary(clang)'"); fi
    if [[ -n "$compiler" ]]; then
        export CC="$compiler"
        export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$compiler"
        export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER="$compiler"
    fi
fi

# The TLS library's C code is bundled with an archiver: `ar`, or `llvm-ar` on LLVM systems.
if [[ -z "${AR:-}" ]] && ! have ar; then
    for candidate in llvm-ar $(compgen -c llvm-ar- 2>/dev/null | sort -u); do
        if have "$candidate"; then export AR="$candidate"; break; fi
    done
    if [[ -z "${AR:-}" ]] && have clang; then
        candidate="$(clang -print-prog-name=llvm-ar 2>/dev/null || true)"
        if [[ "$candidate" == /* && -x "$candidate" ]]; then export AR="$candidate"; fi
    fi
    [[ -n "${AR:-}" ]] || missing+=("An archiver (llvm-ar):   sudo moss install 'binary(llvm-ar)'")
fi

# Without the GNU linker, link with LLVM's lld.
linker="default"
if ! have ld && have ld.lld; then
    export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-fuse-ld=lld"
    linker="lld"
fi

if (( ${#missing[@]} )); then
    printf '\033[1;31mMissing:\033[0m\n'
    printf '    %s\n' "${missing[@]}"
    echo
    echo "    Install these, then run ./install.sh again."
    echo "    If moss can't find a name, try: moss search <word>   (README section 2b)"
    exit 1
fi
echo "    Rust $(rustc --version | cut -d' ' -f2), GTK $(pkg-config --modversion gtk4), libadwaita $(pkg-config --modversion libadwaita-1), libmpv $(pkg-config --modversion mpv)"
echo "    C compiler: ${CC:-cc}   archiver: ${AR:-ar}   linker: $linker"

check_tools

if [[ "$MODE" == "--check" ]]; then
    say "Everything needed to build Matinee is installed."
    exit 0
fi

# ── 3. Build ───────────────────────────────────────────────────────────
say "Building Matinee (the first build takes a few minutes)"
cd "$HERE"
cargo build --release --locked

# ── 4. Install ─────────────────────────────────────────────────────────
install_files "$HERE/target/release/matinee"
