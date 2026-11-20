# Code base review: structure, reuse and maintainability

2026-11-20 · based on commit `73722bf` · line numbers are from that commit

This review looks at how the repository is organized, not at what the app does. It lists what
to keep, what to change, and in which order, judged against the usual rules for a well-structured
procedural code base. It complements [agent-kernel-plan.md](agent-kernel-plan.md), which
restructured the agent loop; the recommendations here cover the rest of the code and follow
that plan's direction (a core with no window, a thin shell around it).

How it was produced: every area was read (Rust chat engine, the rest of the Rust code, the
TypeScript interface, build and CI), and the counts come from `grep`, `awk` and small scripts.
Counts marked "about" are approximate; anything stated as a fact about a specific file was read
at the line given. The JavaScript checks were run (`typecheck`, `check:i18n`, `check:voice`, and
the three test files under `scripts/`); the Rust build and tests were not.

## 1. Summary

The code is in better shape line by line than its structure suggests. Functions are mostly short,
every Rust file opens with a description, there are no TODOs, almost no panics in production
paths, strict TypeScript with no `any`, and CI gates formatting, lints and about 880 tests.

The problems are at the level above the function: where things live, what may depend on what,
and how many times the same small thing is written. Six changes would do the most:

1. **Give the two big flat folders an internal structure.** `src-tauri/src/chat/` has 86 sibling
   modules and `webview/pages/app/` has 51 sibling files, with no grouping in either
   ([section 4](#4-structure-and-boundaries)).
2. **Make dependencies point one way.** About 69 of the 86 chat modules sit in one dependency
   cycle; storage code calls the engine, and the engine calls storage
   ([section 4.2](#42-make-dependencies-point-one-way)).
3. **Write each recurring thing once.** Settings files, HTTP clients, keychain lookups, error
   toasts and formatters are each written between 5 and 150 times
   ([section 6](#6-one-way-to-do-each-recurring-thing)).
4. **Describe each backend command once.** A command is declared by hand in four or five places,
   and 287 of the interface's backend calls bypass the typed wrapper file
   ([section 8](#8-the-boundary-between-interface-and-backend)).
5. **Separate the inherited hosted mode from the native app.** Roughly 10,000 lines that only the
   old Lumo-hosted build uses are compiled into, and partly started by, the native app
   ([section 4.5](#45-separate-the-hosted-mode)).
6. **Close the gaps in the checks.** Three test files are run by nothing and five of their nine
   tests fail; no linter or formatter covers the TypeScript; CI never compiles the Windows or
   macOS code ([section 10](#10-tests-and-tooling)).

[Section 12](#12-suggested-order) puts all recommendations in an order that starts with changes
taking an hour each.

## 2. The code base in numbers

| Area | Files | Lines | Notes |
|---|---|---|---|
| `src-tauri/src/chat/` (+ `chat.rs`) | 102 | 60,300 | The native app's engine; about 16,000 lines are tests |
| `src-tauri/src/tools/` (+ `tools.rs`) | 47 | 16,900 | Connector registry, approvals, keychain |
| `src-tauri/src/schedules/` (+ `schedules.rs`) | 7 | 3,400 | The hosted app's scheduler |
| Other `src-tauri/src/*.rs` and folders | 40 | 8,400 | Windows, tray, updater, settings, dialogs |
| `src-tauri/crates/` (4 crates) | 14 | 5,300 | Kernel, retrieval, sandbox, traces; no Tauri |
| `webview/pages/app/` | 51 | 19,900 | The native interface; `chat.ts` alone is 4,892 lines |
| `webview/` elsewhere | 39 | 9,400 | Hosted-mode bridges and pages, shared code, CSS |
| `scripts/`, `e2e/`, `extension/` | 52 | 3,500 | Build, release, checks; bridge end-to-end tests; browser extension |

344 Tauri commands (289 of them `chat_*`), 5 interface languages, 3 platforms.

## 3. What to keep

These are deliberate strengths; the recommendations below should not erode them.

- **Explicit data flow in the engine.** Domain functions take `config_dir: &Path` (about 446
  parameters) rather than reaching for the app handle, which is confined to 26 of the 102 chat
  files. `store.rs` and `at_rest.rs` never touch the window. This is why 58 files can test
  against a temporary folder.
- **The `Host` seam and the kernel crates.** `chat/host.rs:26-52` lets a whole turn run in a
  test, a terminal or a worker; the four crates compile without Tauri.
- **The connector abstraction.** `tools/registry/connector.rs:148-240` is a small trait with
  optional facets, an injected approval gate and host, and one registration site
  (`tools.rs:44-103`). Keychain access goes through one `SecretStore`.
- **Typed events with an exhaustive reducer.** `ChatEvent` is a 16-variant union in
  `webview/pages/app/api.ts:271-287`, reduced by one switch in `state.ts:151-248`.
- **Safe rendering by construction.** `h()` inserts strings as text (`dom.ts:13`), and all model
  Markdown passes through one sanitised path (`dom.ts:260-269`). There are 3 `innerHTML`
  assignments in all of the TypeScript.
- **Honest checks.** `cargo fmt`, `clippy -D warnings`, tests, live tests against real Qdrant and
  Tika, and a script that verifies every test the kernel plan cites exists and passes. All 344
  commands have a matching permission, with none left over.
- **Comments that explain why**, a build script that fails fast, and a `dist/` folder that cannot
  drift from its sources.

## 4. Structure and boundaries

Principle: a module has one job, lives next to the modules it works with, and its place in the
tree tells a newcomer what it is.

### 4.1 Group the chat modules

`chat.rs:13-98` declares 86 modules in alphabetical order; 81 are `pub`, and the header comment
describes four of them. Feature modules for Slack, video rendering, spreadsheet parsing and the
turn loop are siblings. Proposed grouping, as sub-folders of `chat/`:

| Folder | Modules | Lines |
|---|---|---|
| `turn/` | engine, engine/handlers, host, tools, builtins, deferred, subagents, hooks, functions, privacy, context, prompt, traces | 10,300 |
| `models/` | providers, router, local_models, batched_qwen2, agent_cli, acp, arena | 4,200 |
| `storage/` | store, index, backup, at_rest, versions, sync, vault, audit, uploads, recent_files, outputs, import, coding_import | 5,800 |
| `knowledge/` | memory, knowledge, libraries, embeddings, extraction, retrieval, search, search_all, reader, preview, favicon, learning, calendar | 6,600 |
| `toolkit/` | sandbox, workbench, script_tools, custom_tools, openapi, computer, webpage, terminal, code, canvas, sheets, proposals | 10,000 |
| `media/` | media, audio, video, videogen, animation, hex, recorder | 3,500 |
| `agents/` | agents, org, agent_market, tasks, workflows, skills, skill_catalog, plugins, lifecycle, evals | 8,600 |
| `integrations/` | slack, discord, telegram, api_server, widget.js, share, links | 3,600 |
| `cli/` | headless, terminal_chat, config_cli | 1,600 |
| `ui/` | ui_language, emoji_pack | 600 |
| `commands/` | as today | 5,100 |

How to do it safely: move one folder per pull request, and keep a `pub use` line in `chat.rs` for
each moved module so no caller changes. `scripts/check-kernel-plan.py` finds tests by name, not
by path, so it keeps passing; the 28 lines in `agent-kernel-plan.md` and `review-coverage.md` that cite
source paths need updating.

Three smaller items in the same area:

- `chat.rs:113-319` holds real code (`ChatState` and its tests) among the module list. Move it to
  `chat/state.rs`.
- `context.rs`, `prompt.rs` and `retrieval.rs` are three-line re-exports of the kernel crates.
  Once callers import the crates directly they can go.
- `batched_qwen2.rs` is 337 lines of vendored model code with no tests. It belongs in a crate
  beside the other model runtime code, with a note of where it came from.

### 4.2 Make dependencies point one way

Principle: lower layers do not know about higher ones. If storage calls the engine, neither can
be understood, tested or moved without the other.

A graph built from `use super::x` and `crate::chat::x` lines (non-test code, about 400 edges)
puts about 69 of the 86 chat modules in one strongly connected group. The chains that create it,
each read in the code:

| Upward call | Where | Fix |
|---|---|---|
| Storage knows every domain | `store.rs:378` → `versions::keep`; `versions.rs:199-265` names memory, knowledge, agents, workflows, tasks, custom tools, script tools | Each domain registers its kind with `versions`; `versions` holds a table and names none of them |
| The scheduler calls the engine and the command layer | `tasks.rs:894` → `engine::run_task`; `tasks.rs:860` → `commands::registry_connectors` | The shell passes the runner in when it starts the scheduler |
| Integrations import the engine for a keychain lookup | `engine::api_key` (`engine.rs:501`) called from slack, discord, telegram, media, videogen | Move it to a small `secrets` module (see 6.1) |
| Shared types live beside dispatch logic | `ChatTool` and `Outcome` in `tools.rs` (used by 25 files), `ChatEvent` in `engine.rs` | A leaf `types` module; this alone removes most `tools` ↔ `engine` ↔ `host` pairs |
| Top-level modules reach into chat, and back | `dictation.rs:114,124,449`, `inapp_browser.rs:32,42`, `account.rs:77`, `tools/connectors/bash.rs:298` | File encryption (`at_rest`) is a general service: move it out of `chat`. Pass the vault's environment into the bash connector |
| `tools` ↔ `settings`, `account`, `feature_flags` | `settings/commands.rs:11`, `tools/connectors/filesystem.rs:2`, `account.rs:154`, `feature_flags.rs:99`, `tools/connectors/mcp.rs:1128` | Settings loading below both; account switching calls a hook that `tools` registers |

The target is a short list of layers that can be stated in one line and checked by a script:

```text
commands, cli, integrations     entry points; may use anything below
turn, agents                    orchestration
toolkit, knowledge, models, media
storage, secrets, types         leaves; use nothing above
```

A 30-line script over the `use` lines, run in CI, keeps a fixed layering from decaying; start it
with the current violations listed as allowed and shrink the list.

### 4.3 Give visibility a meaning

Principle: a module shows the few things others need and hides the rest.

`chat` is a private module (`lib.rs:4`), yet inside it there are 1,073 `pub fn` against 19
`pub(crate)` and 9 `pub(super)`. About 60 of the 81 `pub mod` are never named outside `chat/`.
`pub` therefore says nothing about what is safe to change. After the grouping in 4.1, make each
folder's `mod.rs` re-export only what other folders use and mark the rest `pub(super)`. Rust's
`unreachable_pub` lint, enabled per folder as it is cleaned, stops it growing back.

The command files do the opposite of hiding: each begins `use super::*;` and
`commands.rs:15-38` re-exports all of them with globs. Named imports show what each file needs.

### 4.4 Group and split the interface

`webview/pages/app/` has 51 files in one folder. `chat.ts` and `settings.ts` are 39% of it.

| Folder | Files |
|---|---|
| `core/` | api (split by area, plus `events.ts`), state, a new `prefs.ts`, storage-migration, shortcuts, i18n |
| `ui/` | `h.ts`, `icons.ts`, `markdown.ts`, `overlay.ts` (all four from `dom.ts`), ui, emoji, emojiscan, fluent, favicon, vim, mdformat |
| `theme/` | appearance, orbgl, backdropgl, and the theme functions now in `settings.ts:95-119` |
| `voice/` | audio, dictation, vad, live, voice, wakeword, wakematch |
| `chat/` | the pieces of `chat.ts` below, chatimage, searchmode, uploads, variables, promptvars, improve, coderun, changes, terminal, devsite |
| `canvas/` | canvas, diagrams, sheet, share, versions, outputs |
| `pages/` | agents, agentpanel, tasks, libraries, workflows, apps, chatspage; skills and knowledge (from `context.ts`); connectors and instructions (from `pages.ts`) |
| `settings/` | one file per section |
| `shell/` | main, sidebar, search dialog, projects |

Specific points:

- **`dom.ts` has five jobs**: the element builder, an 80-icon set (`:58-140`), Markdown and maths
  (`:164-359`), dialogs and toasts (`:374-474`), and file saving. It also calls the backend
  (`:239` at import time, `:343`, `:482`), which a rendering helper should not.
- **`context.ts` and `pages.ts` are named for nothing in particular.** They hold the Skills and
  Knowledge pages, and the Connectors page and Instructions dialog. `fluent.ts` is emoji, not
  translation.
- **One import cycle**: `canvas.ts:9` imports `diagrams`, `diagrams.ts:6` imports `canvas`, and
  `share.ts:9` imports `canvas` while `canvas.ts:233,282` loads `share` with a dynamic `import()`
  that, in a single bundle, only hides the cycle. Moving `appUrl` and `appFiles` to a small leaf
  file removes it.
- **`main.ts:24` imports the whole settings dialog to apply the theme**, and features import each
  other directly (`chat` → `settings`, `chat` → `pages`, `searchmode` → `chat`).

### 4.5 Separate the hosted mode

The repository builds two products: the native app, and the original shell around the hosted Lumo
web app (`LUMO_HOSTED=1`). The switch is one constant (`lib.rs:57`) tested at 11 places in 7
files, and a second switch, `CORTEX_E2E_TESTING`, is an environment variable read at run time in
production code (`lib.rs:59`, 14 call sites).

Because it is a run-time constant and not a compile-time feature, the native build still
compiles and partly runs the hosted code:

- `schedules/` starts at `app.rs:516`, so two schedulers run in one process (the other is
  `chat/tasks.rs`).
- `skills.rs`, `browser_login.rs` and most of `webview.rs` are in the binary.
- All nine `webview/bridges/` scripts are built and embedded, though injected only when hosted
  (`main_window.rs:52-65`).
- `webview/pages/settings/index.js` (2,715 lines of unchecked JavaScript, described in
  `scripts/build-webview.mjs:49-51` as "frozen as the reference for the React rewrite"),
  `settings.html` and `settings.css` are built and copied, though `settings/window.rs:70-77`
  never opens that page in a native build.
- The `echo` test connector (`tools.rs:85`) and in-memory secret stores (`tools/auth.rs:328-400`)
  ship in release builds; the `test-utils` feature that should gate them is empty
  (`Cargo.toml:105`).

Nothing marks these modules as hosted-only, and several share a name with their native
counterpart (see 9.1). The end-to-end suite and the three orphan test files test only this mode.

This needs a decision from the owner first: **keep hosted mode or remove it.**

- If kept: put it behind a Cargo feature `hosted`, move its Rust under `src/hosted/` and its
  TypeScript under `webview/hosted/`, and build the bridges only with that feature. The default
  build gets smaller and the two schedulers stop coexisting.
- If removed: about 10,000 lines go (schedules, hosted skills, browser sign-in, bridges, the old
  settings page and its styles, `webview/types/`, the end-to-end suite), along with the React
  dev dependency that nothing imports.

Either way, make `CORTEX_E2E_TESTING` a compile-time feature so test hooks cannot be switched on
in a release build.

## 5. Units that are too large

Principle: a function does one thing at one level of detail, and fits on a screen or two.

Most of the code already meets this: of about 1,600 functions outside chat, two exceed 150 lines.
The exceptions are few and concentrated, which makes them cheap to fix.

| Function | Where | Lines | Natural split |
|---|---|---|---|
| `app::run` | `app.rs:14-658` | 645 | CLI modes; plugin setup; the 344-entry handler list; a 190-line `setup` closure (state, deep links, shortcuts, eight background starters). One function per concern, with the ordering rules now in comments (`:507`, `:563`) expressed by call order |
| `traced_turn` | `chat/engine.rs:1241-1706` | 466 | Prepare the request; choose the model; run the loop; record the trace |
| `builtins::run` | `chat/builtins.rs:507-967` | 461 | One `match` over 21 tools: one function per tool, registered in the handler table that `engine/handlers.rs:16-50` already has |
| `builtins::definitions` | `chat/builtins.rs:75-466` | 392 | Each tool's schema next to its handler, or in a data file |
| `TurnSteps::call_model` | `chat/engine.rs:1736-2039` | 304 | Build the request; stream; fail over |
| `sandbox::run_in` | `chat/sandbox.rs:365-656` | 292 | Prepare the workspace; build the command; run; collect results |
| `McpConnector::ensure_connected` | `tools/connectors/mcp.rs:440-669` | 230 | One function per transport |
| `sync_once` | `chat/sync.rs:356-553` | 198 | Pull; merge; push |
| `Composer` class | `webview/pages/app/chat.ts:1793-3267` | 1,475 | Menus (`:2054-2450`), attachments (`:2735-2843`), dictation (`:2961-3060`), suggestions (`:1995-2034`) |
| `renderAppearance` | `webview/pages/app/settings.ts:2341` | 216 | One function per card |
| `LibrariesPage.render`, `editAgent` | `libraries.ts:157`, `agents.ts:151` | 239, 264 | List, detail and editor as separate functions |

Files that hold several unrelated things:

| File | Lines | What it mixes | Split into |
|---|---|---|---|
| `chat/engine.rs` | 3,307 | Event type, prompt assembly, keychain lookup, agent messaging, routing, output spill files, the loop, compaction, the task runner, notifications, titling | `turn/{events, request, loop, compaction, messaging, notify}.rs` |
| `chat/local_models.rs` | 2,141 | Catalogue, downloader, weight loading, batching worker, HTTP endpoint, audio processing, embedder | `models/local/{catalog, download, chat, speech, embed, server}.rs` |
| `chat/api_server.rs` | 1,743 | Settings and tokens plus five wire protocols (OpenAI chat, Anthropic, MCP, Responses, Ollama) and a website widget | One file per protocol |
| `chat/libraries.rs` | 1,760 | Library records, Office and zip text extraction (`:136-347`), archive import, folder and connector sync, search | Extraction belongs with `extraction.rs`; sync apart from records |
| `chat/tasks.rs` | 1,627 | Next-run arithmetic, RSS parsing (`:447-535`), folder watching, webhooks, storage, and the app-wide background loop (`:847`) | Triggers, storage and the loop apart |
| `chat/script_tools/mod.rs` | 3,343 | Signing keys and ledger (`:152-530`), storage, review and promotion, grants, rate limits, execution; 45% of the file is tests | `signing.rs`, `lifecycle.rs`, `run.rs`, and a sibling `tests.rs` |
| `chat/embeddings.rs` | 1,091 | Settings, client, cache, fusion, reranking, and an inline Qdrant client (`:471-820`) | `qdrant.rs` apart |
| `webview/pages/app/chat.ts` | 4,892 | Tool labels, transcript rendering, turn actions, speech, sending, the composer, pickers, three views, queue, drafts, side panels | `chat/{tool-labels, transcript, turn-actions, speech, send, composer/*, home-view, chat-view, compare, queue, drafts, sources-panel}.ts` |
| `webview/pages/app/settings.ts` | 2,909 | Dialog shell, theme, and about 40 cards each with its own hand-written types and backend calls | `settings/{index, profile, providers, audio, permissions, general, appearance, integrations, tools}.ts` |
| `webview/static/styles/app.css` | 1,946 | About 1,500 rules in sections ordered by when features arrived; 63 selectors declared more than once | One stylesheet per interface folder, concatenated by the build |

`conversations_commands.rs` (48 commands) and `misc_commands.rs` (33; its header reads "The
remaining chat commands") hold commands for skills, sharing, translation and agents. They
should be redistributed to the area each belongs to once the folders exist.

## 6. One way to do each recurring thing

Principle: a piece of knowledge has one home. Copies drift, and the drift is where bugs come
from. Two of the cases below have already drifted into visible differences.

### 6.1 Rust

| Pattern | Copies | Evidence | Single home |
|---|---|---|---|
| Load and save a JSON settings file | About 38 | 28 hand-written `read_to_string(..).ok().and_then(from_str).unwrap_or_default()` chains in chat (`memory.rs:102`, `audio.rs:103`, `arena.rs:39`, `privacy.rs:28`, `workflows.rs:98`), 23 private `fn path`, and about 10 more outside chat (`tools/connector_state.rs:53-79`, `mcp/config.rs:111-141`, `reporting.rs:89-103`). `dictation.rs:106-130` and `inapp_browser.rs:24-48` differ in one file name | A generic `SettingsFile<T>` with `path`, `load`, `save`; it writes atomically and logs a parse failure |
| Atomic writes | Mixed | `store::write_json` (`store.rs:43`) and `schedules/store.rs:414-440` write to a temporary file and rename; most other savers call `fs::write` directly, so a crash mid-write can truncate a settings file | The same helper as above |
| HTTP client | 36 | `reqwest::Client::builder()` at 31 sites in chat and 5 outside, with timeouts of 6, 15, 20, 30 and 60 seconds and four user agents | `http::client(timeout)` with two or three named timeouts |
| Keychain lookup for the local account | 23 | `AccountId::parse(LOCAL_ACCOUNT)` + `keychain_secrets` in 17 files (`engine.rs:502`, `commands.rs:43`, `slack.rs:70`) | `secrets::local()` |
| Chat platform integration | 3 | `slack.rs`, `discord.rs` and `telegram.rs` repeat settings (`:38-48`, `:41-51`, `:43-53`), token handling, a `RUNNING` flag with supervisor loop (`:189-213`, `:194-216`, `:304-330`) and a ~107-line `session` | A `ChatPlatform` trait: each service supplies connect, receive and send; pairing, supervision and routing are shared |
| Name validation and slugs | 9 | `valid_name` in `skills.rs:165`, `custom_tools.rs:41`, `vault.rs:84`, `emoji_pack.rs:57`, `script_tools/mod.rs:541`; `store::valid_id`; `slug` in `plugins.rs:380`, `openapi.rs:72`, `animation.rs:109` | One `names` module; these checks guard file paths, so one tested version matters |
| Tool argument parsing | 76 | `.get("k").and_then(Value::as_*)` in 25 files | A `#[derive(Deserialize)]` struct per tool, which also documents the arguments |
| Tool results | 87 | `Outcome { text, is_error }` built by hand; the `ok()` / `err()` helpers are private to `builtins.rs:493,500` | `Outcome::ok` / `Outcome::err` |
| Truncating text | 71 | `chars().take(n).collect()` in 39 files | `text::clip(s, n)` |
| Native file dialog | 21 | The dialog-plus-channel pattern in 9 command files (`libraries_commands.rs:87-104`) | `pick_file(app, filters).await` |
| "Only the main window may call this" | 11 | Four helper definitions (`schedules/bridge.rs:80`, `skills.rs:393`, `notifications.rs:30`, `account.rs:95`) and seven inline checks | One guard function |
| MCP connector ids | 16 | `format!("mcp-{id}")` at 10 sites, `starts_with("mcp-")` at 6 | A small `McpId` type |
| Per-connector special cases | 8 places | Adding a connector touches `tools.rs`, `exposure.rs:123-170`, `icons.rs:13-31`, `commands.rs:92-155`, `registry.rs:300,656,706,714`, `result_store.rs`, and the TypeScript catalog | Exposure class, icon and log redaction as methods on the `Connector` trait |
| Built-in tool declaration | 3 per tool | Name constant, `definitions`, `NAMES` (`builtins.rs:469`) and a match arm | One table entry per tool (see section 5) |

### 6.2 TypeScript

| Pattern | Copies | Evidence | Single home |
|---|---|---|---|
| Call the backend, show the error | 148 | `toast(String(e), 'error')` (46 in `settings.ts`, 28 in `chat.ts`); 54 more `.catch(() => undefined)` that fail silently | `call(command, args)` that reports failures the same way everywhere |
| Bytes to base64 | 7 | `dom.ts:480`, `dictation.ts:218` (exported as `toBase64` and not reused), `chat.ts:906,2765,2784,2797,2810` | Import the existing `toBase64` |
| File size labels | 6 | `coderun.ts:38` divides by 1,024 and `outputs.ts:38` by 1,000, so the same file shows two sizes in two panels | `format.ts` |
| Dates, token counts, initials | 8 | `when()` in `tasks.ts:19`, `versions.ts:17`, `workflows.ts:110`, `context.ts:58`; `main.ts:681,1009`; `settings.ts:217`, `main.ts:666` | `format.ts` |
| HTML escaping | 2 | `share.ts:23` and `chat.ts:4043`; the second does not escape quotes | One `escapeHtml`; fix the second |
| Stored preferences | About 80 uses | Eight near-identical getter and setter pairs in `state.ts:260-391`; 24 keys across 21 files, each with its own `try/catch` | `pref<T>(key, fallback)` |
| Buttons, inputs, dialog footers | About 300 | Raw `h('button', {class: 'btn…'})` 173 times against `button()` 104; `class: 'dialog-actions'` 38 times against `dialogActions()` once; raw inputs 90 against `field()` 16 | Use the components `ui.ts` already has |
| Event listener with fallback | 5 | `api.ts:447`, `pages/theme.ts:34`, `pages/dictate.ts:138`, `bridges/connectors.ts:341`, `bridges/schedules.ts:15` | `shared/tauri.ts` |
| Design tokens | 2 systems | `app.css` defines 61 tokens of its own (`--bg`, `--accent`); `cortex-tokens.css` defines 48 `--cortex-*` tokens used only by the other pages. No name is shared, and `app.css` has about 100 colour literals outside its tokens | One token file for every page |

One sanitisation exception belongs here: `searchmode.ts:81` rewrites `innerHTML` of
already-rendered Markdown to add citation links, bypassing the single safe path and discarding
the listeners it attached. A text-node walk like `dom.ts:274-296` does the same job.

## 7. Errors and state

### 7.1 Errors

Principle: the caller can tell what went wrong, and a failure is never silently turned into a
default.

- **Errors are strings.** `chat/` has 692 `Result<_, String>` and no error type at all; about 440
  conversions are `.map_err(|e| e.to_string())`. The rest of the app adds about 250 more. Typed
  errors exist only in `tools/error.rs`, `schedules.rs:88`, `schedules/store.rs:88` and the
  sandbox crate. A caller cannot tell "not found" from "disk full" from "network down", and text
  meant for the user shares a channel with diagnostics.
  Recommendation: introduce one `thiserror` enum per layer, starting with storage
  (`NotFound`, `Invalid`, `Io`, `Locked`), and convert to a string only in the command layer,
  which is the one place that must.
- **Parse failures become defaults.** 28 load chains return `Default` when a settings file does
  not parse, without logging; `store::read_json` logs (`store.rs:53-60`). A corrupted file then
  looks like a fresh install. The shared helper in 6.1 fixes all of them.
- **Some saves are discarded.** Of 167 `let _ =` in chat, about 11 drop the result of a save
  (`store::save_conversation` three times, `write_json` three times, `at_rest::write` twice).
  Log these at least.
- **Three ways to handle a poisoned lock**: recover (`chat.rs:127-131`), unwrap
  (`at_rest.rs:106`, 11 in `dictation.rs`), or `expect` (33 in `tools/registry.rs`). Pick one and
  put it in a helper.
- **Logging is split** between `log` macros (95) and `println!` / `eprintln!` (59, of which 26
  are the terminal chat's own output).

### 7.2 Hidden state

Principle: a function's behaviour depends on its arguments. State it reaches for sideways makes
it unpredictable and hard to test.

The engine mostly follows this (section 3). The exceptions are about 55 process-wide statics, 43
of them in chat:

| Where | What | Problem |
|---|---|---|
| `chat/sandbox.rs:36-52` | `ALWAYS`, `LOADED` | Caches a setting read from `config_dir` in globals not keyed by it: the first folder asked wins for the whole process. `script_tools` and `index` key by path |
| `chat/local_models.rs:332,337,512,673,1414,1647` | `D`, `C`, `L`, `P`, `S`, `E` | Six globals with one-letter names holding the download list, the loaded model, the job queue, the speech model and the embedder |
| `chat/at_rest.rs:32,35` | `STATE`, `LOCKED` | Changes the behaviour of every read and write in 51 modules; correct as a design, but worth one documented type instead of two statics |
| `chat/engine.rs:747,2232` | `MESSAGE_DEPTH`, session allowances | Leak between tests in one process |
| `chat/sandbox.rs:167-190` | A test-only global | Consulted inside production code |
| `tools/approvals.rs:17-19` | `PENDING`, `REQUEST_SEQ` | Global, although the approval gate is otherwise injected |
| `tools/keychain/pending_deletes.rs:15-16` | `ROOT`, `LOCK` | Must be initialised at `app.rs:509-512` before any keychain read; only a comment says so |
| `slack.rs:189`, `discord.rs:194`, `telegram.rs:304`, `share.rs:133`, `emoji_pack.rs:146` | One `RUNNING` flag each | Goes away with the shared supervisor in 6.1 |

Move these into the structs that already exist for the purpose (`ChatState`, `ToolRegistry`) or
into a struct created at startup and passed down.

In the interface: `state.ts:72` exports one mutable object that eight other files write
directly (24 writes in `main.ts`), there are 41 module-level `let` variables, and modules talk
through four mechanisms (direct calls, window events, registration arrays, constructor
callbacks). Routing writes through a few named functions in `state.ts`, as `applyEvent` already
does for backend events, would give one place to look when the screen shows the wrong thing.

### 7.3 Strings where a closed set is meant

- Run and task statuses are string literals compared elsewhere (`subagents.rs:365,418,442,449`
  against `:342` and `handlers.rs:82`; `terminal.rs`; `tasks.rs:594,686,821`). These are enums.
- `ChatState::record` (`chat.rs:198-213`) matches on the event's serialised JSON (`"started"`,
  `"done"`, `"text"`) although the typed `ChatEvent` exists.
- Version kinds are `(&str, &str)` pairs (`versions.rs:201-265`); trace artifact kinds are 26
  literals in 7 files; `"user"` and `"assistant"` appear about 150 times.
- Event channel names are literals on both sides in three styles (`chat://event`,
  `dictation://x`, `connectors:changed`); `EVENT` (`chat.rs:110`) is the only constant.
- 17 `CORTEX_*` environment variable names are inline literals. One `env.rs` listing them would
  also be their documentation.

## 8. The boundary between interface and backend

Principle: an interface between two parts is described once, and both sides are checked against
that description.

**A command is declared by hand four or five times**: the function, the 344-entry
`generate_handler!` list (`app.rs:54-397`, which the formatter does not touch, hence its two
indent styles), `permissions/*.toml`, `capabilities/*.json`, and the TypeScript caller. The sets
match today, but nothing checks that.

**The permission scheme has collapsed into one entry.** `allow-chat-read-spreadsheet`, described
as "Read an uploaded workbook" (`permissions/chat.toml:402`), grants 143 commands, including
non-chat ones (`browser_prefs`, `app_menu_action`, `dictation_*`). Four commands are granted
twice. Whoever adds a command evidently appends to that list.

**The interface mostly bypasses its typed wrapper.** `api.ts` wraps 65 commands; 287 further
`tauriInvoke(` calls in 31 files pass about 233 distinct command names as string literals (95 in
`settings.ts`, 45 in `chat.ts`), 58 of them with no result type. Types "mirror the Rust structs"
by hand (`api.ts:1-4`), in mixed casing, and `ConnectorManifest` is defined twice with different
shapes (`api.ts:308`, `types/index.ts:59`). A renamed field on the Rust side fails at run time.

**Rust reads a TypeScript file as text.** `tools/connectors/mcp/catalog.rs:12-19` embeds
`webview/shared/connector-catalog.ts` and picks out lines starting with `url: '`. Reformatting
that file would silently empty the list. `feature_flags.rs:12-17` shares a JSON file between the
two sides, which is the pattern to use here too.

Recommendations, cheapest first:

1. Add a test that parses `app.rs`, the permission files and the capability files and fails if
   the three sets differ. This makes today's accidental consistency permanent.
2. Replace the 143-command entry with one permission per area (conversations, libraries,
   settings, …), each with a true description.
3. Generate the handler list and permission files from one table (a build script or a small
   macro), so adding a command is one edit plus the function.
4. Generate TypeScript types and command wrappers from the Rust signatures (`ts-rs` and
   `tauri-specta` are the usual tools; evaluate one on a single command file first). Then the 287
   raw calls become typed calls, and a mismatch is a compile error.
5. Settle command names. Today: `set` 24, `save` 24, `update` 5; `delete` 15, `remove` 6; `list`
   10, `get` 7, and bare nouns. One verb per meaning.
6. Move the logic that has leaked into commands down a layer: `chat_import_plugin` (93 lines,
   `knowledge_commands.rs:527`), `chat_add_documents` (73, `libraries_commands.rs:86`),
   `chat_export_canvas` (66, `conversations_commands.rs:423`), and the `ocr`,
   `conversation_markdown` and `copy_dir` helpers in `commands.rs`. The 12 command files have no
   tests, so logic there is untested logic.

## 9. Names

### 9.1 The same name for different things

| Name | What each one is |
|---|---|
| `skills.rs` / `chat/skills.rs` | The hosted app's script cache / the native app's skills |
| `schedules/` / `chat/tasks.rs` | The hosted app's run queue / the native scheduler |
| `settings.rs` / about 20 chat settings files | Seven hosted-era fields / everything else |
| `tools/` / `chat/tools.rs` | The connector registry / a tool as offered to the model |
| `chat/sandbox.rs` / `chat/script_tools/sandbox.rs` / `crates/cortex-sandbox` | Code execution with bubblewrap / a 9-line re-export / the WASM sandbox |
| `tools/commands.rs`, `settings/commands.rs`, `chat/commands/` | Three command layers |
| `chat/{privacy, lifecycle, context, prompt, evals}.rs` | Same names as the kernel crate's modules they wrap |

Section 4.5 resolves the first three. For the rest, rename by role: `chat/sandbox.rs` →
`code_runner.rs`, `chat/tools.rs` → `tool_dispatch.rs`.

### 9.2 Inherited names

The rename to Cortex left live identifiers and configuration with the old names:

- `LUMO_URL` and `LUMO_HOSTED` (`lib.rs:50,57`, and set in `.github/workflows/ci.yml:25`);
  `window.lumoDesktopVersion` (`main_window.rs:46`, `main.ts:658`); the `lumoAppFiles` wire key
  (`canvas.ts:171`); `window.Lumo*` bridge globals.
- `tauri.conf.json`: the updater endpoint is `lumo.protondownload.com` (`:33`), the publisher and
  signing identity are "Proton AG" (`:44,63`), and `proton-lumo` is a registered link scheme
  (`:38`). `Cargo.toml` still carries the upstream author. **These are worth an owner decision
  independent of code quality**: a build of this fork asks Proton's server for updates.
- File-format markers (`LUMOREST1`, `LUMOSYNC1`, `.lumo`, `lumo-agent`) are compatibility
  constants and must stay, but they are spread over `at_rest.rs:21`, `sync.rs:32`,
  `canvas.rs:117`, `agents.rs:203` and `links.rs:33`. Collect them in one `legacy.rs` beside
  `rename_migration.rs`, with one comment explaining why they keep the old name.

## 10. Tests and tooling

Principle: a rule that no machine checks is a suggestion.

| Finding | Evidence | Recommendation |
|---|---|---|
| Three test files are run by nothing, and five of their nine tests fail | `scripts/connector-builtins.test.mjs`, `connector-connection.test.mjs`, `reset-settings.test.mjs` are referenced by no script, hook or CI job. Run with `node --test`: 3 fail with "navigator is not defined", 2 with "Cannot use import statement outside a module" | Fix and add them to CI, or delete them with the hosted mode (4.5) |
| Interface unit tests hide inside a translation check | `scripts/check-i18n.mjs` also tests vim keys, the Markdown toolbar, prompt variables, emoji scanning and image handling | A real test runner: `*.test.ts` files beside the code, run by `node --test`, under `npm test` |
| The interface's central logic is untested | No tests for `state.applyEvent`, which is pure, nor for anything in `chat.ts` or `settings.ts` | Start with `applyEvent` and the functions extracted in section 5 |
| No linter or formatter for TypeScript, JavaScript or CSS | No ESLint, Prettier, Biome, Stylelint or `.editorconfig`; indentation is 2 spaces in TypeScript, 4 in `index.js`; lines reach 602 columns in `settings.ts` | Add a formatter and a linter with an import-cycle rule; format once in a commit of its own |
| CI builds Linux only | One `apt-get`-based runner. 33 `#[cfg(windows)]` and 38 `#[cfg(target_os = "macos")]` blocks are never compiled or linted | `cargo check` for the Windows and macOS targets in a matrix job |
| End-to-end tests are not in CI | `e2e/README.md` says "CI uses the same `run-in-container.sh` entry point"; `ci.yml` has no such job, and the suite needs a checkout of Proton's web clients | Decide with 4.5. For the native app, a small smoke suite on the same WebDriver setup |
| The 12 command files and `engine/handlers.rs` have no direct tests | 4,600 lines; `batched_qwen2.rs` none; Slack, Discord and Telegram one or two each | Thin commands (section 8) need none; handlers get tests through `TestHost` |
| Network code is tested only live or through environment overrides | No HTTP mocking; `CORTEX_TEST_*` variables gate live tests | A local stand-in server helper shared by tests, as some tests already build ad hoc |
| Inline tests swell the largest files | `script_tools/mod.rs` is 45% tests, `lifecycle.rs` 85% | Sibling `tests.rs` files, as `engine/turn_tests.rs` and `tools/registry/tests.rs` already do |
| No shared manifest settings | No `[workspace.dependencies]` or `[workspace.lints]`; `sha2` and `rand` versions differ between crates; three `rand` versions in the lockfile | Declare shared versions and lints once at the workspace root |
| The toolchain floats | No `rust-toolchain.toml`; CI uses `stable`, so a new compiler release can fail CI on an unrelated change | Pin it and bump deliberately |
| The translation catalog collects code | `scripts/ui-strings.mjs` guesses which string literals are interface text; about 41 of the 2,562 catalog entries are shader comments and CSS values, sent for translation into five languages | Exclude the WebGL files, or mark interface strings explicitly |
| The embedded interpreter is not reproducible in CI | `crates/cortex-sandbox/wasm/js-runtime.wasm` (4 MB) is committed; no job rebuilds it or checks it against its sources | A CI step that rebuilds and compares, or records its hash |
| Dependencies with no user | `react` (dev dependency, imported nowhere); `webview2-com` (no reference under `src/`; confirm on Windows); `anyhow` used in one file | Remove |

## 11. Repository and documentation

- **`.gitignore` ignores `docs/`, and `docs/` is tracked.** A new document, this one included,
  has to be added with `git add -f`, and a contributor's new doc is silently left out. Remove
  the line.
- **There is no map of the code.** `README.md` describes 33 of the 86 chat modules, and its
  project layout section (`:187-216`) describes the hosted app. Add `docs/architecture.md`: the
  layers from 4.2, one line per folder, which module owns which file under `chat/` in the data
  folder, and the rule for adding a command, a tool, a connector and a setting.
- **There is no contributor guide.** The conventions this review recommends (section 13) belong
  in a short `CONTRIBUTING.md`, next to the commands to run before pushing.
- **The changelog stops at the fork.** `changelog.md` has one "Unreleased" entry (the rename);
  the native app's features are recorded only in `feature-parity.md`. The version is 0.5.9 in
  three hand-maintained files.
- **Release scripts have no caller in this repository.** `make-dmg.sh`,
  `make-version-manifest.mjs` and `validate-version.sh` are referenced by nothing here; the
  README points to a `RELEASING.md` in Proton's internal repository. Either document a release
  process for this project or remove them.
- **`scripts/` mixes five purposes in three languages** (build, release, checks, generators,
  tests; shell, Node, Python). Sub-folders `build/`, `release/`, `check/`, `generate/` would say
  what each is for.
- **Generated files are committed without a list.** `src-tauri/gen/schemas/` (650 KB, rewritten
  by every build), the WASM interpreter, the translation files, the Luminance operation table
  and the icons each come from a different script. One table in the architecture document
  naming the generator of each is enough.
- **The browser extension stands alone**: no build, types or tests, its own accent colour
  (`#6d4aff` against the app's `#8b6dff`), and the default port written twice
  (`extension/popup.js:4,113`). At 141 lines this is fine; note it in the README.

## 12. Suggested order

Each step leaves the app working and is independent of the ones after it. Sizes: S is hours,
M is days, L is a week or more.

**Step 0. Decisions for the owner (no code)**

- Keep or remove hosted mode (4.5).
- Updater endpoint, publisher and signing identity in `tauri.conf.json` (9.2).

**Step 1. Quick fixes, no behaviour change (each S)**

1. Remove `docs/` from `.gitignore`.
2. Fix or delete the three orphan test files; add `npm test` and run it in CI.
3. Add the test that handlers, permissions and capabilities match; split the 143-command
   permission by area.
4. Remove unused dependencies; add `[workspace.dependencies]`, `[workspace.lints]`, a pinned
   toolchain and `.editorconfig`.
5. One `format.ts` (fixes the two file-size labels), reuse `toBase64`, one `escapeHtml`
   (fixes the missing quote escaping), fix `searchmode.ts:81`.
6. Key the sandbox's "always allow" cache by folder; name the one-letter statics.
7. Log discarded saves and unparseable settings files.

**Step 2. Shared helpers (each S to M)**

8. `SettingsFile<T>` with atomic writes; migrate the 38 savers module by module.
9. `http::client`, `secrets::local`, `names`, `text::clip`, `Outcome::ok/err`, the main-window
   guard, the file dialog helper.
10. `pref()` and `call()` in the interface; use the existing `ui.ts` components.
11. A formatter and linter for the interface, applied in one commit.

**Step 3. Structure (M to L, mechanical)**

12. Group `chat/` into folders, one per pull request, with re-exports (4.1).
13. Group `webview/pages/app/` (4.4); split `dom.ts`; remove the import cycle.
14. Split `chat.ts` and `settings.ts` along the seams in section 5.
15. Split `app::run`, `builtins::run` and `definitions`, `traced_turn`, `call_model`.
16. Write `docs/architecture.md` and `CONTRIBUTING.md` as the structure settles.

**Step 4. Boundaries (L)**

17. The leaf `types` module and the inversions in 4.2; then the layering check in CI.
18. Tighten visibility folder by folder (4.3).
19. Typed errors, starting with storage (7.1).
20. Hosted mode behind a feature, or removed (4.5).
21. Generate the handler list and permissions; then generate TypeScript types and wrappers (8).
22. One `ChatPlatform` for Slack, Discord and Telegram.

**Step 5. Crates (L, continuing the kernel plan)**

23. Once `tools/` and `storage/` no longer take the app handle in their loaders
    (`tools/connector_state.rs:45-79` is the typical blocker), move them into
    `crates/cortex-tools` and `crates/cortex-store`, as the kernel plan's section 5 sketched.
    A crate boundary is the strongest form of the layering rule: the compiler enforces it.

## 13. Rules to hold from here on

Short enough to fit in `CONTRIBUTING.md`; each is checkable.

1. A new module goes in the folder for its area, and may import only from its own layer or
   below (CI script).
2. A function over about 100 lines, or a file over about 800 non-test lines, needs a reason in
   review (`clippy::too_many_lines` can enforce the first).
3. Items are private unless another folder uses them.
4. Settings files, HTTP clients, keychain reads, name checks and formatters come from the shared
   helpers; a second implementation is a review finding.
5. Fallible functions below the command layer return a typed error; only commands convert to
   text.
6. A closed set of values is an enum, on both sides of the boundary.
7. No new process-wide `static` holding mutable state; put it in a struct created at startup.
8. The interface calls the backend only through generated or wrapped functions, never with a
   command name as a string literal.
9. A command is a thin wrapper: parse, call one domain function, convert the result.
10. Tests for a file over about 300 lines of tests live in a sibling `tests.rs`.
