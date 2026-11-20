# Implementation plan for the code base improvements

2026-11-20 · based on commit `73722bf` · implements [codebase-improvements.md](codebase-improvements.md)

The review says what to change and why. This plan says how: 34 work packages in five phases,
each with the files it touches, the steps, how to tell it is done, and what can go wrong. Section
references such as "review 6.1" point into the review.

Nothing in this plan changes what the app does for its user, except the handful of small fixes
listed in package 0.8.

## Status, 2026-11-20

Work happened on one branch (pull request #7) rather than one pull request per package; each
package is one or more commits there. Every row below is printed by
`python3 scripts/measure-plan.py` from the code as it is (`--check` fails on a missed target; CI
runs it), so the table can be checked rather than trusted:

| Measure | Start | Now | Target |
|---|---|---|---|
| Tracked files that are ignored | 12 | 0 | 0 |
| Interface tests failing | 5 of 9, not run | 0, run in CI | 0 |
| Commands in the largest permission | 143 | 48 | Under 50 |
| Hand-written settings loaders | About 28 | 0 | 0 |
| HTTP clients built in place | 36 | 10, each for a reason (below) | 1 |
| Raw error toasts | 148 | 1, the helper itself | 0 |
| Files directly in `src-tauri/src/chat/` | 86 | 2 | Under 15 |
| Files directly in `webview/pages/app/` | 51 | 0 | Under 5 |
| Largest interface file | 4,892 lines | 759 (`webview/bridges/connectors.ts`; 722 in `pages/`) | Under 800 |
| Rust functions over 150 lines | 11 | 0, in the app and the crates (the generated `handlers.rs` aside) | 0 |
| Rust files over 800 lines before their test module | 16 | 0, in the app and the crates | 0 |
| Layering violations allowed | About 69 modules in one cycle | 0 | 0 |
| Public functions in chat | About 1,060 | 376 | Under 400 |
| `Result<_, String>` in chat, outside `commands/` | About 420 | 0 below the entry points (the two kernel `Steps` methods keep the kernel's type) | 0 below the entry points |
| Mutable process-wide statics | About 55 | 9, each with its reason (`scripts/check-statics.py`) | Under 10 |
| Hosted-only commands in the default build | At least 10 | 0 `schedule_*` or `skill_*`; the sign-in command stays (below) | 0 |
| Hand edits to add a command | 4 to 5 | 1, plus a script | 1 |
| Backend calls by string name | 287 | 0 unchecked: every name is checked against the generated command list | 0 |

An earlier version of this table gave 0 for long functions and long files while
`toolkit/sandbox.rs` (913 lines, `run_in` 293), `tools/connectors/mcp.rs` (1,134 lines,
`ensure_connected` 230), `cortex-tools/src/registry.rs` (1,184), `automation/evals.rs` (815) and
`traced_turn` (165) were over: the measure had been taken on `chat/` alone. Each is split now,
and the script measures the whole app and the crates.

**Done**: phase 0 (0.1 to 0.8), phase 1 (1.1 to 1.6), phase 2 (2.1 to 2.8), 3.1 to 3.11, 4.1.

4.1 in detail: `cortex-store` holds the conversation store (conversations, their index, backups and
  the trash, version history, the checkpoints of answers in progress), settings files,
  encryption at rest and the storage errors, with no window, keychain or network in it; the app
  re-exports its modules under `chat::`. Sync, the vault and the importers stay in the app, as
  they use Tauri, the keychain or the network. `cortex-tools` holds the tool contract: the
  connector trait and registry, approval and exposure rules, credentials and connector state,
  tool results, OAuth, and the echo connector as a reference; the registry reports failures
  through a hook the app sets. The concrete connectors that need the window or the keychain
  (MCP, filesystem, bash, browser, mail), the keychain itself and the Tauri commands stay in
  `tools/`, which re-exports the crate's modules.

2.7 in detail: one token file (`static/styles/tokens.css`) for every page, and no colour literal
  outside it (the agent tints computed from a hue variable aside). `app.css` is now 59 files in
  one folder per interface area under `webview/styles/app/` (`core`, `shell`, `chat`, `ui`,
  `settings`, `pages`, `canvas`, `theme`, `voice`); `order.txt` lists them in the order the build
  joins them, which is the order the rules had before, so the built stylesheet is byte-for-byte
  the one it replaced. A stylesheet left out of `order.txt` fails the build.

3.7 in detail: state a module keeps while the app runs is a type of its own in `ChatState`,
  reached with `state.part::<T>()` from the host a turn has or the command's app handle: the
  background commands, the computer tool's screenshots, the workbench's status, emoji downloads,
  the share server, sync's one-at-a-time lock and the tool vectors; the API server's request log
  is in its own managed state, and the link prompt and the connectors-changed debounce in
  Tauri's. Some statics simply went: the queue for agent-to-agent turns (a turn now spawns the
  next directly), the sandbox's settings cache and sync's derived key (each read is cheap).
  Writes that read a file, change it and write it back (the activity log, the pending keychain
  deletes) share one per-file lock in `cortex-store`. The nine that stay are listed by
  `scripts/check-statics.py`, each with a `Process-wide:` comment.

**Checked in the real app**: section 8's checklist runs as `e2e/user-journeys/` (the real app
under a virtual display, through tauri-driver, against a real model), in the light and dark themes,
and passes; a second journey covers switching chats while answers run, unread marks, two answers at
once and recovering an answer the app was killed in the middle of; a third shows the app adapting to
its user: what they say about themselves in one chat is kept as facts they can see and edit, and a
new chat acts on it without being told; with learning turned on, an agent's repeated procedure
becomes a skill, the weekly review proposes merging a fact said twice, and the daily evals score
the starter suite. Running them found and fixed: the daily evals running nothing for a user with no
suite (and then the starter written after the copy it runs on), the note that an agent learned a
skill never shown, a background call (the reflection that keeps a skill) lost to one 502, a model
saying a fact was saved without saving it, background answers not marked unread, an answer lost when the app closed mid-way, no way to retry a
message stopped before any text, new chats indistinguishable while answering, and titles written
from a clipped first line.

**Deliberate departures**:

