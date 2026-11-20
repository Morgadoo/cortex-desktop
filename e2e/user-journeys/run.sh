#!/usr/bin/env bash
# Drives the real app like a user, under a virtual display, against a real model:
#
#   checklist   the manual checklist of docs/codebase-improvements-plan.md section 8, in the light
#               and dark themes (chat, stop, regenerate, edit, composer menus, modes, queue,
#               sources, settings, pages, search, language)
#   multitask   switching chats while answers run, drafts, unread, two answers at once, and
#               coming back after the app is killed mid-answer
#   adapt       what the user says about themselves in one chat is remembered, shown to them, and
#               used in the next chat without being said again
#
#   CORTEX_TEST_PROVIDERS=path/to/providers.json DEEPSEEK_API_KEY=… e2e/user-journeys/run.sh [checklist|multitask|adapt|all]
#
# CORTEX_TEST_PROVIDERS is a chat/providers.json naming the model to use (its defaultModel); the
# key goes in the provider's usual environment variable. Each run starts from an empty profile in
# a temporary folder; screenshots and results go to $OUT (default: e2e/user-journeys/out).
#
# Needs: xvfb-run, dbus-launch, WebKitWebDriver (webkit2gtk-driver), tauri-driver
# (cargo install tauri-driver), python3 with selenium, and a debug build
# (npm run build:webview && (cd src-tauri && cargo build)).
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
which="${1:-all}"
: "${CORTEX_TEST_PROVIDERS:?set CORTEX_TEST_PROVIDERS to a providers.json naming the test model}"
export CORTEX_APP="${CORTEX_APP:-$root/src-tauri/target/debug/cortex-desktop}"
OUT="${OUT:-$here/out}"
mkdir -p "$OUT"

if [ -z "${INSIDE_XVFB:-}" ]; then
    INSIDE_XVFB=1 exec xvfb-run -a -s "-screen 0 1440x900x24" "$0" "$@"
fi
eval "$(dbus-launch --sh-syntax)"

profile() {
    export HOME
    HOME=$(mktemp -d)
    export XDG_CONFIG_HOME="$HOME/.config"
    local dir="$XDG_CONFIG_HOME/io.github.morgadoo.cortex-desktop"
    mkdir -p "$dir/chat"
    echo '{"updateChecks":false}' > "$dir/diagnostics.json"
    cp "$CORTEX_TEST_PROVIDERS" "$dir/chat/providers.json"
}

export WEBKIT_DISABLE_COMPOSITING_MODE=1 LIBGL_ALWAYS_SOFTWARE=1
driver=""
start_driver() {
    tauri-driver --native-driver "$(command -v WebKitWebDriver)" > "$OUT/driver.log" 2>&1 &
    driver=$!
    sleep 2
}
stop_driver() {
    [ -n "$driver" ] && kill "$driver" 2>/dev/null || true
    pkill -f "$CORTEX_APP" 2>/dev/null || true
    sleep 2
}
trap stop_driver EXIT

status=0
if [ "$which" = checklist ] || [ "$which" = all ]; then
    for theme in light dark; do
        profile
        start_driver
        python3 "$here/checklist.py" "$OUT" "$theme" | tee -a "$OUT/summary.txt" || status=1
        stop_driver
    done
fi
if [ "$which" = multitask ] || [ "$which" = all ]; then
    profile
    start_driver
    python3 "$here/multitask.py" "$OUT" run | tee -a "$OUT/summary.txt" || status=1
    sleep 3
    python3 "$here/multitask.py" "$OUT" after-restart | tee -a "$OUT/summary.txt" || status=1
    stop_driver
fi
if [ "$which" = adapt ] || [ "$which" = all ]; then
    profile
    start_driver
    python3 "$here/adapt.py" "$OUT" | tee -a "$OUT/summary.txt" || status=1
    stop_driver
fi
if grep -q '^FAIL' "$OUT/summary.txt"; then status=1; fi
exit $status
