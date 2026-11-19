#!/usr/bin/env bash
#
# Seed the app's real settings and connector state before the suite runs, so the
# filesystem connector comes up pointed at a scratch directory with fs_write gated on
# approval. Goes through the app's normal config path (XDG_CONFIG_HOME) rather than
# test hooks in the connector, which also exercises the settings -> connector wiring.
#
# Env:
#   XDG_CONFIG_HOME   where the app reads its config from (required)
#   CORTEX_E2E_SCRATCH  disposable directory for tool writes and MCP execution journal

set -euo pipefail

scratch=${CORTEX_E2E_SCRATCH:?CORTEX_E2E_SCRATCH must be set}
config_home=${XDG_CONFIG_HOME:?XDG_CONFIG_HOME must be set}
config_dir=$config_home/io.github.morgadoo.cortex-desktop
account_dir=$config_dir/accounts/e2e-test-account
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
node_binary=$(command -v node)

# Recreated per run so a leftover file cannot make an assertion pass.
rm -rf "$scratch"
mkdir -p "$scratch" "$config_dir" "$account_dir"

cat >"$config_dir/settings.json" <<JSON
{
  "search_shortcut": "Ctrl+L",
  "launch_at_startup": false,
  "settings_hotkey_enabled": false
}
JSON

cat >"$account_dir/settings.json" <<JSON
{
  "filesystem_allowed_roots": ["$scratch", "$config_home"],
  "filesystem_write_enabled": true,
  "bash_enabled": true
}
JSON

# Inside an allowed parent, but protected by the filesystem connector itself.
echo 'protected-e2e-sentinel' >"$config_dir/e2e-protected.txt"

cat >"$account_dir/mcp_servers.json" <<JSON
{
  "servers": [
    {
      "id": "security", "display_name": "Security fixture",
      "transport": "stdio", "command": "$node_binary",
      "args": ["$repo_root/e2e/fixtures/mcp-server.cjs"],
      "env": { "CORTEX_E2E_MCP_JOURNAL": "$scratch/mcp-journal.jsonl" }
    },
    {
      "id": "legacy-cache", "display_name": "Offline legacy cache fixture",
      "transport": "stdio", "command": "/usr/bin/false", "args": []
    }
  ]
}
JSON

# The offline peer cannot overwrite this old cache during startup warm-up.
mkdir -p "$account_dir/mcp_tools"
cat >"$account_dir/mcp_tools/legacy-cache.json" <<JSON
[
  { "name": "read", "description": "Old read tool", "input_schema": {"type":"object"}, "read_only": true, "policy": "auto_approve" },
  { "name": "write", "description": "Old write tool", "input_schema": {"type":"object"}, "read_only": false, "policy": "auto_approve" },
  { "name": "unannotated", "description": "Old unannotated tool", "input_schema": {"type":"object"}, "policy": "auto_approve" }
]
JSON

# Preserve the explicit overrides exercised by the existing approval specs. MCP tools
# start without overrides so the policy spec can exercise discovered defaults.
# Its user-override case runs afterward and leaves an explicit `ask` on cleanup.
cat >"$account_dir/connector_state.json" <<JSON
{
  "disabled_connectors": ["mcp-security", "mcp-legacy-cache"],
  "tool_policies": {
    "echo": { "ask": "ask" },
    "filesystem": { "fs_write": "ask" }
  }
}
JSON

# Scheduling specs persist checkpoints and runs; start each suite with an empty ledger.
cat >"$account_dir/schedules.json" <<JSON
{ "version": 1, "schedules": {}, "runs": {} }
JSON

echo "seed-fixtures: scratch=$scratch config=$config_dir account=$account_dir"
