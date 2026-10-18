#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
appimage_arch="${ROM_SCRAPPER_ARCH:-$(uname -m)}"
appimage_tool="${APPIMAGE_TOOL_BIN:?Set APPIMAGE_TOOL_BIN to the matching appimagetool executable}"
appimage_runtime="${APPIMAGE_RUNTIME_FILE:?Set APPIMAGE_RUNTIME_FILE to the matching AppImage runtime}"

case "$appimage_arch" in
    x86_64|aarch64) ;;
    *) echo "Unsupported AppImage architecture: $appimage_arch" >&2; exit 2 ;;
esac

cd "$repo_root"
cargo build --release --locked

appdir="$repo_root/dist/Miyoo-ROMScrapper.AppDir"
mkdir -p "$appdir/usr/bin"
install -m 755 target/release/rom-art-rs "$appdir/usr/bin/miyoo-romscrapper"
install -m 755 packaging/linux/AppRun "$appdir/AppRun"
install -m 644 packaging/linux/miyoo-romscrapper.desktop "$appdir/miyoo-romscrapper.desktop"
install -m 644 assets/miyoo-romscrapper.png "$appdir/miyoo-romscrapper.png"
install -m 644 assets/miyoo-romscrapper.svg "$appdir/miyoo-romscrapper.svg"

mkdir -p "$repo_root/dist"
APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$appimage_arch" "$appimage_tool" \
    --no-appstream --runtime-file "$appimage_runtime" \
    "$appdir" "$repo_root/dist/Miyoo-ROMScrapper-${appimage_arch}.AppImage"
