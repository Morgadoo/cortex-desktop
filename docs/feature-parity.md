# Feature parity: Mistral Vibe and the inspiration apps

This fork reimplements Mistral Vibe's desktop app as an open, multi-provider chat built on
[rig](https://rig.rs). This file is the feature-by-feature comparison behind that work. Each row
says what the reference does, what this app does, and where the difference is.

Status: **Done** (matches), **Adapted** (same job, done differently on purpose), **Partial**,
**Missing**, or **N/A** (does not apply to a local, single-user app).

## Mistral Vibe, screen by screen

References: the Vibe screenshots from the source conversation (rows 1–6 checked side by side
against the 5 that survived), and Mistral's documentation: docs.mistral.ai/vibe,
/vibe/work/web/skills and /vibe/choose-chat-work-code. Rows marked "(docs)" come from it.

### 1. Sidebar

| Vibe | Here | Status |
|---|---|---|
| Sidebar toggle, back / forward, search | Same; search is Ctrl+K over conversations | Done |
| Hamburger app menu (top left) | The OS menu bar and tray carry these actions | Adapted |
| Work / Code switch | Work / Code / **Agents**: a third mode for the agent workforce (openbot, grok-bot) | Done + extra |
| New chat, Apps (Preview), Context, Scheduled | Same, with Context opening Skills, Agents, Knowledge, Connectors, Libraries, Workflows, Instructions | Done |
| Projects, Conversations | Same; projects have an icon, tone, files and shared-chat context | Done |
| "Upgrade to Pro" card | None: no subscription | N/A |
| Profile and workspace at the bottom | Profile menu with theme, settings, export | Done |

### 2. Home

| Vibe | Here | Status |
|---|---|---|
| Logo and a greeting with the user's name | Same, using the name from Profile | Done |
| "Work mode is better with your apps connected · Connect ×" banner | Same, dismissible, rotating app badge | Done |
| Composer: "Type / for quick access", +, approval mode, Fast ▾, mic | Same; the mic takes the send button's place while the input is empty | Done |
| Model picker | Vibe has none (Mistral only). Here it is any rig provider's model, plus "Compare models…" | Adapted |
| Suggestions: set up and discover, state of AI, connect apps, "Discover more" | Same; "Discover more" reveals five more | Done |
| Dashed bubble for a temporary chat (top right) | Same | Done |

### 3. The + menu

| Vibe | Here | Status |
|---|---|---|
| Upload files | Upload files (documents and images) | Done |
| Folders › | Folders the assistant can read; pick one for this message, or add one | Done |
| Skills › (search, New skill, All / Personal / Shared with me, built-in list) | Search, New skill, All / Mine / Built-in | Done; "Shared with me" is N/A without accounts |
| Knowledge › | Knowledge on/off, count, open Knowledge | Done |
| Connectors › | Each ready connector switched on or off for this chat | Done |
| Projects › | Start in or move to a project | Done |
| Libraries › | Search a library for this message | Done |
| Workflows › | Step-by-step workflows (4 built in, editable), run as a tracked plan | Done |
| Reset input | Same | Done |
| (none) | Canvas | Extra |

### 4. Built-in skills (Skills flyout)

| Vibe | Here | Status |
|---|---|---|
| Set up my Knowledge | `set-up-my-knowledge` | Done |
| Discover Knowledge | `discover-knowledge` | Done |
| Sanitize my Knowledge | `sanitize-my-knowledge` | Done |
| Build my Knowledge from Connectors | `build-knowledge-from-connectors` | Done |
| Create a Skill from my Knowledge | `create-skill-from-knowledge`, through the `save_skill` tool | Done |
| Built-ins documented at docs.mistral.ai/vibe/work/web/skills: challenge-my-thinking, data-analysis, deep-research, doc-coauthoring, document-review, internal-comms, meeting-prep, research-synthesis, skill-creator, stakeholder-translator, structured-extraction, vibe-work-onboarding | All twelve (onboarding is `getting-started`) | Done |
| Skills toggled individually, built-ins included | Same, on the Skills page | Done |
| (none) | write-email, summarize-document | Extra |

### 5–6. Approval mode and speed (also Mistral's docs)

| Vibe | Here | Status |
|---|---|---|
| Ask: ask before every change to your files | Same | Done |
| Accept edits: approve file edits automatically | Same | Done |
| Fast: quick answers | Same | Done |
| Think: complex tasks | Same; reasoning effort through rig, refused gracefully by models that cannot reason | Done |
| Code approval modes Ask / Plan / Accept edits / Full auto (Vibe Code) | All four; Plan refuses changes and asks for a plan, Full auto asks nothing | Done |
| Steer or stop a running task (docs.mistral.ai/vibe) | Typing while a task runs sends guidance that joins at its next step; Stop still stops | Done |

### 7. Context pages

| Vibe | Here | Status |
|---|---|---|
| Skills page (mine / built-in, search, import SKILL.md) | Same | Done |
| Connectors page with a catalog | Same; MCP catalog with OAuth dynamic registration | Done |
| Knowledge (memories) | Same; the model saves facts, the user edits them | Done |
| Agentic Knowledge Base (release notes, Sept 2026): editable pages, recalled only when relevant, cited | Knowledge pages: written by the model (`knowledge_write`) or you, BM25 recall of the relevant ones into the prompt, others by title, `knowledge_read` / `knowledge_search`, [Knowledge: page] citation pills | Done |
| Libraries | Local BM25 retrieval over documents, cited by document | Done |
| Instructions modal with live preview | Same | Done |
| Scheduled tasks: once, daily, weekly, monthly, yearly (docs) | All five, plus unattended / autonomous approval for each task | Done |
| Temporary Chat kept out of history, search, memory and training (docs) | Out of history and search, deleted on restart, and the memory tools are withheld | Done |

### 8. Conversation

| Vibe | Here | Status |
|---|---|---|
| Breadcrumb header (project / title ▾) | Same, plus a usage and cost meter | Done |
| `/context` view (Vibe 3.1.0) | Clicking the usage meter shows what fills the window: instructions and tools, messages, answers, tool calls and results, thinking, images, summary | Done |
| `/branch` / `/fork` (Vibe 2.25.3, 3.0.4) | "Branch into a new chat" in the conversation menu | Done |
| "Jump to bottom" button (Vibe 3.1.0) | Same | Done |
| Composer placeholder "Type / for quick access" | Same | Done |
| Thinking, collapsed when the answer arrives | Same | Done |
| Activity timeline; "Running N actions" while working | Same; repeated tools merge into one table | Done |
| Search results card | Same | Done |
| ask_user question picker | Same | Done |
| Citations inline, Sources chip | Same | Done |
| Actions row: rate, regenerate, copy, time | Same, plus edit-and-resend on your messages | Done |
| Skill chip on a deep-research request | Skill, workflow, library, folder and canvas requests show as chips in your message; the plan panel follows the steps | Done |
| Canvas card (edit, copy, download, expand) | Same, with versions and "Ask about this" on selected text | Done |

### 9. Code mode

| Vibe | Here | Status |
|---|---|---|
| A session bound to a folder | Same | Done |
| File and terminal tools with approvals | Same; Accept edits lets file edits through | Done |
| Knows the repository | Branch, uncommitted files, recent commits, and `AGENTS.md` / `CLAUDE.md` in the prompt | Done |
| Review changes | Changes view: files with +/− and a coloured diff; edits appear as diffs in the timeline | Done |
| Commit and open pull requests (docs) | Commit, then Pull request: branches off the default branch, pushes, and opens the PR with `gh` or GitHub's compare page | Done |
| Integrated terminal and background commands (Vibe Desktop, release notes Nov 2026) | Terminal panel in coding sessions: run long commands (dev servers, watchers) in the folder, live output, stop; the model starts, reads and stops them (`terminal_*`), starting asks for approval; stopping ends the whole process group; all stop when the app quits | Done |

### 10. Apps

| Vibe | Here | Status |
|---|---|---|
| Apps (Preview) | Mini apps the model builds, run in a sandboxed frame with their own CSP; Preview / Code; Apps page | Done |
| App canvas opens beside the chat (Vibe screenshots, Nov 2026): the reply keeps a compact "title · App" card with an accent border; the side panel has close, code / preview, title, copy, full screen, download; inline, the preview is clipped with an Expand bar | Same layout; Download saves the app as one HTML file; Escape closes the panel | Done |
| Spreadsheets (release notes, Sept 2026): analyse, clean and create Excel/CSV, edit in Canvas, download .xlsx | Sheet canvases (CSV) edited as a grid with rows/columns, saved as versions; download as .xlsx (rust_xlsxwriter) or CSV; uploaded .xlsx/.xls/.ods read into CSV (calamine); heavy analysis via run_code | Done |

### 11. Settings and appearance

| Vibe | Here | Status |
|---|---|---|
| Settings dialog (profile, permissions, general, appearance) | Same, plus Providers & models | Done |
| Light / dark / system | Same | Done |
| (none) | Accent colour (presets + custom) and background palettes: the graphite default, plus T3 Code's six | Extra |

### 12. Vibe changelog, September–November 2026 (versions 2.25–3.1.2)

Checked item by item. Many are for the Vibe CLI in a terminal; where one has a desktop
equivalent, that is what is compared.

| Vibe | Here | Status |
|---|---|---|
| Thinking shown as a collapsible "Thought" row (3.0.2), "Thought for 1s" (screenshots) | Same; the duration is stored with the reply, so it survives a restart | Done |
| Tool calls fold into a compact summary line (2.25.1) | Same; repeated tools merge into a table | Done |
| Ctrl+O toggles all tool groups (2.25.1) | Same | Done |
| Ctrl+R searches prompt history (3.1.0) | Same; Up / Down in an empty composer walk earlier prompts | Done |
| "Jump to bottom" button (3.1.0) | Same | Done |
| `/context` (3.1.0) | Click the usage meter | Done |
| `/branch`, `/fork` (2.25.3, 3.0.4) | "Branch into a new chat" | Done |
| `/skills` with search, on/off, preview (3.1.2, 2.25.5) | Skills page: search, toggles, the skill's text | Done |
| Skills invoked with `@` (Vibe Work docs) | `@` or `/` at the start of the composer | Done |
| Project skills override global ones (3.1.2) | Your skills override built-ins of the same name | Done |
| Messages sent during a wait reach the agent at once (3.1.2) | Steering: typing while a task runs joins it at its next step | Done |
| `/loop`: recurring prompts in natural language (2.26.0) | Ask for it in chat (`schedule_task`), or Scheduled | Done |
| `/voice`, narrator reads turns aloud, Esc stops it (2.26.1) | Settings → Audio, read aloud and auto-play; the speaker button stops it | Done |
| Side panel of background processes with Stop (2.26.0) | Terminal panel in coding sessions | Done |
| Undo and redo in the composer (2.26.0) | The system's undo and redo in the text box | Done |
| Unseen indicators for sessions needing attention (2.25.8) | Unread dots on chats and agents | Done |
| Archived sessions hidden from active lists (2.25.8) | Archive | Done |
| Pinned todo plan (2.25.8) | Plan panel beside the conversation | Done |
| Subagent history with live status (2.25.5) | Sub-agent rows with live status and reports; "View every step" opens a worker's transcript (each tool call, its input and result, then the report), kept apart from the main context | Done |
| Errors and warnings inline, not pop-ups (3.1.0) | Errors are cards in the conversation; notices (like a backup model taking over) are pop-ups | Partial |
| Large tool outputs saved to a file, start and end shown to the model (3.0.1) | Results over 24,000 characters are cut, older ones cleared; `tool_results__read` reads them back | Adapted |
| Footer: active model and thinking level; click to change (3.0.3) | Model and Fast / Think in the composer | Done |
| Monthly usage gauge (3.1.0) | This month's tokens, cost and chats in the profile menu; also per conversation and per agent | Done |
| Session PRs shown with links (3.0.3) | Changes view opens the pull request | Done |
| Smart-approve: risky calls escalate to a prompt and say why (2.25.4) | Commands that are hard to undo (recursive deletes, sudo, force-push, piping a download into a shell, dropping tables…) and deleting tools always ask, in every mode, with the reason; scheduled runs refuse them. A fixed list, not a classifier | Adapted |
| Sticky prompt pinned above the section being read (3.1.1) | Same; click it to jump back to the prompt | Done |
| `/release-notes` (3.0.4) | "What's new" in the profile menu | Done |
| Vim keybindings, mouse capture, cursor blink (3.0.2, 3.1.1) | Terminal-editor settings | N/A |
| `/teleport` to Vibe Code Web (2.26.0) | No cloud sessions | N/A |
| Worktrees, plugins, hooks, `vibe config schema`, headless `-p` limits | CLI and coding-agent configuration | N/A |
| Mobile apps, admin rollout, workspaces | Single-user desktop app | N/A |

### 13. From Mistral's docs, page by page (24 pages, about 300 items checked)

The screens without screenshots were checked against Mistral's own pages for them (Knowledge,
Scheduled tasks, Safety and approvals, Files and Canvas, Mini apps, Custom instructions,
Projects, Connectors and MCP connectors, Libraries, Workflows, Spreadsheets, Voice mode, Web
search, Image generation, Code interpreter, Work Desktop, and the Code Desktop pages). The
items most likely to differ:

| Vibe (docs) | Here | Status |
|---|---|---|
| Approval prompt: `Continue`, `Always allow`, `Decline` | Same labels | Done |
| Stop interrupts the task (black square) | Same | Done |
| Pinned chats in their own **Pinned** section | Same | Done |
| Command palette (Cmd+K) with groups: Chats, Modes, Context, Settings, View, Actions… | Ctrl+K: Actions (new chat, modes, context pages, settings, sidebar, archive, recently deleted, what's new) and Chats, with content search | Done |
| Shortcuts: new session, sidebar, settings, Work / Code, Cmd+1–9 to sessions | Ctrl+N, Ctrl+B, Ctrl+,, Ctrl+Shift+1 / 2 / 3 (Work / Code / Agents), Ctrl+1–9 to the chats in the sidebar | Done |
| Voice: Autosend on/off; cancel discards a recording | Send after dictation; a discard button while recording | Done |
| Scheduled tasks: Once / Daily / Weekly (several days) / Monthly / Yearly, plain-words confirmation ("Every Monday at 9:00"), pause and resume, unread run in the sidebar | Same | Done |
| Knowledge: inline read / write pills; edit by asking; a "Sensitive data" toggle | Citation pills for pages read; saves show as steps; facts and pages editable directly; personal-data filter in Settings | Adapted |
| Connector detail: Interactive / Read-only functions, each with `Always allow`, `Refresh tools` | Connector page lists tools; approvals per call with Always allow | Partial |
| Canvas: versions with arrows, change view, restore; `Ask Work` on a selection | History: ‹ › through versions (who and when), a Changes view (added and struck-through lines), Restore as a new version; "Ask about this" on a selection; line breaks kept | Done |
| Canvas preview of HTML, React, Mermaid, Marp; several canvases as tabs | Apps (HTML, JS) in the side panel; Markdown canvases. Not built: Mermaid / Marp preview (about 3 MB more in the binary, and the page's CSP forbids the inline styles Mermaid needs), tabs | Partial |
| Mini apps: file list in code view, ZIP download | Single-file apps, downloaded as one HTML file | Adapted |
| Public share link for apps and spreadsheets | Nothing leaves the device | N/A |
| Spreadsheets: sheet tabs, `Open in Google Sheets` | One sheet per canvas; .xlsx / CSV download | Partial |
| Libraries: numbered footnotes jumping to the passage, roles, org sharing | Citations by document; single user | Partial |
| Workflows: tag in the input and above the sent message, `Workflow started: {name}` | Chip in the input and the message; the plan panel follows the steps | Done |
| Custom MCP connector: name, Server URL, description; auth detected (none, Bearer/Basic, OAuth 2.1 with dynamic registration) | Same | Done |
| Work Desktop: `+` → Folders → Add a folder, Remove all | Folders in the + menu | Done |
| Code Desktop right panel: Files, Git Changes, Terminal, Debug, Background processes; diff Unified / Split | Files (filter, read with line numbers), Changes with Unified / Split, Terminal with background processes; no Debug panel | Partial |
| Integrated browser with "Open in external browser?" | Links open in the system browser | Adapted |

## The inspiration apps

| Feature | Seen in | Here | Status |
|---|---|---|---|
| Any provider, local models | open-webui, LibreChat, lobehub, osaurus | 15 rig providers, Ollama, llama.cpp, custom base URLs | Done |
| Side-by-side model comparison | open-webui, LibreChat | Compare 2–3 models, keep the best answer | Done |
| Code interpreter | open-webui, LibreChat | `run_code`: Python, JS, Bash in a sandboxed workspace (bubblewrap) | Done |
| Artifacts / runnable apps | LibreChat, lobehub | Mini apps and canvases | Done |
| Image generation | open-webui, LibreChat, lobehub | `generate_image` via rig (OpenAI, xAI, Together) | Done |
| Voice input | open-webui, lobehub | Dictation via rig transcription (OpenAI, Groq) | Done |
| Read aloud (TTS) | open-webui, lobehub | Speaker under each answer, via rig speech models (xAI, OpenAI); verified live | Done |
| Audio provider setup | open-webui (Admin → Audio) | Settings → Audio, as Open WebUI sets it up. Speech to text: chat providers, any OpenAI-compatible server (base URL, key, model, language) or ElevenLabs. Text to speech: chat providers, OpenAI-compatible (Kokoro, LocalAI, Speaches…), xAI, ElevenLabs (account voices listed) or the system voice. Each has its voice, speed and keychain-stored key. Auto-send after dictation, auto-play of answers. OpenAI-compatible TTS and auto-play verified live | Done |
| RAG over documents | open-webui, onyx | Libraries and project files, cited | Done |
| Web search with citations | open-webui, onyx, LibreChat | Keyless web search, citation pills, Sources | Done |
| Deep research | onyx | deep-research skill with parallel sub-agents | Done |
| MCP tools | LibreChat, lobehub, osaurus | MCP connectors and catalog | Done |
| Agents / presets | LibreChat, lobehub, openbot | Agents with persona, model, tools, skills and schedules | Done |
| Autonomous scheduled agents | openbot, grok-bot | Scheduled tasks with unattended / autonomous runs | Done |
| Create an agent from one prompt | openbot.run | "New agent" asks for a description; the default model drafts name, avatar, description, instructions, tools, skills and thinking, reviewed before saving | Done |
| Persistent agent workspace that learns from work | openbot.run, open.bot | Each agent keeps a notebook (`update_notebook`), read at the start of every conversation and run, editable on its page | Done |
| Agents hand tasks to each other | openbot.run, open.bot (orchestrator → specialists) | `delegate` tasks can name a teammate; it works with its own instructions, model and allowed tools, and reports back. The assistant's prompt lists the team | Done |
| Failover to a backup model | openbot.run | Per-agent backup model, used when the main one fails before anything streamed, with a notice | Done |
| Cost per bot and usage limits | open.bot | Spend per agent this month (conversations, tokens, dollars) on its card and page; optional monthly budget that stops further turns | Done |
| Approval cards in the conversation (Approve / Review / Deny) | open.bot | Approval requests appear above the composer with the call's details: allow once, always, or deny | Adapted |
| Tool policies per bot: allowed, ask, blocked | open.bot | Per agent, each tool is Allowed, Ask first (asks even in Full auto; refused in unattended runs) or Blocked | Done |
| Channels: Slack, Teams, phone | openbot.run, open.bot | Local desktop app with no server to receive messages | N/A |
| Multiplayer channels, SSO, SCIM, tenants, admin plane | open.bot | Single-user app | N/A |
| Audit trail of what bots did | open.bot | Every tool step is kept in its conversation; each agent's page lists its runs | Adapted |
| Memory | open-webui, LibreChat | Knowledge | Done |
| Prompt library | open-webui, LibreChat | Skills (`/name`) and Workflows | Done |
| Response versions and branching | LibreChat, lobehub | A branch tree: regenerate and edit branch at any message; ‹ 1 / n › switches; branch points inside a branch come back with it | Done |
| Conversation import | LibreChat, open-webui | ChatGPT and Claude exports (zip or conversations.json), visible branch only | Done |
| Archive / folders / tags | open-webui | Archive, pin, projects as folders, and tags searchable with `#tag` | Done |
| Export | all | Each conversation as Markdown; all data | Done |
| Personal-data filter | (none) | Optional redaction before cloud models | Extra |
| Context management | (best practice) | Tool-result clearing, compaction at 60%, measured tokens and cost | Done |
| Design system | T3 Code | Tokens, shared components, T3's control styling and palettes | Done |

## Built for years of use

| Need | Here |
|---|---|
| A long history stays fast | A conversation index (`chat/index/`) keeps each sidebar row with the file's size and time; listing parses only files that changed. Rebuilt automatically if missing |
| Find anything ever said | Search (Ctrl+K) covers titles and every message, all words must match, with highlighted snippets; `#tag` still filters tags |
| The list stays short | The sidebar shows the newest 30 chats, plus pinned and open ones, then "Show more"; search reaches all |
| It adapts to you | Skills you use most come first in the `/` menu; earlier prompts are a keystroke away (Up, Ctrl+R) |
| Mistakes are recoverable | Deleting moves a conversation to Recently deleted for 30 days (Settings → Profile), with Restore |
| Files are protected | The first change of each day keeps a copy of the conversation as it was; state files are copied daily; 30 days kept |
| Damage is visible | A file that cannot be read is reported in Settings with "Restore the backup"; the damaged file is kept in `quarantine/` |
| Formats can evolve | Each conversation records its format version (`schema`), so migrations know what they read |

## Known gaps

- **Reference coverage:** sections 1–6 were checked against the first 5 Vibe screenshots, and the
  app canvas flow (thinking, card, side panel) against 3 fresh ones from November 2026. Screens
  without a screenshot (Context pages, Scheduled, Settings, Code mode) follow Mistral's docs.
- **Voice input against a real provider:** dictation was verified end to end in the app with a
  virtual microphone and a mock transcription endpoint. The test proxy has no transcription
  endpoint (404), so it has not run against a real speech-to-text model yet. The same applies to
  the OpenAI-compatible and ElevenLabs speech-to-text engines and to ElevenLabs voices, which
  need keys this setup does not have.
- **Pull requests to non-GitHub remotes:** the branch is pushed, and the pull request is opened on that host by hand.
- **Branding:** the app still carries the Lumo name and logo; a new name is the owner's decision.