- Of the 63 selectors declared more than once (2.7), four were merged. The rest have a rule
  between the two declarations that sets the same properties, or sit in different media or
  theme blocks, so merging them would change which rule wins; they stay where they are. The
  `--cortex-*` token names stay as aliases until the old settings page is rewritten.
- Which connectors are exposed, and which send data out, stays in central fail-closed tables
  (`tools/exposure.rs`) instead of moving onto the connector trait (review 6.4): one table is
  easier to audit, and pseudo-connectors have no implementation to carry it.
- `pending_deletes` keeps its process-wide root (3.7): it already fails loudly when not set up.
- The sign-in command (`start_browser_login`) stays in the default build (3.8): the window's
  navigation rules, in every build, send a page that asks for another account to it. The
  scheduler's and the skill scripts' commands are in the hosted build only.
- The local models' state is one process-wide `LocalModels` rather than a field of `ChatState`
  (3.7): the weights take gigabytes and the processor runs one model at a time, and the model
  layer (connecting a model, embedding, transcribing) is reached from the app, the CLI and the
  local API with no app state to carry it. The script tools' signing record stays one per process
  for the same reason as a security check: there must be one record per data folder.
- Of the ten HTTP clients still counted, two are the helper itself and one is in a test. Five are
  built in place on purpose: web search sends a full browser user agent, crash reports send
  Sentry's headers, the Luminance connector turns redirects off, the HEX image server is local
  and skips the proxy, and model downloads set only a connect timeout. The MCP OAuth client is
  handed to the auth library, and the hosted sign-in (`browser_login.rs`, `hosted` builds only)
  is left as it is.

**Found on the way**: a DeepSeek model behind an OpenAI-compatible gateway got a 404, because
the `/v1` of the gateway's address was dropped; fixed, and checked with a headless run against a
real gateway.

## 1. How to read this plan

- **A package is one pull request**, unless it says "one pull request per folder" or similar.
- **Sizes**: S is under half a day, M is one to three days, L is one to two weeks. They are
  estimates for someone who knows the code, and assume the Rust build is warm.
- **Phases are ordered by risk and payoff**, not by strict dependency. Section 9 lists the
  ordering constraints that are real; everything else can be reordered or done in parallel.
- **Phase 0 is worth doing even if nothing else is.** It is about two days of work in total and
  makes every later change safer.

| Phase | What | Packages | Total size | Risk |
|---|---|---|---|---|
| 0 | Safety net and hygiene | 0.1 to 0.8 | About 2 to 3 days | Low |
| 1 | Shared helpers | 1.1 to 1.6 | About 2 weeks | Low |
| 2 | Structure: move and split | 2.1 to 2.8 | About 4 to 5 weeks | Low per step, high in churn |
| 3 | Boundaries | 3.1 to 3.11 | About 8 to 10 weeks | Medium |
| 4 | Crates | 4.1 | About 2 weeks | Medium |

## 2. Rules for every change

1. **One kind of change per pull request.** A move contains no edits; an edit contains no moves;
   a reformat contains nothing else. This keeps each diff reviewable and each revert small.
2. **The gate.** Every pull request passes all of this before merge; it is what CI runs today,
   plus the steps this plan adds:

   ```bash
   npm run typecheck && npm run build:webview && npm run check:i18n && npm run check:voice
   npm test                                        # from package 0.5 on
   cd src-tauri
   cargo fmt --all --check
   cargo clippy --workspace --features cortex/test-utils --all-targets -- -D warnings
   cargo test --workspace --features cortex/test-utils
   cd .. && python3 scripts/check-kernel-plan.py
   ```

3. **What must not change**, because users' data and the two halves of the app depend on it:
   - File names and formats under the data folder (`chat/*.json`, `conversations/`, the
     `LUMOREST1` and `LUMOSYNC1` markers, `.lumo` sync files).
   - Whether a given file is encrypted at rest (see package 1.1).
   - Keychain service and account names.
   - Command names, event names and payload shapes, until package 3.9 changes them on both sides
     at once.
   - `src-tauri/i18n/catalog.json` during moves and splits: it must come out byte-identical.
4. **Interface changes get a manual pass.** The interface has no automated tests of its own, so
   every pull request that touches `webview/pages/app/` walks the checklist in section 8.
5. **Documents that cite line numbers** (`agent-kernel-plan.md`, `review-coverage.md`, the
   review) state the commit they were written against. Do not chase their line numbers after
   each move; update file paths once per phase.

## 3. Decisions

### 3.1 For the owner, before the packages named

| Decision | Blocks | Default this plan assumes |
|---|---|---|
| **D1. Hosted mode**: keep it, or remove it | 0.4, 3.8 | Keep it, behind a build feature (package 3.8). Isolating it first makes a later removal a single deletion, so this default closes no door |
| **D2. Updater and publisher**: `tauri.conf.json` points updates at `lumo.protondownload.com` and names "Proton AG" as publisher | Nothing here; it is a release matter | No change in this plan. Until decided, a build of this fork asks Proton's server for updates |
| **D3. Hosted CI runners for Windows and macOS** | 0.7 | Use GitHub-hosted runners for one compile-only job |

### 3.2 Technical choices made here

These are recommendations; each is easy to swap before its package starts.

| Choice | Recommendation | Why |
|---|---|---|
| Formatter and linter for the interface | Biome, as one dev dependency | The repository delays new packages by three days (`.npmrc`) and keeps few dependencies; one tool covers both jobs. ESLint with Prettier is the fallback |
| Permissions | One per command area, about 12 | All chat commands are granted to the same windows (`capabilities/chat.json`), so per-command entries add 289 blocks and no protection |
| Handler list and permission files | Generated by a script from the `#[tauri::command]` functions, committed, checked for drift in CI | The same pattern as `i18n/catalog.json` today; no new build-time machinery |
| TypeScript types for commands | Decide after a two-day trial (package 3.10) | Whether the available generators fit the Tauri and rig versions pinned here should be checked, not assumed |
| Where shared Rust helpers live | A new top-level module `support/`, below `chat/` and `tools/` | Both need them; it becomes the `cortex-store` crate in phase 4 |
| Test runner for the interface | `node --test` over esbuild-bundled `*.test.ts` | Already how `scripts/check-i18n.mjs` works; no new dependency |

## 4. Phase 0: safety net and hygiene

Independent of each other unless noted. Do these first.

### 0.1 Stop ignoring `docs/` (S)

