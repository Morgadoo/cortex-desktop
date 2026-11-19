#!/usr/bin/env bash
# The updater compares the manifest version against the version compiled into the
# app from tauri.conf.json. If the three hand-maintained versions drift, a release
# ships that no installed client will ever consider newer.
#
# plutil parses JSON as well as plists; jq is not installed on the macOS runners.
set -euo pipefail

cd "$(dirname "$0")/.."

fail() {
    echo "validate-version: $1" >&2
    exit 1
}

conf_version=$(plutil -extract version raw src-tauri/tauri.conf.json) ||
    fail "could not read version from src-tauri/tauri.conf.json"
pkg_version=$(plutil -extract version raw package.json) ||
    fail "could not read version from package.json"
cargo_version=$(awk '
    /^\[package\]/ { in_package = 1; next }
    /^\[/          { in_package = 0 }
    in_package && /^version[[:space:]]*=/ {
        gsub(/["[:space:]]/, "")
        sub(/^version=/, "")
        print
        exit
    }
' src-tauri/Cargo.toml)

echo "tauri.conf.json=$conf_version package.json=$pkg_version Cargo.toml=$cargo_version"

[ -n "$conf_version" ] || fail "tauri.conf.json version is empty"
[ "$conf_version" = "$pkg_version" ] ||
    fail "package.json version ($pkg_version) != tauri.conf.json ($conf_version)"
[ "$conf_version" = "$cargo_version" ] ||
    fail "src-tauri/Cargo.toml version ($cargo_version) != tauri.conf.json ($conf_version)"

# The bundler runs in build-macos-unsigned, before cortex-mac:sign, so any updater
# artifact it produced would contain the unsigned app. package-macos builds the
# archive from the signed bundle instead; see scripts/make-update-archive.sh.
if grep -q createUpdaterArtifacts src-tauri/tauri.conf.json; then
    fail "bundle.createUpdaterArtifacts must not be set: the bundler runs before signing"
fi

# `tauri add updater` injects updater:default here. capabilities/default.json is
# windows:["*"] with remote.urls, so that would hand the update trigger to the
# remote webapp loaded in cortex-window.
if grep -q '"updater:' src-tauri/capabilities/default.json; then
    fail "updater plugin permissions must not be in the remote-exposed default capability"
fi

# Needed to test against a local http:// manifest, and trivially left behind. The
# https requirement is a real defence for a manifest served from a host outside
# our control, so treat these as never-commit.
for flag in dangerousInsecureTransportProtocol dangerousAcceptInvalidCerts dangerousAcceptInvalidHostnames; do
    if grep -q "$flag" src-tauri/tauri.conf.json; then
        fail "$flag must not be committed: it was only for local update testing"
    fi
done

pubkey=$(plutil -extract plugins.updater.pubkey raw src-tauri/tauri.conf.json) ||
    fail "plugins.updater.pubkey missing from src-tauri/tauri.conf.json"
case "$pubkey" in
    REPLACE_WITH_*) fail "plugins.updater.pubkey is still the placeholder" ;;
esac

echo "validate-version: ok"
