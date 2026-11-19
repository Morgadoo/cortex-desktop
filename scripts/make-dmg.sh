#!/usr/bin/env bash
# Package an existing Cortex.app, preserving its signature.
# Usage: bash scripts/make-dmg.sh <path-to-Cortex.app> [output-dir]
set -euo pipefail

fail() { echo "make-dmg: $1" >&2; exit 1; }

repo_root=$(cd "$(dirname "$0")/.." && pwd)
app_path=${1:?usage: make-dmg.sh <path-to-Cortex.app> [output-dir]}
out_dir=${2:-"$repo_root/build/dmg"}

[ "$(uname -s)" = Darwin ] || fail "requires macOS"
command -v create-dmg >/dev/null || fail "install create-dmg first: brew install create-dmg"
[ -d "$app_path" ] || fail "no such app bundle: $app_path"
[ "$(basename "$app_path")" = Cortex.app ] || fail "expected a bundle named Cortex.app"
version=$(plutil -extract CFBundleShortVersionString raw "$app_path/Contents/Info.plist")
build=$(plutil -extract CFBundleVersion raw "$app_path/Contents/Info.plist")
mkdir -p "$out_dir"
out_dir=$(cd "$out_dir" && pwd)
output="$out_dir/Cortex-$version-$build.dmg"
[ ! -e "$output" ] || fail "output already exists: $output"

staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT
ditto "$app_path" "$staging/Cortex.app"

# Finder automation requires a logged-in desktop session. Do not use
# --skip-jenkins/--sandbox-safe: those options skip the visual layout.
create-dmg \
  --volname "Install Cortex" \
  --volicon "$repo_root/src-tauri/icons/icon.icns" \
  --background "$repo_root/src-tauri/dmg/background.tiff" \
  --window-pos 200 120 \
  --window-size 660 400 \
  --icon-size 100 \
  --text-size 14 \
  --icon "Cortex.app" 180 190 \
  --hide-extension "Cortex.app" \
  --app-drop-link 480 190 \
  "$output" "$staging"

echo "make-dmg: $output"
