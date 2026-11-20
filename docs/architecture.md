# How Cortex Desktop is built

A map of the code for anyone changing it: what each part is for, which way dependencies may
point, where data lives, and the steps for the common changes. For what the app does, see the
[README](../README.md); for why the code is shaped this way, see the
[review](codebase-improvements.md) and its [plan](codebase-improvements-plan.md).

## What the shape is for

Cortex is a personal AI system that runs on the user's own computer and grows with them. Most of
what it learns is data it keeps: memories, knowledge pages, skills it saves, agents and their
notebooks, tools it writes and has approved, scheduled tasks, and traces of every run that evals
and the lifecycle job read to decide what to keep. The architecture follows from that:

- **The user owns the data.** Everything lives in one folder on their computer, in documented
  JSON files, optionally encrypted; keys live in the OS keychain. Nothing needs a server.
- **Growth is data, not code.** A new skill, agent, workflow, tool or connector is a file the app
  reads, not a build. The code is the engine that runs them, kept small and predictable.
- **Every model and tool goes through one loop and one gate.** Approvals, privacy filtering,
  hooks, traces and budgets apply whether a turn comes from the window, the terminal, a
  scheduled task, a chat platform or the local API.
- **The core does not need a window.** The agent loop and its decisions are in crates with no
  Tauri dependency, so other front ends and tests reuse them.

## The pieces

```text
webview/                  the interface: TypeScript, no framework, bundled by esbuild
   ↕  Tauri IPC: commands (src-tauri/src/handlers.rs) and events (chat://event, …)
src-tauri/src/            the app crate: window, IPC, and the chat engine
src-tauri/crates/         the AI core that needs no window
   cortex-kernel          the agent loop, approval policy, context, prompt, privacy, evals
   cortex-retrieval       tokenizer and BM25
   cortex-traces          run traces, feedback and the artifact registry (SQLite)
   cortex-sandbox         the WebAssembly sandbox for tools the assistant writes
   cortex-store           conversations, their index, backups and trash, version history,
                          partial answers, settings files, encryption at rest, storage errors
   cortex-tools           the tool contract: connector trait and registry, approval and
                          exposure rules, credentials, connector state, tool results
```

### The chat engine: `src-tauri/src/chat/`

Folders sit in layers. A module may use its own layer and the layers below it, never one above;
`scripts/check-layers.py` enforces this in CI.

| Layer | Folder | What it holds |
|---|---|---|
| 0 | `base/` | What every layer shares: the events a turn streams to the window, the built-in tools' names, asking the user to approve a call |
| 0 | `storage/` | Conversations and their index, backups, versions, sync, encryption at rest, the vault and keychain, traces, uploads, imports |
| 1 | `models/` | Providers and their catalogues, connecting to a model, routing, the built-in runtime, coding-agent back ends, the privacy filter, the arena |
| 1 | `grounding/` | Memories, knowledge pages, libraries, embeddings and extraction, retrieval, web and chat search, reading pages, calendars |
| 1 | `toolkit/` | The code sandbox and workbench, written and custom tools, OpenAPI servers, computer use, coding sessions, canvases, spreadsheets, outputs |
| 1 | `multimedia/` | Images, speech, video, animation, recording |
| 1 | `ui/` | The interface's translations and the emoji pack |
| 2 | `turn/` | One turn: the engine and its tool handlers, the `Host` seam, tool dispatch and built-ins, sub-agents, hooks, context and prompt |
| 2 | `automation/` | Agents, scheduled tasks, workflows, skills and plugins, learning, history (restoring versions), the lifecycle job, evals |
| 3 | `commands/` | The IPC commands the window calls: thin wrappers over the layers below |
| 3 | `cli/`, `integrations/` | The terminal modes; Slack, Discord, Telegram, the local API, share links |

Every module is also re-exported by name from `chat.rs`, so code writes `crate::chat::store`,
not the folder path; moving a module between folders changes no caller.

`turn/host.rs` is the seam between a turn and whatever runs it: the window (`TauriHost`), a
terminal or eval (`turn/headless_host.rs`), or a test (`host::testing::TestHost`).

### The rest of the app crate: `src-tauri/src/`