- **Change**: remove the `docs/` line from `.gitignore`.
- **Done when**: `git ls-files -ci --exclude-standard` prints nothing.

### 0.2 A test that the command surface is consistent (S)

- **Why first**: 0.3, 3.8 and 3.9 all edit the command lists; this test is what makes that safe.
- **Change**: new `src-tauri/tests/ipc_surface.rs`. It reads, as text, `src/app.rs` (the names
  inside `generate_handler!`), every `permissions/*.toml` and every `capabilities/*.json`, and
  asserts:
  1. every handler appears in exactly one permission's `commands.allow`;
  2. no permission names a command that is not a handler;
  3. every permission identifier is granted by at least one capability, and every `allow-*`
     identifier a capability names exists.
- **Expect**: assertion 1 fails at first for four commands granted twice (`chat_transcribe`,
  `dictation_prefs`, `chat_record_status`, `chat_record_stop`). Check which windows need each
  before removing a duplicate: the dictation and recorder windows have their own capability
  files, so the second grant may be deliberate. If it is, relax assertion 1 to "at least one".
- **Done when**: the test passes in `cargo test` and fails if a handler is commented out.

### 0.3 Split the 143-command permission (S)

- **Depends on**: 0.2.
- **Change**: in `permissions/chat.toml`, replace `allow-chat-read-spreadsheet` (line 402) with
  one permission per area, named after the command files: `allow-chat-conversations`,
  `allow-chat-knowledge`, `allow-chat-libraries`, and so on, each with a description that is
  true. Move `browser_prefs`, `set_browser_prefs`, `app_menu_action` and the `dictation_*`
  commands to `permissions/desktop.toml`. Update `capabilities/chat.json` to grant the new
  identifiers. Leave the 124 single-command permissions alone; package 3.9 regenerates the file.
- **Watch out**: a command missing from every permission is rejected at run time, not at build
  time. Package 0.2's test is the guard.
- **Done when**: no permission's description disagrees with its commands; the app starts and the
  checklist in section 8 passes.

### 0.4 The three orphan test files (S)

- **Depends on**: D1. If hosted mode is to be removed, delete the files and skip the rest.
- **Change**: make them pass, then run them (package 0.5).
  - `scripts/connector-builtins.test.mjs`: three tests fail with "navigator is not defined".
    Add a `navigator` stub to the `vm` context the bundle runs in.
  - `scripts/reset-settings.test.mjs`: two tests fail because `pages/settings/index.js` now
    begins with an `import`. Bundle it with esbuild first, as `connector-builtins.test.mjs`
    already does for its target.
- **Done when**: `node --test scripts/*.test.mjs` reports 9 of 9.

### 0.5 A test command for the interface (S to M)

- **Change**:
  1. New `scripts/test-webview.mjs`: bundle every `webview/**/*.test.ts` with esbuild into a
     temporary folder and run `node --test` on the output.
  2. `package.json`: `"test": "node scripts/test-webview.mjs && node --test scripts/*.test.mjs"`.
  3. Move the unit checks out of `scripts/check-i18n.mjs` (vim keys, Markdown toolbar,
     dev-server preview, prompt variables, emoji scanning, image handling, wake-word matching)
     and out of `check-vad.mjs` and `check-live.mjs` into `*.test.ts` files beside the modules
     they test. `check-i18n.mjs` keeps only the catalog and matcher checks its name promises.
  4. First new test: `state.applyEvent`, one case per `ChatEvent` variant.
  5. `.github/workflows/ci.yml`: add `npm test` to the Webview job; `.githooks/pre-push`: run it
     after the type check.
- **Done when**: CI runs `npm test`; `check-i18n.mjs` mentions no module but `i18n.ts`.

### 0.6 Manifests, toolchain and editor settings (S)

- **Change**:
  - `src-tauri/Cargo.toml`: add `[workspace.dependencies]` for crates used by more than one
    member (`serde`, `serde_json`, `sha2`, `rand`, `tokio`, `thiserror`) and reference them with
    `workspace = true`; add `[workspace.lints]` and `lints.workspace = true` in each member.
  - New `src-tauri/rust-toolchain.toml` naming the current stable release with `rustfmt` and
    `clippy`.
  - `package.json`: remove `react`.
  - New `.editorconfig`: UTF-8, LF, final newline, 2 spaces for TypeScript, CSS and JSON, 4 for
    Rust.
  - `webview2-com` (`Cargo.toml:129`) has no reference under `src/`. Remove it only after a
    Windows build confirms nothing needs it (package 0.7 provides that build).
- **Done when**: each shared dependency's version is written once, at the workspace root; the
  gate passes.

### 0.7 Compile the Windows and macOS code in CI (S)

- **Depends on**: D3.
- **Change**: a new job in `ci.yml` with a matrix of `windows-latest` and `macos-latest` that
  runs `npm ci && npm run build:webview` and then `cargo clippy --workspace -- -D warnings`. No
  tests, no bundling.
- **Expect**: first run to report warnings in the 33 Windows-only and 38 macOS-only blocks that
  no CI has linted. Fix them in the same pull request.
- **Done when**: the job is green and required.

### 0.8 Small fixes (S each; the only user-visible changes in this plan)

| Fix | Where | Visible effect |
|---|---|---|
| One file-size formatter | New `webview/pages/app/format.ts`; replace `coderun.ts:38`, `outputs.ts:38`, `settings.ts:237,762`, `libraries.ts:51`, `uploads.ts:48` | Sizes agree between panels |
| Reuse `toBase64` | Replace the loops at `dom.ts:480`, `chat.ts:906,2765,2784,2797,2810` with the export from `dictation.ts:218`, moved to `format.ts` | None |
| One `escapeHtml` | `share.ts:23` is the complete one; use it at `chat.ts:4043` | Exported pages escape quotes |
| Citation links without `innerHTML` | `searchmode.ts:81`: walk text nodes as `dom.ts:274-296` does | Copy buttons keep working in search answers |
| Key the "always allow" cache by folder | `chat/sandbox.rs:36-52` | None |
| Name the one-letter statics | `chat/local_models.rs:332,337,512,673,1414,1647` | None |
| Log discarded saves | The roughly 11 `let _ =` on `save_conversation`, `write_json`, `at_rest::write` | A line in the log when a save fails |

