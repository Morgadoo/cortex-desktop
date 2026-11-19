#!/usr/bin/env bash
# Uninstall Cortex Desktop from macOS.
# Removes the app bundle, user data, and optionally Keychain credentials.

set -euo pipefail

BUNDLE_ID="io.github.morgadoo.cortex-desktop"
APP_NAME="Cortex"
APP_PATH="/Applications/${APP_NAME}.app"
LAUNCH_AGENT_LABEL="${APP_NAME}"
LAUNCH_AGENT_PLIST="${HOME}/Library/LaunchAgents/${LAUNCH_AGENT_LABEL}.plist"

DRY_RUN=0
SKIP_CONFIRM=0

usage() {
	cat <<EOF
Usage: $(basename "$0") [options]

Remove Cortex Desktop and its app-owned data from this Mac.

Options:
  --app-path <path>   App bundle to remove (default: ${APP_PATH})
  --dry-run           Print actions without making changes
  --yes               Skip the main confirmation prompt (Keychain still prompts)
  -h, --help          Show this help

User-chosen filesystem folders from settings are not removed.
EOF
}

log() {
	printf '%s\n' "$*"
}

run() {
	if [[ "${DRY_RUN}" -eq 1 ]]; then
		printf '[dry-run] %s\n' "$*"
	else
		"$@"
	fi
}

confirm() {
	local prompt="$1"
	if [[ "${SKIP_CONFIRM}" -eq 1 ]]; then
		return 0
	fi
	printf '%s [y/N] ' "${prompt}"
	read -r reply
	case "${reply}" in
		[yY] | [yY][eE][sS]) return 0 ;;
		*) return 1 ;;
	esac
}

remove_path() {
	local path="$1"
	if [[ -e "${path}" ]]; then
		log "Removing ${path}"
		run rm -rf "${path}"
	else
		log "Not found (skipped): ${path}"
	fi
}

quit_app() {
	if pgrep -x "${APP_NAME}" >/dev/null 2>&1; then
		log "Quitting ${APP_NAME}..."
		run pkill -x "${APP_NAME}" || true
		sleep 1
	fi
}

disable_launch_agent() {
	if [[ -f "${LAUNCH_AGENT_PLIST}" ]]; then
		log "Disabling launch agent ${LAUNCH_AGENT_LABEL}..."
		if [[ "${DRY_RUN}" -eq 1 ]]; then
			log "[dry-run] launchctl bootout gui/$(id -u)/${LAUNCH_AGENT_LABEL}"
		else
			launchctl bootout "gui/$(id -u)/${LAUNCH_AGENT_LABEL}" 2>/dev/null || true
		fi
		remove_path "${LAUNCH_AGENT_PLIST}"
	fi
}

remove_keychain_entries() {
	if ! confirm "Remove stored credentials from Keychain (service: ${BUNDLE_ID})?"; then
		log "Skipping Keychain cleanup."
		return 0
	fi

	if ! command -v security >/dev/null 2>&1; then
		log "security(1) not found; skipping Keychain cleanup."
		return 0
	fi

	local deleted=0

	if [[ "${DRY_RUN}" -eq 1 ]]; then
		if security find-generic-password -s "${BUNDLE_ID}" >/dev/null 2>&1; then
			log "[dry-run] security delete-generic-password -s ${BUNDLE_ID} (all matching entries)"
			deleted=1
		fi
		if [[ "${deleted}" -eq 0 ]]; then
			log "No Keychain entries found for ${BUNDLE_ID}."
		else
			log "Keychain entries would be removed."
		fi
		return 0
	fi

	while security find-generic-password -s "${BUNDLE_ID}" >/dev/null 2>&1; do
		local account
		account="$(
			security find-generic-password -s "${BUNDLE_ID}" 2>&1 |
				awk -F'"' '/"acct"/ { print $4; exit }'
		)"

		if [[ -n "${account}" ]]; then
			log "Removing Keychain entry: ${account}"
			security delete-generic-password -s "${BUNDLE_ID}" -a "${account}" >/dev/null 2>&1 || break
		else
			log "Removing Keychain entry for ${BUNDLE_ID}"
			security delete-generic-password -s "${BUNDLE_ID}" >/dev/null 2>&1 || break
		fi

		deleted=$((deleted + 1))
	done

	if [[ "${deleted}" -eq 0 ]]; then
		log "No Keychain entries found for ${BUNDLE_ID}."
	else
		log "Removed ${deleted} Keychain entr$( [[ "${deleted}" -eq 1 ]] && echo y || echo ies )."
	fi
}

while [[ $# -gt 0 ]]; do
	case "$1" in
		--app-path)
			APP_PATH="$2"
			shift 2
			;;
		--dry-run)
			DRY_RUN=1
			shift
			;;
		--yes)
			SKIP_CONFIRM=1
			shift
			;;
		-h | --help)
			usage
			exit 0
			;;
		*)
			log "Unknown option: $1"
			usage
			exit 1
			;;
	esac
done

if [[ "$(uname -s)" != "Darwin" ]]; then
	log "This script only supports macOS."
	exit 1
fi

log "Cortex Desktop uninstall"
log "  App:        ${APP_PATH}"
log "  Bundle ID:  ${BUNDLE_ID}"
log "  Data:       ${HOME}/Library/Application Support/${BUNDLE_ID}"
log ""

if ! confirm "Remove Cortex and all app-owned data listed above?"; then
	log "Cancelled."
	exit 0
fi

quit_app
disable_launch_agent
remove_path "${APP_PATH}"
remove_path "${HOME}/Library/Application Support/${BUNDLE_ID}"
remove_path "${HOME}/Library/Caches/${BUNDLE_ID}"
remove_path "${HOME}/Library/Logs/${BUNDLE_ID}"
remove_path "${HOME}/Library/WebKit/${BUNDLE_ID}"
remove_path "${HOME}/Library/Saved Application State/${BUNDLE_ID}.savedState"
remove_path "${HOME}/Library/Preferences/${BUNDLE_ID}.plist"

remove_keychain_entries

log ""
log "Cortex has been uninstalled."
