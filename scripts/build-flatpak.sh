#!/usr/bin/env bash
# Builds Cortex as a Flatpak bundle from the .deb the Linux build makes.
#
#   npx tauri build --bundles deb
#   scripts/build-flatpak.sh src-tauri/target/release/bundle/deb/cortex_*_amd64.deb [Cortex.flatpak]
#
# Needs flatpak and flatpak-builder, with Flathub as a remote (`flatpak remote-add --user
# --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo`); the GNOME runtime and
# SDK are installed from it on first use. Install the bundle with `flatpak install Cortex.flatpak`.
set -euo pipefail

deb=$(realpath "${1:?usage: scripts/build-flatpak.sh path/to/cortex.deb [out.flatpak]}")
out=$(realpath -m "${2:-Cortex.flatpak}")
dir=$(cd "$(dirname "$0")/../flatpak" && pwd)
app=io.github.morgadoo.cortex-desktop
# Flathub's shared modules (the tray icon's libayatana-appindicator), at a known commit.
shared=d3a85ee07ae4ff98fe7561f958ac58ac9e6e39bd

cp "$deb" "$dir/cortex.deb"
if [ ! -d "$dir/shared-modules/.git" ]; then
  rm -rf "$dir/shared-modules"
  git clone --quiet https://github.com/flathub/shared-modules "$dir/shared-modules"
fi
git -C "$dir/shared-modules" fetch --quiet origin "$shared" 2>/dev/null || true
git -C "$dir/shared-modules" -c advice.detachedHead=false checkout --quiet "$shared"

user=()
[ "$(id -u)" -ne 0 ] && user=(--user)
flatpak-builder "${user[@]}" --force-clean --install-deps-from=flathub --disable-rofiles-fuse \
  --state-dir="$dir/.flatpak-builder" --repo="$dir/repo" "$dir/build" "$dir/$app.yml"
flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
  "$dir/repo" "$out" "$app"
echo "build-flatpak: $out"