## 5. Phase 1: shared helpers

Each package adds one helper and moves its callers onto it. Migrate callers a few files per
pull request; the helper and the old code coexist until the last caller moves.

### 1.1 One way to load and save a settings file (M)

- **Change**: new `src-tauri/src/support/settings_file.rs`.

  ```rust
  /// A JSON settings file at a fixed place under the config folder.
  pub struct SettingsFile<T> { relative: &'static str, sealed: bool, _t: PhantomData<T> }

  impl<T: Serialize + DeserializeOwned + Default> SettingsFile<T> {
      pub const fn sealed(relative: &'static str) -> Self;   // goes through at_rest, as chat files do
      pub const fn plain(relative: &'static str) -> Self;    // plain text, as account settings do
      pub fn path(&self, config_dir: &Path) -> PathBuf;
      pub fn load(&self, config_dir: &Path) -> T;            // default if absent; logs if unreadable
      pub fn save(&self, config_dir: &Path, value: &T) -> Result<(), String>;   // temp file, rename
  }
  ```

  A module then declares `const FILE: SettingsFile<Settings> = SettingsFile::sealed("chat/slack.json");`
  and its private `path`, `load` and `save` functions go.
- **Migrate**: the 28 hand-written chains in `chat/` (start with `slack.rs`, `discord.rs`,
  `telegram.rs`, `memory.rs`, `audio.rs`, `arena.rs`, `privacy.rs`, `workflows.rs`); then
  `dictation.rs:106-130` and `inapp_browser.rs:24-48`; then the plain ones (`settings.rs:103-122`,
  `tools/connector_state.rs:53-79`, `mcp/config.rs:111-141`, `proton_mail/config.rs`,
  `luminance/config.rs`, `mcp/tool_cache.rs`, `reporting.rs:89-103`).
- **Watch out**:
  - **Do not change whether a file is sealed.** For each migrated file, check whether it is read
    with `at_rest::read_to_string` or `std::fs` today and pick `sealed` or `plain` to match.
  - Some loaders report a parse failure to the user (`settings.rs:108` calls `config_failure`).
    Keep that by giving `load` a variant that returns the error.
  - `reporting.rs` reads its file before Tauri starts, with a hand-built folder path. It can use
    the helper with that path.
- **Done when**: `grep -rn "unwrap_or_default()" src-tauri/src | grep from_str` finds no settings
  loader; every settings write goes through a temporary file.

### 1.2 Small Rust helpers (M)

One module each under `src-tauri/src/support/`, with tests; migrate callers in follow-up pull
requests.

| Helper | Replaces | Callers |
|---|---|---|
| `http::client(Timeout)` with named timeouts and one user agent | 36 `reqwest::Client` constructions | 26 chat files, 5 others |
| `secrets::local()` | `AccountId::parse(LOCAL_ACCOUNT)` + `keychain_secrets` | 23 sites in 17 files; also absorbs `engine::api_key` (`engine.rs:501`) |
| `names::{valid_id, valid_name, slug}` | Five `valid_name`, `store::valid_id`, three `slug` | Compare the nine bodies first: where the rules differ (length, allowed characters), keep each rule under its own name. These guard file paths |
| `text::clip(s, n)` | 71 `chars().take(n).collect()` | 39 files |
| `Outcome::ok` / `Outcome::err` | 87 struct literals; make `builtins.rs:493,500` public methods | About 20 files |
| `guard::main_window(&webview)` | Four helper definitions and seven inline checks | 11 sites |
| `dialogs::pick_file(app, filters).await` | 21 dialog-plus-channel blocks | 9 command files |
| `locks::lock(&mutex)` | Three poisoning conventions | Recover, as `chat.rs:127-131` does |

- **Done when**: each grep in section 7 for these patterns reaches its target.

### 1.3 Interface helpers (M)

- **Change**, all under `webview/pages/app/` (they move to `core/` in package 2.3):
  - `format.ts`: sizes and base64 (from 0.8), plus `when()`, token counts and `initials`.
  - `prefs.ts`: `pref<T>(key, fallback)` returning `{ get, set }` with the `try/catch` inside.
    Replace the eight pairs in `state.ts:260-391`, then the other uses across 21 files.
    **Keep every key string exactly**, or users lose their preferences.
  - `call.ts`: `call<T>(command, args)` that invokes and, on failure, shows the toast and
    rethrows; `callQuiet` for the 54 places that intentionally ignore failure, so that silence
    is a visible choice. Replace the 148 `toast(String(e), 'error')` sites file by file.
  - `shared/tauri.ts`: move `listen` here from `api.ts:447` and use it in `pages/theme.ts:34`,
    `pages/dictate.ts:138`, `bridges/connectors.ts:341`, `bridges/schedules.ts:15`.
- **Done when**: `localStorage.` appears only in `prefs.ts` and `storage-migration.ts`.

### 1.4 Share the connector catalog as data (S)

- **Why now**: it must happen before any formatter touches the TypeScript (package 1.5).
- **Change**: move the catalog's entries from `webview/shared/connector-catalog.ts` into
  `webview/shared/connector-catalog.json`; the `.ts` file imports the JSON and exports it typed.
  `tools/connectors/mcp/catalog.rs:12-19` parses the JSON with `serde_json` instead of matching
  lines that begin `url: '`. This is how `feature_flags.rs:12-17` already shares its file.
- **Done when**: the tests in `catalog.rs` pass and reformatting `connector-catalog.ts` cannot
  change what Rust reads.

### 1.5 Formatter and linter for the interface (M)

- **Depends on**: 1.4. Schedule it when few branches are open: it touches every file.
- **Change**:
  1. Add the tool and its configuration, set to the code's current style (2 spaces, single
     quotes, semicolons) with a line width that the code can meet without mass rewrapping.
  2. One commit that only reformats `webview/`, `scripts/` and `extension/`. List its hash in a
     new `.git-blame-ignore-revs`.
  3. A second commit that turns on lint rules and fixes what they find. Start with the
     defaults; add a rule forbidding `innerHTML` assignment outside `ui/markdown.ts`.
  4. `package.json`: `lint` and `format` scripts; CI runs `lint`.
