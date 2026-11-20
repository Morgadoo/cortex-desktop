# Contributing to Cortex Desktop

Thanks for helping. Cortex is GPL-3.0 and meant to stay a personal, local-first desktop app;
changes that keep the user in control of their data and their models are the most welcome.

Read [docs/architecture.md](docs/architecture.md) first: it maps the code, the layers and where
data lives, and gives the steps for adding a command, a tool, a connector or a setting.

## Setting up

You need Node.js 22, Rust (the version in `src-tauri/rust-toolchain.toml` installs itself through
rustup) and the [Tauri prerequisites](https://tauri.app/start/prerequisites/) for your platform.

```bash
npm install
npm run dev
git config core.hooksPath .githooks     # optional: type-check and test before each push
```

## Before you push

Run what CI runs:

```bash
npm run typecheck && npm run lint && npm run build:webview && npm run check:i18n && npm test
cd src-tauri
cargo fmt --all --check
cargo clippy --workspace --features cortex/test-utils --all-targets -- -D warnings
cargo test --workspace --features cortex/test-utils
cd ..
python3 scripts/check-layers.py
python3 scripts/generate-commands.py --check
```

If you touched the hosted web app's code (`webview/bridges/`, `src-tauri/src/schedules/`,
`skills.rs`, `pdf_export.rs`), also run clippy and the tests with
`--features cortex/test-utils,cortex/e2e`.

Interface changes have few automated tests. Open the app and try what you changed, in the light
and dark themes; the checklist in section 8 of
[the improvement plan](docs/codebase-improvements-plan.md) covers the main flows.

## How the code is written

These keep the code base easy to change; reviews hold changes to them.

1. **A module lives in the folder for its role** and imports only from its own layer or below
   (`scripts/check-layers.py`). If you need something from above, the code is in the wrong place.
2. **Small units.** A function over about 100 lines, or a file over about 800 lines of code
   before its tests, needs a reason. Tests over about 300 lines go in a sibling `tests.rs`.
3. **Items are private** unless another folder uses them.
4. **One way to do each thing.** Settings files go through `support::settings_file`, HTTP
   clients through `support::http`, the local keychain through `chat::secrets`, slugs through
   `support::names`; in the interface, preferences through `core/prefs.ts`, backend calls
   through `core/api.ts` or `core/call.ts`, formatting through `core/format.ts`. A second
   implementation is a review finding.
5. **A closed set of values is an enum** (on both sides of IPC), not string literals.
6. **No new process-wide mutable `static`.** Put state in a struct created at startup and passed
   down, or in Tauri's managed state.
7. **Commands are thin**: parse, call one function below, convert the result. Logic in a command
   is logic without tests.
8. **Model output reaches the page only through `ui/markdown.ts`**, which sanitises it; `h()`
   inserts strings as text. `npm run lint` rejects `innerHTML` anywhere else.
9. **Data formats are a promise.** File names under the config folder, JSON field names,
   keychain names and stored preference keys are what users already have on disk: change them
   only with a migration.
10. **Comments say why**, not what. Every file opens with a `//!` (Rust) or `/** */` (TypeScript)
    comment saying what it is for.

## Commits and pull requests

- One kind of change per commit: a move has no edits beyond paths, a reformat has nothing else.
- Write the commit message as a sentence about what changes and why.
- Interface text is translated: new strings land in `src-tauri/i18n/catalog.json` via
  `npm run i18n:catalog`, and need translations (`scripts/translate_ui.py`) before the Rust test
  that checks every language passes.
- Keep pull requests focused; a large change goes in as a series that each leave the app working.

## Reporting problems

Open an issue with what you did, what you expected and what happened. Settings → General →
Error reports and updates → Export diagnostics saves the app's log, which usually says why.

## Releases

See [docs/releasing.md](docs/releasing.md): one-time signing setup, `node scripts/set-version.mjs`, a tag, and the release workflow does the rest.