| Module | Role |
|---|---|
| `app.rs`, `handlers.rs` | Startup and the command list (`handlers.rs` is generated) |
| `support/` | Services below everything: settings files, HTTP clients, slugs |
| `tools/` | The connector registry, connectors (files, shell, browser, MCP, mail, Luminance), approvals, keychain |
| `settings/`, `account.rs` | Device and account settings; the active account |
| `main_window.rs`, `webview.rs`, `quick_ask/`, `dictation.rs`, `inapp_browser.rs`, `tray.rs`, `dialog/` | Windows and their behaviour |
| `updater.rs`, `reporting.rs`, `rename_migration.rs`, `reset.rs` | Updates, diagnostics, the Lumo-to-Cortex migration, reset |
| `schedules/` | The scheduling timing engine; with the `hosted` feature also the hosted web app's run queue |
| `browser_login.rs`, `skills.rs`, `pdf_export.rs` | The hosted web app's sign-in, skill cache and PDF export (`skills.rs` and `pdf_export.rs` only with `hosted`) |

### The interface: `webview/`

| Folder | Holds |
|---|---|
| `pages/app/core/` | Backend calls (`api.ts`, `call.ts`), state and the event reducer, preferences (`prefs.ts`), shortcuts, i18n |
| `pages/app/ui/` | The element builder `h()`, icons, Markdown rendering (the only place HTML from a model is inserted), dialogs, components |
| `pages/app/shell/` | The entry point, sidebar, search dialog, projects |
| `pages/app/chat/` | The chat view, composer, transcript, sending, queue, drafts, side panels |
| `pages/app/canvas/`, `pages/`, `settings/`, `voice/`, `theme/` | Work products; the full pages; the settings dialog by section; voice; appearance |
| `bridges/`, `pages/settings/`, `types/` | The hosted web app's bridges and old settings page (built into the app only with `hosted`) |
| `shared/` | What both kinds of page use: the Tauri entry point, the connector catalog (`connector-catalog.json`), the logger |
| `static/` | HTML, CSS and images, copied as they are; every colour is a token in `static/styles/tokens.css` |

## Two builds

The default build is the native Cortex app. The `hosted` Cargo feature builds the original Lumo
Desktop, whose window loads the hosted Lumo web app and talks to Rust through `webview/bridges/`;
the native build contains none of that code. The `e2e` feature (which implies `hosted`) is for
the WebDriver suite in `e2e/`, and is the only build that honours `CORTEX_E2E_TESTING`.

## Where data lives

In the app's config folder (see the README for each platform). Files under `chat/` are sealed
when the user turns on encryption at rest (`storage/at_rest.rs`), except the folders other
programs read (`chat/sandbox`, `chat/agent-runs`, `skills`).

| Path | Owner |
|---|---|
| `settings.json`, `accounts/<id>/settings.json` | `settings.rs` (plain: read before decryption) |
| `diagnostics.json` | `reporting.rs` (plain: read before the app starts) |
| `accounts/<id>/mcp_servers.json`, `connector_state.json`, `mcp_tools/` | `tools/` |
| `chat/conversations/`, `index/`, `backups/`, `trash/`, `projects.json`, `instructions.json` | `storage/store.rs`, `index.rs`, `backup.rs` |
| `chat/versions/` | `storage/versions.rs`; restoring is `automation/history.rs` |
| `chat/memories.json`, `knowledge.json`, `libraries/`, `embeddings.json`, `search.json`, `extraction.json`, `calendars.json` | `grounding/` |
| `chat/agents.json`, `tasks.json`, `workflows.json`, `skills.json`, `learning.json`, `lifecycle.json`, `agent-market.json` | `automation/` |
| `chat/canvases/`, `sandbox/`, `sandbox.json`, `script-tools/`, `tools/`, `openapi/`, `workbench*`, `computer.json` | `toolkit/` |
| `chat/audio.json` | `multimedia/audio.rs` |
| `chat/privacy.json`, `arena.json`, `acp-agents.json` | `models/` |
| `chat/hooks.json`, `functions.json`, `tool-outputs/` | `turn/` |
| `chat/slack.json`, `discord.json`, `telegram.json`, `api-server.json` | `integrations/` |
| `chat/audit/`, `recent-files.json` | `storage/` |
| OS keychain, service `io.github.morgadoo.cortex-desktop` | `tools/auth.rs`; the local profile's store is `storage/secrets.rs` |