- **Watch out**:
  - `scripts/ui-strings.mjs` finds interface text by scanning string literals. After the
    reformat commit, run `npm run i18n:catalog`; the catalog must not change. If it does, the
    formatter changed a literal the scanner depends on: adjust the configuration, not the
    catalog.
  - `scripts/connector-connection.test.mjs:30-33` cuts `index.js` between function-name markers.
    Exclude `pages/settings/index.js` from formatting; it is frozen.
- **Done when**: CI fails on an unformatted file; `catalog.json` is byte-identical.

### 1.6 Stop collecting code into the translation catalog (S)

- **Change**: in `scripts/ui-strings.mjs`, skip `orbgl.ts` and `backdropgl.ts` and literals that
  are CSS values. Regenerate; remove the dropped entries from the five language files.
- **Done when**: the catalog has no entry beginning `//` and none containing `gradient(`.

## 6. Phases 2 to 4

### Phase 2: structure

These packages move code without changing it. They are mechanical, and the gate catches nearly
every mistake; the cost is conflicts with other open branches, so do them in a quiet period and
quickly.

### 2.1 Absolute paths in `chat/` (S, one pull request)

- **Why**: the chat files refer to each other as `super::store` (937 `super::` uses). Once a
  file moves into a sub-folder, `super` means the sub-folder and every such path breaks. Paths
  written `crate::chat::store` keep working wherever the file lives.
- **Change**: rewrite `super::<module>` to `crate::chat::<module>` in every top-level file of
  `src-tauri/src/chat/`, and `super::super::<module>` inside their test modules. Leave
  `use super::*;` in test modules alone.
- **Done when**: the gate passes with no other change.

### 2.2 Group `chat/` into folders (M, one pull request per folder)

- **Depends on**: 2.1.
- **Change**, per folder in the table of review 4.1 (`storage/` first, since it has the fewest
  dependencies; `turn/` last):
  1. `git mv` the files into `chat/<folder>/`; add `chat/<folder>/mod.rs` declaring them.
  2. In `chat.rs`, replace the moved `pub mod x;` lines with `pub mod <folder>;` and
     `pub use <folder>::{x, y, …};`. Every existing `crate::chat::x` path keeps resolving.
  3. Fix `include_str!` paths that are relative to the file: `api_server.rs:690` (`widget.js`,
     which moves with it), `workbench.rs:31`, `ui_language.rs:35-44`,
     `script_tools/mod.rs:2127-2128`.
- **Also**: move `ChatState` (`chat.rs:113-319`) to `chat/state.rs`; move `batched_qwen2.rs`
  under `models/local/`.
- **Done when**: `chat.rs` contains only the folder list, the re-exports and constants; `git log
  --follow` still traces each file.

### 2.3 Group the interface into folders (M, one pull request per folder)

- **Depends on**: 1.5, so moves are not mixed with reformatting.
- **First**, in its own pull request: make `scripts/ui-strings.mjs:143` walk
  `webview/pages/app/` recursively. Today it reads one level only, so the first file moved into
  a sub-folder would drop its strings from the catalog, and `check:i18n` would not notice
  because it regenerates the catalog with the same scanner. Update the hard-coded paths in
  `scripts/check-i18n.mjs`, `check-vad.mjs`, `check-live.mjs` (or do 0.5 first, which removes
  them).
- **Change**: move files into the folders of review 4.4; fix relative imports (the type check
  finds every one); update the entry `pages/app/main.ts` in `scripts/build-webview.mjs` if
  `main.ts` moves to `shell/`.
- **Rename while moving**: `context.ts` → `pages/skills.ts` and `pages/knowledge.ts`;
  `pages.ts` → `pages/connectors.ts` and `pages/instructions.ts`; `fluent.ts` →
  `ui/emoji-fluent.ts`.
- **Done when**: `catalog.json` is byte-identical before and after; the checklist passes.

### 2.4 Split `dom.ts`, `chat.ts` and `settings.ts` (L, several pull requests)

- **Depends on**: 2.3.
- **Order** (each is one pull request, smallest first):
  1. `dom.ts` → `ui/h.ts`, `ui/icons.ts`, `ui/markdown.ts`, `ui/overlay.ts`. Move the three
     backend calls (`:239`, `:343`, `:482`) out to their callers. Keep `dom.ts` for one release
     as a file that re-exports the four, so the other 28 importers change later.
  2. Break the cycle: move `appUrl` and `appFiles` from `canvas.ts` to `canvas/app-files.ts`;
     replace the dynamic `import('./share')` at `canvas.ts:233,282` with a normal import.
  3. `chat.ts`, by the seams in review section 5: tool labels (`:35-120`), transcript
     (`:216-760`), turn actions (`:854-1445`), speech (`:1445-1499`), send (`:1581-1790`), then
     the composer's menus, attachments, dictation and suggestions, then the three views, then
     queue, drafts and the side panels.
  4. `settings.ts`: one file per section; each card's local type moves with it.
  5. `main.ts`: sidebar, search dialog, project editor and usage dialog into `shell/`; theme
     application no longer imports the settings dialog (`main.ts:24`).
- **While there**: in each extracted file, replace raw `h('button', …)` and `h('input', …)` with
  the `ui.ts` components. Do not do this as a separate sweep.
- **Tests**: each extracted piece of pure logic gets a `*.test.ts` (tool labels, queue, drafts,
  prompt history).
- **Done when**: no file under `webview/pages/app/` exceeds 800 lines; the import-cycle check
  (package 3.3) reports none.

### 2.5 Split the oversized Rust functions (M to L)

Protected by existing tests where noted; add tests first where there are none.

| Function | Split into | Safety net |
|---|---|---|
| `app::run` (`app.rs:14-658`) | `cli::dispatch`, `plugins`, `handlers` (the list, in its own file), `setup::{state, deep_links, shortcuts, background}`, `on_exit` | None automated: start the app in both modes and run the checklist |
| `builtins::run` and `definitions` (`builtins.rs:75-967`) | One function and one schema per tool, in a table | `engine/turn_tests.rs`; add one test per built-in that lacks one before splitting |
| `traced_turn` (`engine.rs:1241-1706`), `call_model` (`:1736-2039`), `run_calls` (`:2041`) | Prepare, choose model, loop, record; build request, stream, fail over | The 42 turn tests |
| `sandbox::run_in` (`sandbox.rs:365-656`) | Prepare workspace, build command, run, collect | Its inline tests |
| `McpConnector::ensure_connected` (`mcp.rs:440-669`) | One function per transport | `mcp/tests.rs` (27 tests), `tests/mcp_stdio.rs` |
| `sync_once` (`sync.rs:356-553`) | Pull, merge, push | Its inline tests |

