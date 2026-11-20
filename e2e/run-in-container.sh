#!/usr/bin/env bash
#
# The E2E run itself, inside a Linux environment with the toolchain already present:
# bundle the stub, build the app, drive it with WebdriverIO under Xvfb.
#
# Two callers, and this is the only description of the steps: run.sh supplies the
# container locally, and the CI job is already in one. Both must reach here rather than
# restating the steps, or the two drift.
#
# Expects CLIENTS_DIR to point at a clients checkout (run.sh mounts it; CI clones it).

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

echo "=== installing host tooling (esbuild)"
npm ci --no-audit --no-fund

echo "=== bundling the E2E testing stub"
./e2e/build-stub.sh

echo "=== bundling the webview scripts"
node scripts/build-webview.mjs

echo "=== building the app for linux"
# The suite drives the hosted web app's bridges, which only an `e2e` build contains.
(cd src-tauri && cargo build --features e2e)

echo "=== seeding app fixtures"
./e2e/seed-fixtures.sh

echo "=== installing the webdriver suite"
cd "$repo_root/e2e"
npm ci --no-audit --no-fund

echo "=== running specs"
# WebKitGTK needs a display; the app needs a session bus for tray/appindicator. The
# outer timeout is a backstop: a wedged webview or driver would otherwise hold the
# run open indefinitely, which reads like a slow suite rather than a failure.
exec timeout --foreground "${CORTEX_E2E_TIMEOUT:-600}" xvfb-run -a dbus-run-session -- npm test