A settings file is declared once, with its path and whether it is sealed:

```rust
const FILE: SettingsFile<Settings> = SettingsFile::sealed("chat/example.json");
```

`support/settings_file.rs` loads it (the default when absent; logged when unreadable) and saves
it through a temporary file. Moving a file between sealed and plain is a data migration.

## Generated and committed files

| File | Made by | Checked in CI |
|---|---|---|
| `src-tauri/src/handlers.rs`, `src-tauri/permissions/commands.toml` | `scripts/generate-commands.py` | `--check` |
| `webview/pages/app/core/generated/ipc.ts` | `scripts/generate-types.sh` (ts-rs, from Rust types marked `typescript`) | `--check` |
| `webview/pages/app/core/generated/commands.ts` | `scripts/generate-commands.py` (every command name, as a TypeScript union) | `--check` |
| `src-tauri/i18n/catalog.json` | `scripts/ui-strings.mjs` (`npm run i18n:catalog`) | `npm run check:i18n` |
| `src-tauri/i18n/{ar,de,es,fr,pt}.json` | `scripts/translate_ui.py` | a Rust test: every catalog string translated |
| `src-tauri/gen/schemas/` | `tauri-build`, on every build | no |
| `src-tauri/crates/cortex-sandbox/wasm/js-runtime.wasm` | `scripts/build-js-runtime.sh` | no |
| `src-tauri/src/tools/connectors/luminance/operations.json` | `scripts/generate-luminance-operations.mjs` | no |
| `src-tauri/icons/` | `npm run icons:desktop` | no |
| `scripts/check-layers-allow.txt` | `scripts/check-layers.py --write` (empty now; keep it so) | `check-layers.py` |

## Common changes

### A command the window calls

1. Write the function with `#[tauri::command]` in the `commands/` file for its area (or the
   module it belongs to, outside chat). Keep it thin: parse, call one function below, convert
   the result.
2. Run `python3 scripts/generate-commands.py`. It registers the command and adds it to its
   file's permission.
3. A command in a new file: grant the file's new permission (`allow-<path>`) in
   `capabilities/chat.json` (the main window) or `default.json`. A command only a separate window
   may call: mark it `// IPC: own permission` and add it to `permissions/windows.toml`.
4. Call it from the interface through `core/api.ts` or `core/call.ts`.

`tests/ipc_surface.rs` fails if a command is registered but not permitted, or the reverse.

Command names start with a verb: `list` for a collection, `get` for one item, `save` for writing a
whole value, `set` for one field (or the fields given), `add` to put something into a collection,
`delete` for removal.

### A type the window receives

Derive its TypeScript declaration from the Rust type instead of writing it again in `api.ts`:

```rust
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
```

then run `bash scripts/generate-types.sh` and import it from `core/generated/ipc.ts`. ts-rs
follows the type's `serde` attributes, so the declaration is what the window really gets. The
provider, model, project, instructions, chat settings, memory, speed and schedule types are
converted, and so are the conversation (its turns, parts, versions and usage), the sidebar rows,
search hits, the trash, storage health, the context breakdown, an agent's spend, skills, workflows
and the events a turn streams; the others in `api.ts` are still written by hand and move over
one area at a time. Agents and scheduled tasks are generated too.

- A 64-bit integer field needs `#[cfg_attr(feature = "typescript", ts(type = "number"))]`: it
  travels as a JSON number, not a `bigint`.
- A field serde leaves out (`skip_serializing_if = "Option::is_none"`) needs `ts(optional)` to be
  declared `field?: T`. One left out when empty or false (`"Vec::is_empty"`, `"String::is_empty"`,
  `"std::ops::Not::not"`) needs `ts(as = "Option<T>", optional)`.
- A `&'static str` that holds one of a few words is better declared as their union:
  `ts(type = "'user' | 'assistant'")`.
- `cortex-kernel` has its own `typescript` feature, turned on by the app's; types from other
  crates need the same before the app's types can name them.