- **Done when**: no function exceeds 150 lines (the `awk` measure in section 7), then turn on
  `clippy::too_many_lines` at that threshold in `[workspace.lints]`.

### 2.6 Split the oversized Rust files (L, one pull request per file)

- **Depends on**: 2.2.
- **Change**: each file in the second table of review section 5 becomes a folder module whose
  `mod.rs` re-exports the same names. Move inline tests of more than about 300 lines to a
  sibling `tests.rs` in the same step (`script_tools/mod.rs`, `lifecycle.rs`).
- **Order**: `api_server.rs` (five protocols that barely share code), `embeddings.rs` (the
  inline Qdrant client), `local_models.rs`, `script_tools/mod.rs`, `libraries.rs`, `tasks.rs`,
  `engine.rs` last.
- **Done when**: no file under `src-tauri/src/` exceeds 800 non-test lines.

### 2.7 Stylesheets (M)

- **Change**: one token file for every page (merge `cortex-tokens.css` and the 61 tokens at the
  top of `app.css`, keeping both sets of names as aliases at first); split `app.css` into one
  file per interface folder, concatenated by `scripts/build-webview.mjs` in a fixed order;
  merge the 63 selectors declared more than once.
- **Watch out**: rule order decides which rule wins. Concatenate in the order the sections
  appear today, and compare screenshots of each page in both themes before and after.
- **Done when**: no colour literal outside the token file, except in the WebGL files.

### 2.8 Documentation (M, written as phase 2 settles)

- `docs/architecture.md`: the layers, one line per folder, which module owns which file under
  the data folder, the table of generated files and their generators, and how to add a command,
  a tool, a connector and a setting.
- `CONTRIBUTING.md`: the gate, the rules in review section 13, the manual checklist.
- `README.md`: replace the module table and the hosted-mode layout with a pointer to the
  architecture document.
- `scripts/`: sub-folders `build/`, `release/`, `check/`, `generate/`; update `package.json`,
  `ci.yml`, `e2e/` and the documents that name them.

### Phase 3: boundaries

Here the code changes shape, not just place. Each package needs review by someone who knows the
area.

### 3.1 Leaf modules (M)

- **Change**:
  - `chat/types.rs`: move `ChatTool` and `Outcome` out of `tools.rs`, and `ChatEvent` out of
    `engine.rs`. Re-export from the old places for one release.
  - `support/at_rest.rs`: move `chat/at_rest.rs` here; `dictation.rs` and `inapp_browser.rs`
    stop importing from `chat`.
  - `account.rs:77` takes the local account constant from `support`, not `chat`.
- **Done when**: `host`, `tools` and `engine` no longer import each other for types alone.

### 3.2 Turn the upward calls around (L)

One pull request per row of the table in review 4.2.

| Upward call | Change |
|---|---|
| `versions.rs:199-265` names every domain | `versions` exposes `register(kind, load, restore)`; each domain registers at startup; `versions` iterates its table |
| `tasks.rs:894` calls `engine::run_task`; `:860` calls `commands::registry_connectors` | `tasks::start(runner, connectors)` receives both as parameters from `commands::setup` |
| `tools/connectors/bash.rs:298` calls `chat::vault::current()` | The registry's host supplies extra environment variables; chat implements that |
| `tools` ↔ `settings`, `account`, `feature_flags` | Settings loaders move to `support`; `account` calls a hook that `tools` registers |
| `tasks.rs:847`: the scheduler loop also syncs libraries and runs maintenance | A `background` module owns the loop and calls each job |

- **Done when**: the allow-list of package 3.3 no longer contains these entries.

### 3.3 Check the layering in CI (S to add, then ongoing)

- **Change**: new `scripts/check/layers.mjs`. It reads `use` lines in `src-tauri/src/` and
  `import` lines in `webview/pages/app/`, maps each file to its folder's layer (review 4.2), and
  fails on an import from a lower layer to a higher one, or on a cycle between files. Known
  violations live in `scripts/check/layers-allow.txt`; the script also fails when an allowed
  violation no longer exists, so the list can only shrink.
- **Add it early**: it can be written right after 2.2 and 2.3, with every current violation
  allowed. Packages 3.1 and 3.2 then empty the list.
- **Done when**: the allow-list is empty and the script is a required CI step.

### 3.4 Visibility (M, one pull request per folder)

- **Depends on**: 2.2.
- **Change**: in each `chat/<folder>/`, turn on `#![warn(unreachable_pub)]`, change what the
  compiler reports to `pub(super)` or `pub(crate)`, and reduce the folder's `mod.rs` to the
  names other folders use. In `chat/commands/`, replace `use super::*;` with named imports, and
  the glob re-exports in `commands.rs:15-38` with the generated list (package 3.9).
- **Done when**: `chat.rs` no longer needs the blanket re-exports added in 2.2.

### 3.5 Typed errors (L, one pull request per layer)

- **Change**:
  1. `support/error.rs`: `StoreError { NotFound, Invalid, Io, Locked, Corrupt }` with
     `thiserror`. Convert `support` and `chat/storage/` first.
  2. Then one enum per folder as needed (`knowledge`, `toolkit`, `models`), each with a
     `From<StoreError>`.
  3. The command layer keeps returning `Result<_, String>` to the interface, through one
     `impl From<Error> for String` that chooses the user-facing wording.
- **Do not** convert all 692 signatures at once. A function that only forwards an error can
  keep `String` until its folder's turn.
- **Done when**: no `Result<_, String>` below `commands/`, `cli/` and `integrations/`.

### 3.6 Closed sets as enums and constants (M)

- **Change**: enums for run, task and terminal statuses (`subagents.rs`, `tasks.rs`,
  `terminal.rs`), version kinds (`versions.rs:201-265`) and trace artifact kinds, each with
  `serde(rename_all)` chosen so the JSON on disk and on the wire is unchanged.
  `ChatState::record` (`chat.rs:198-213`) matches on `ChatEvent`, not its JSON. One
  `chat/events.rs` holds every event channel name; one `support/env.rs` holds the 17 `CORTEX_*`
  variable names with a line of documentation each.
