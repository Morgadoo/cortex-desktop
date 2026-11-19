#!/usr/bin/env bash
# Build the Tauri updater payload for a macOS .app: a gzipped tar of the bundle,
# verified after a round trip. scripts/make-update-manifest.sh signs it.
#
# Usage:
#   scripts/make-update-archive.sh <path-to-.app> [output-dir]
#
# Environment:
#   SKIP_GATEKEEPER_CHECKS=1        skip codesign/stapler/spctl (local unsigned builds)
set -euo pipefail

app_path=${1:?usage: make-update-archive.sh <path-to-.app> [output-dir]}
out_dir=${2:-.}

fail() {
    echo "make-update-archive: $1" >&2
    exit 1
}

[ -d "$app_path" ] || fail "no such app bundle: $app_path"
mkdir -p "$out_dir"

app_name=$(basename "$app_path")
app_parent=$(cd "$(dirname "$app_path")" && pwd)
out_dir=$(cd "$out_dir" && pwd)

version=$(plutil -extract CFBundleShortVersionString raw "$app_path/Contents/Info.plist")
build=$(plutil -extract CFBundleVersion raw "$app_path/Contents/Info.plist")
archive="$out_dir/Cortex-$version-$build.app.tar.gz"

# The plugin's macOS installer hardcodes tar.gz and strips the first path
# component of every entry, so the archive needs exactly one top-level component
# and no "./" prefix. COPYFILE_DISABLE stops macOS tar emitting AppleDouble "._*"
# siblings, which would add a second top-level entry.
echo "==> archiving $app_path"
COPYFILE_DISABLE=1 tar -czf "$archive" -C "$app_parent" "$app_name"

roots=$(tar -tzf "$archive" | sed 's|/.*||' | sort -u)
[ "$roots" = "$app_name" ] || fail "archive must have exactly one root entry, got: $roots"
if tar -tzf "$archive" | grep -qE '(^|/)\._'; then
    fail "AppleDouble entries leaked into the archive"
fi

# The extracted copy is what users actually run, so verify that rather than the
# source bundle: this is the check the built-in bundler flow never performs.
echo "==> verifying round trip"
verify_dir=$(mktemp -d)
trap 'rm -rf "$verify_dir"' EXIT
tar -xzf "$archive" -C "$verify_dir"
extracted="$verify_dir/$app_name"

[ -x "$extracted/Contents/MacOS/cortex-desktop" ] || fail "executable bit lost in the archive"

if [ "${SKIP_GATEKEEPER_CHECKS:-}" = "1" ]; then
    echo "    skipped (SKIP_GATEKEEPER_CHECKS=1)"
else
    codesign --verify --deep --strict --verbose=2 "$extracted"
    xcrun stapler validate "$extracted"
    spctl -a -vvv -t exec "$extracted"
fi

echo "==> $archive"