### A built-in tool

Its name goes in `chat/base/tool_names.rs` and `NAMES`, its schema in `turn/builtins.rs`'s
`definitions`, and its handling in `builtins::run` (or the engine's handler table in
`turn/engine/handlers.rs` when it needs the turn's state). Add a turn test in
`turn/engine/turn_tests.rs`.

### A connector

A type implementing `cortex_tools::registry::Connector` (re-exported as `tools::registry`),
registered in `tools::swap_account` (`tools.rs`). `crates/cortex-tools/src/echo.rs` is the
smallest complete one, a model to start from. Its exposure class (`cortex-tools`'s `exposure`),
icon (`tools/icons.rs`) and log redaction (`tools/commands.rs`) are tables outside the
connector, on purpose: one table is easier to audit than a flag on each connector.

### A setting

A field on the module's settings struct (with `#[serde(default)]`, so older files still load),
and, if the window changes it, a command as above. A per-device interface convenience instead
goes in `core/prefs.ts`.

### A style

Rules for the app window go under `webview/styles/app/`, in the folder of the interface area
they style (`chat/`, `settings/`, `pages/`, ...); a new file is added to `order.txt` there, which
is the order the build joins them into `dist/styles/app.css` (a file it leaves out fails the
build). The other pages' stylesheets are under `static/styles/`. Colours, shadows and gradients go in
`static/styles/tokens.css` as named tokens (a value for each theme where they differ), and the rule uses
`var(--name)`. Rule order decides which rule wins, so moving or merging rules is checked with
screenshots of every page in both themes:

```sh
npm run build:webview && node scripts/screenshot-pages.mjs shots/before
# change the stylesheets
npm run build:webview && node scripts/screenshot-pages.mjs shots/after
node scripts/screenshot-pages.mjs --compare shots/before shots/after
```

### An error

Storage returns `StoreError` (`cortex-store`): which file, and whether it was missing, locked,
unreadable or unwritable. The layers above return `chat::error::ChatError`, which wraps it and adds
network, model and invalid-request failures; `?` converts one into the other. Words meant for the
user go in `Invalid`; a library's error with no kind of its own is `Internal`. Only the commands
(and the CLI and chat-app integrations) turn an error into a `String`, the text the window shows,
as do the few boundaries outside `chat/` that take text: the kernel's step loop, the tool
registry's connector call and the sandbox's nested calls.

### State that lasts while the app runs

A cache, the commands a module started, or the server it runs, is a type of the module's own,
made with `Default` and kept in `ChatState`: `host.chat_state().part::<Terminals>()` in a turn,
`app.state::<ChatState>().part::<Terminals>()` in a command. It belongs to the app (or to the
test's host, so tests do not share it) and nothing outside the module can reach it. A
process-wide `static` is for what really is one per process (the loaded model, the keychain,
the encryption key every file read uses); `scripts/check-statics.py` lists them, each with a
`Process-wide:` comment saying why.

### A module

Put it in the folder for its role, add it to that folder's `mod.rs` and to the folder's
`pub use` line in `chat.rs`, and import only from its own layer or below.

## Checks

| Command | What it checks |
|---|---|
| `npm run typecheck`, `npm run lint`, `npm test` | The interface: types, Biome, the HTML-sink rule and no import cycles, unit tests |
| `npm run check:i18n` | The translation catalog is current |
| `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test` (in `src-tauri`, `--workspace --features cortex/test-utils`) | The Rust code, native build |
| the same with `--features cortex/test-utils,cortex/e2e` | The Rust code with the hosted web app's |
| `python3 scripts/check-layers.py` | Dependencies point down the chat engine's layers |
| `python3 scripts/check-statics.py` | At most nine process-wide mutable statics, each saying why |
| `python3 scripts/measure-plan.py --check` | The measures of the improvements plan (file and function sizes, layering, errors, statics) meet their targets |
| `python3 scripts/generate-commands.py --check` | The command list and permissions match the code |
| `python3 scripts/check-kernel-plan.py` | Every test the kernel plan cites exists and passes |
| `./e2e/run.sh` | The bridge contract, in a container (needs Proton's web clients checkout) |