- **Watch out**: add a test per enum that the serialized form equals the old literal.
- **Done when**: the status literals appear only in `serde` attributes and tests.

### 3.7 Globals into state (M)

- **Change**, in the order of review 7.2: the local-model globals become one `LocalModels`
  struct held by `ChatState`; `engine::MESSAGE_DEPTH` and the session allowances become fields
  of the turn's state; `tools/approvals.rs:17-19` moves into the `ApprovalGate` implementation;
  `keychain/pending_deletes.rs` gets an `init` that returns a handle the keychain needs, so the
  ordering at `app.rs:509-512` is enforced by the types; the test-only global at
  `sandbox.rs:167-190` becomes a parameter.
- **Done when**: the count of mutable statics (section 7) is under 10, each with a comment
  saying why it must be process-wide.

### 3.8 Hosted mode behind a feature (L)

- **Depends on**: D1, 0.2.
- **Change**:
  1. `Cargo.toml`: features `hosted` and `e2e`. `lib.rs:57` becomes
     `cfg!(feature = "hosted")`; `e2e_testing_enabled()` becomes `cfg!(feature = "e2e")`.
  2. Move `schedules/`, `schedules.rs`, `skills.rs`, `browser_login.rs` and the hosted parts of
     `webview.rs` under `src/hosted/`, compiled only with the feature. The native scheduler
     keeps using `Timing`, which moves to `support`.
  3. Their commands leave the default handler list and permission files; with the feature they
     are added back. Package 0.2's test runs for both feature sets.
  4. Move `webview/bridges/`, `pages/settings/`, `pages/login.ts`, `types/`, `settings.html`,
     `login.html` and their styles under `webview/hosted/`; `scripts/build-webview.mjs` builds
     them only when asked (`--hosted`), and `main_window.rs:52-65` embeds them only with the
     feature.
  5. The `echo` connector and the in-memory secret stores move behind `test-utils`.
  6. CI: the existing jobs build the default; one extra job builds and tests
     `--features hosted,e2e`. `e2e/` builds with those features.
- **If D1 is "remove"**: after step 6 is green, delete `src/hosted/`, `webview/hosted/`, `e2e/`,
  the three test files and the two features.
- **Done when**: the default build contains no `schedule_*` or `skill_*` command and starts one
  scheduler.

### 3.9 One description per command (L)

- **Depends on**: 0.2, 0.3, 2.2.
- **Change**:
  1. `scripts/generate/commands.mjs` scans for `#[tauri::command]` functions and writes
     `src/app/handlers.rs` (the `generate_handler!` list) and the permission files, one
     permission per command file. Both are committed; CI regenerates and fails on a difference.
     Adding a command becomes: write the function, run the script.
  2. Thin commands: move the logic listed in review section 8, item 6, into its domain module,
     with tests there.
  3. Redistribute `misc_commands.rs` and the strays in `conversations_commands.rs` to the area
     each belongs to.
  4. Names, done together with package 3.10 so the compiler finds every caller: `save` for
     writing a whole value, `set` for one field, `delete` for removal, `list` for collections.
- **Done when**: no hand-edited handler list or chat permission file remains.

### 3.10 Typed calls from the interface (L)

- **Step 1, a two-day trial**: generate TypeScript types from the Rust types of one command file
  (`providers_commands.rs`, 15 commands). Compare a type generator used alone against one that
  also generates the call wrappers. Check each against the pinned `tauri` version and against
  the hand-written types in `api.ts` (mixed `snake_case` and `camelCase`: the generated output
  shows which side is wrong). Write down the choice in `docs/architecture.md`.
- **Step 2**: generate `webview/pages/app/core/api.gen.ts`, committed and drift-checked like the
  catalog. Split the hand-written `api.ts` by area on top of it.
- **Step 3**: migrate the 287 raw `tauriInvoke` calls, one interface folder per pull request
  (`settings/` has 95, `chat/` 45, `pages/` about 80). Delete each file's local copies of
  backend types as it migrates; `ConnectorManifest`, `Doc` and `Library` lose their duplicates.
- **Step 4**: a lint rule that forbids importing `tauriInvoke` outside `core/` and
  `webview/hosted/`.
- **Done when**: `grep -rn "tauriInvoke" webview/pages/app` finds only `core/`.

### 3.11 One model for things that come in threes (M each)

- **Chat platforms**: a `ChatPlatform` trait (connect, receive, send, describe) implemented by
  Slack, Discord and Telegram; one supervisor, one pairing flow, one settings shape. The three
  `RUNNING` statics go. Add tests against a stand-in server, since each has one or two today.
- **Connector descriptors**: exposure class, icon and log-redaction rule become methods on
  `Connector` with defaults; the tables in `exposure.rs:123-170`, `icons.rs:13-31` and
  `commands.rs:92-155` and the special cases in `registry.rs:300,656,706,714` read from them.
  Adding a connector then touches its own file and the registration site.
- **Built-in tools**: after 2.5, each built-in is one table entry holding name, schema, handler
  and a `Deserialize` struct for its arguments; `NAMES` and the `match` go, and the 76
  hand-parsed argument reads become struct fields.

### Phase 4: crates

### 4.1 Extract `cortex-store` and `cortex-tools` (L)

