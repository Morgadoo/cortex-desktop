#!/usr/bin/env bash
# Sign updater payloads and write the latest.json for one platform. Each
# platform's manifest is served from https://lumo.protondownload.com/desktop/{{target}}/latest.json.
#
# Usage:
#   scripts/make-update-manifest.sh <payload>...
#
# Payloads: a macOS .app.tar.gz from make-update-archive.sh, a .deb / .rpm, or a
# Windows NSIS -setup.exe / .msi. Each gets a .sig beside it; latest.json is
# written to the current directory. Needs jq and the repo's node_modules.
#
# Environment:
#   CORTEX_UPDATE_BASE_URL            public dir the payloads will be served from (required)
#   CORTEX_MINIMUM_SUPPORTED_VERSION optional minimum app version; older apps must update
#   TAURI_SIGNING_PRIVATE_KEY       minisign private key, read by the tauri CLI
#   TAURI_SIGNING_PRIVATE_KEY_PASSWORD
set -euo pipefail

fail() {
    echo "make-update-manifest: $1" >&2
    exit 1
}

[ $# -gt 0 ] || fail "usage: make-update-manifest.sh <payload>..."
: "${TAURI_SIGNING_PRIVATE_KEY:?set TAURI_SIGNING_PRIVATE_KEY to the minisign private key}"
: "${CORTEX_UPDATE_BASE_URL:?set CORTEX_UPDATE_BASE_URL to the public host serving the artifacts}"

root=$(cd "$(dirname "$0")/.." && pwd)
version=$(jq -er .version "$root/src-tauri/tauri.conf.json")
base=${CORTEX_UPDATE_BASE_URL%/}
min=${CORTEX_MINIMUM_SUPPORTED_VERSION:-}
node "$root/scripts/validate-update-minimum.mjs" "$min" "$version"

# Lookup is {os}-{arch}-{installer} then {os}-{arch}, picked by the bundle type
# baked into the running binary. There is no darwin-universal fallback, so the
# one universal archive is listed under both macOS arches.
keys=()
payloads=()
os=""
for payload in "$@"; do
    [ -f "$payload" ] || fail "no such file: $payload"
    name=$(basename "$payload")
    case "$name" in
        *"$version"*) ;;
        *) fail "$name does not carry version $version" ;;
    esac
    case "$name" in
        *.app.tar.gz) payload_keys="darwin-aarch64 darwin-x86_64" ;;
        *.deb) payload_keys="linux-x86_64-deb" ;;
        *.rpm) payload_keys="linux-x86_64-rpm" ;;
        *-setup.exe) payload_keys="windows-x86_64-nsis" ;;
        *.msi) payload_keys="windows-x86_64-msi" ;;
        *) fail "unrecognised payload: $name" ;;
    esac
    for key in $payload_keys; do
        [ -z "$os" ] || [ "$os" = "${key%%-*}" ] || fail "payloads span $os and ${key%%-*}"
        os=${key%%-*}
        case " ${keys[*]-} " in
            *" $key "*) fail "more than one payload for $key" ;;
        esac
        keys+=("$key")
        payloads+=("$payload")
    done
done

echo "==> signing"
for payload in "$@"; do
    abs="$(cd "$(dirname "$payload")" && pwd)/$(basename "$payload")"
    # npx resolves the tauri CLI from the repo's node_modules.
    (cd "$root" && npx --no-install tauri signer sign "$abs")
    [ -s "$payload.sig" ] || fail "signer produced no signature for $payload"
done

manifest=$(jq -n \
    --arg version "$version" \
    --arg pub_date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg minimum "$min" \
    '{version: $version, notes: "Cortex \($version)", pub_date: $pub_date, platforms: {}}
     + (if $minimum == "" then {} else {minimum_supported_version: $minimum} end)')

for i in "${!keys[@]}"; do
    payload=${payloads[$i]}
    manifest=$(jq \
        --arg key "${keys[$i]}" \
        --arg signature "$(< "$payload.sig")" \
        --arg url "$base/$(basename "$payload")" \
        '.platforms[$key] = {signature: $signature, url: $url}' <<< "$manifest")
done

echo "$manifest" > latest.json
echo "==> latest.json ($os)"
cat latest.json