- **Depends on**: 1.1, 3.1, 3.2, 3.5.
- **Change**: `support/` (settings files, encryption at rest, names, errors) and
  `chat/storage/` become `crates/cortex-store`. Then `tools/` becomes `crates/cortex-tools`
  once its loaders take a folder path, not the app handle (`tools/connector_state.rs:45-79` is
  the typical case), and its three window-facing files (`commands.rs`, `icons.rs`, the approval
  gate's implementation) stay in the app.
- **Why last**: a crate boundary is the layering rule enforced by the compiler. It is cheap
  once the cycles are gone and very expensive before.
- **Done when**: `cargo test -p cortex-store -p cortex-tools` runs without WebKitGTK installed,
  as the kernel crate's does.

## 7. How to tell it is working

Measure at the start, and again after each phase. Each line is one command from the repository
root.

| Measure | Today | Target | After |
|---|---|---|---|
| Tracked files that are ignored: `git ls-files -ci --exclude-standard \| wc -l` | 12 | 0 | 0.1 |
| Interface tests failing: `npm test` | 5 of 9, and not run | 0, run in CI | 0.5 |
| Commands in the largest permission | 143 | Under 50, description true | 0.3 |
| Hand-written settings loaders: `grep -rn "and_then(.*from_str" src-tauri/src \| grep -c unwrap_or_default` | About 28 | 0 | 1.1 |
| HTTP clients built in place: `grep -rn "reqwest::Client::\(builder\|new\)" src-tauri/src \| wc -l` | 36 | 1 | 1.2 |
| Raw error toasts: `grep -rn "toast(String(e)" webview \| wc -l` | 148 | 0 | 1.3 |
| Files directly in `src-tauri/src/chat/` | 86 | Under 15 | 2.2 |
| Files directly in `webview/pages/app/` | 51 | Under 5 | 2.3 |
| Largest interface file | 4,892 lines | Under 800 | 2.4 |
| Rust functions over 150 lines | 11 | 0 | 2.5 |
| Rust files over 800 lines before their test module | 16 | 0 | 2.6 |
| Layering violations allowed: `wc -l scripts/check/layers-allow.txt` | Not measured (about 69 modules in one cycle) | 0 | 3.3 |
| Public functions in chat: `grep -rnE "^\s*pub (async )?fn" src-tauri/src/chat \| wc -l` | About 1,060 | Under 400 | 3.4 |
| `Result<_, String>` in chat, outside `commands/` | About 420 | 0 below the entry points | 3.5 |
| Mutable process-wide statics: `python3 scripts/check-statics.py --list` | About 55 | Under 10 | 3.7 |
| Hosted-only commands in the default build (`schedule_*`, `skill_*`, sign-in) | At least 10 | 0 | 3.8 |
| Hand edits to add a command | 4 to 5 | 1, plus a script | 3.9 |
| Backend calls by string name: `grep -rn "tauriInvoke[<(]" webview/pages/app \| grep -vc "core/"` | 287 | 0 | 3.10 |

## 8. Manual checklist for interface changes

Run with `npm run dev` after `npm run build:webview`, in the light and dark themes. About ten
minutes.

1. Start: the app opens to the home view; the sidebar lists chats; no error in the log.
2. Chat: send a message and watch it stream; stop one mid-answer; regenerate; edit a prompt.
3. Composer: attach a file; open the `/`, `@`, `#` and `:` menus; switch mode with Shift+Tab;
   queue a message with Alt+Enter while an answer runs.
4. Tools: an answer that runs code shows its activity; an approval prompt appears and both
   answers work.
5. Side panel: open a canvas, an app and the Sources panel; switch tabs; split.
6. Settings: open each of the six sections; change and revert one setting in each; add and
   remove a provider.
7. Pages: Agents, Tasks, Libraries, Skills, Knowledge and Connectors each open and list.
8. Search: Ctrl+K finds a chat and runs a command.
9. Language: switch the interface to another language and back; no English left on the pages
   visited above.
10. Other windows: Quick Ask and dictation open.

Package 2.4 is the natural point to turn the first five into an automated smoke test on the
WebDriver setup `e2e/` already has.

## 9. Ordering constraints

Only these are real; the rest is recommendation.

| Do this | Before | Because |
|---|---|---|
| 0.2 Command surface test | 0.3, 3.8, 3.9 | A command dropped from a permission fails only at run time |
| D1 | 0.4, 3.8 | Decides whether hosted code is fixed or deleted |
| 1.4 Catalog as JSON | 1.5 | Rust reads the catalog's source text; a formatter would change it |
| 1.5 Formatter | 2.3, 2.4 | Moves and splits should not be mixed with reformatting |
| Recursive scan in `ui-strings.mjs` | 2.3 | Otherwise moved files silently lose their translations |
| 2.1 Absolute paths | 2.2 | `super::` paths break when a file moves down a level |
| 2.2, 2.3 Folders | 3.3, 3.4 | The layer of a file is its folder |
| 3.1 Leaf modules | 3.2 | The inversions need somewhere below both sides |
| 2.5 Built-ins split | 3.11 (built-in table) | One function per tool is the table's entry |
| 3.9 Generated handlers | 3.10 names | Renames need both sides generated |
| 1.1, 3.1, 3.2, 3.5 | 4.1 | A crate cannot depend back on the app |

Three tracks can run in parallel once phase 0 is done, since they touch different files: the
Rust helpers and structure (1.1, 1.2, 2.1, 2.2, 2.5, 2.6), the interface (1.3 to 1.6, 2.3, 2.4,
2.7), and the command boundary (3.9, 3.10).

## 10. Risks

| Risk | Where | Mitigation |
|---|---|---|
| Long-lived branches conflict with the moves | 2.2, 2.3, 1.5 | Announce a quiet period; do each folder in a day; rebase other branches right after |
| A moved or renamed file changes what users have on disk | 1.1, 1.3, 3.6 | Rule 3 in section 2; a test per enum and per settings file that the path and JSON are unchanged |
| Translations silently lost | 1.5, 2.3, 2.4 | `catalog.json` must be byte-identical; the recursive scan comes first |
| Interface regressions with no automated tests | 2.3, 2.4, 2.7 | The checklist; unit tests for each extracted piece of logic; small pull requests |
| A command becomes unreachable | 0.3, 3.8, 3.9 | Package 0.2's test, run for each feature set |
| Typed errors change the text users see | 3.5 | One conversion at the command boundary; compare messages for the common failures before and after |
| The plan stalls half-way through a phase | All | Every package leaves the tree working and is worth having alone; the re-exports added in 2.2 and 2.4 mean a half-done grouping still builds |
| Generated-code tools do not fit the pinned dependency versions | 3.10 | The two-day trial decides; hand-written wrappers over generated types are the fallback |

## 11. Not in this plan

- New features, and changes to what the app does, beyond the fixes in package 0.8.
- The updater endpoint, publisher and signing identity (decision D2).
- Rewriting the interface in a framework. The build script's note about a React rewrite of the
  old settings page concerns hosted mode only.
- Splitting the remaining 3% of long functions under the 150-line threshold.
- The browser extension, which is 141 lines and fine as it is.
