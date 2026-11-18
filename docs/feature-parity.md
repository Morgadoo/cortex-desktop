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
| Code mode (screenshots): "New Session", Extensions in the sidebar, "Send a message", "Working · 12s" / "Thinking · …" status, "Created nyan-snake.html" / "Edited …" steps | Same; Extensions opens the connectors (MCP) | Done |
| Code mode: "Found tools" / "Loaded tool details" steps (tools loaded on demand) | Same idea: past 30 connector tools, only the core ones (files, shell, browser, terminal) go with each request; the rest are named by connector, and `find_tools` loads the matches for the next step and the rest of the conversation. Shown as "Found tools" with what was loaded. Verified with a 35-tool MCP server: the model found and used the calendar tool | Done |
| Thinking, collapsed when the answer arrives | Same | Done |
| Activity timeline; "Running N actions" while working | Same; repeated tools merge into one table | Done |
| Search results card | Same | Done |
| ask_user question picker | Same | Done |
| Citations inline, Sources chip | Same | Done |
| Citations with each site's favicon; hover card with site, copy, preview image, title and description; Sources chip with overlapping favicons; Sources side panel (favicon, domain, title, snippet) (Vibe screenshots, Nov 2026) | Same. Favicons and previews are fetched by the app, cached in `chat/favicons/` and `chat/previews/`, and shown as `data:` images, since the page loads nothing from the web | Done |
| AFP article reader (Vibe's AFP partnership) | No news-agency feed; sources open in the browser | N/A |
| Actions row: rate, regenerate, copy, time | Same, plus edit-and-resend on your messages | Done |
| Skill chip on a deep-research request | Skill, workflow, library, folder and canvas requests show as chips in your message; the plan panel follows the steps | Done |
| Canvas card (edit, copy, download, expand) | Same, with versions, "Ask about this" on selected text, and a Preview / Code switch (the rendered document or its Markdown source); Markdown, Mermaid and Marp blocks in answers have the same switch | Done |

### 9. Code mode

| Vibe | Here | Status |
|---|---|---|
| A session bound to a folder | Same | Done |
| File and terminal tools with approvals | Same; Accept edits lets file edits through | Done |
| Knows the repository | Branch, uncommitted files, recent commits, and `AGENTS.md` / `CLAUDE.md` in the prompt | Done |
| Review changes | Changes view: files with +/− and a coloured diff; edits appear as diffs in the timeline | Done |
| Commit and open pull requests (docs) | Commit, then Pull request: branches off the default branch, pushes, and opens the PR with `gh` (or `glab` for GitLab), else the host's new-request page: GitHub, GitLab (self-hosted too), Bitbucket, Codeberg, Gitea and Forgejo, or the link any host prints during the push. Tested against a real push whose server prints a Gitea-style link | Done |
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
| Errors and warnings inline, not pop-ups (3.1.0) | Errors are cards in the conversation; notices (a backup model taking over) are lines in the reply, kept with it | Done |
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
| Mode menu as in Claude: Auto (the assistant handles permission decisions), Accept edits, Plan; number keys; Shift+Tab | Mode menu with Auto (default), Accept edits, Plan and Ask; 1–4 pick, Shift+Tab cycles, remembered. In Auto, each change Ask would confirm is checked against the user's request by the model: clear ones run, others ask with the reason; hard-to-undo commands always ask | Done |
| Stop interrupts the task (black square) | Same | Done |
| Pinned chats in their own **Pinned** section | Same | Done |
| Command palette (Cmd+K) with groups: Chats, Modes, Context, Settings, View, Actions… | Ctrl+K: Actions (new chat, modes, context pages, settings, sidebar, archive, recently deleted, what's new) and Chats, with content search | Done |
| Shortcuts: new session, sidebar, settings, Work / Code, Cmd+1–9 to sessions | Ctrl+N, Ctrl+B, Ctrl+,, Ctrl+Shift+1 / 2 / 3 (Work / Code / Agents), Ctrl+1–9 to the chats in the sidebar; all but Ctrl+1–9 can be rebound | Done |
| Voice: Autosend on/off; cancel discards a recording | Send after dictation; a discard button while recording | Done |
| Scheduled tasks: Once / Daily / Weekly (several days) / Monthly / Yearly, plain-words confirmation ("Every Monday at 9:00"), pause and resume, unread run in the sidebar | Same | Done |
| Knowledge: inline read / write pills; edit by asking; a "Sensitive data" toggle | Citation pills for pages read; saves show as steps; facts and pages editable directly; personal-data filter in Settings | Adapted |
| Connector detail: Interactive / Read-only functions, each with `Always allow`, `Refresh tools` | Same: the two groups apart, each with an Always allow switch, every function's own Allow / Ask / Off, and Refresh tools; an expanded connector stays open while settings change | Done |
| Canvas: versions with arrows, change view, restore; `Ask Work` on a selection | History: ‹ › through versions (who and when), a Changes view (added and struck-through lines), Restore as a new version; "Ask about this" on a selection; line breaks kept | Done |
| Canvas preview of HTML, React, Mermaid, Marp; several canvases as tabs | Apps (HTML, JS) and Sources open in the side panel as tabs. Mermaid blocks are drawn as diagrams and Marp decks (```marp, or Markdown whose front matter says `marp: true`, in answers or document canvases) as slides, both in the mini-app sandbox with their libraries from the CDN mini apps may use (so drawing one contacts cdn.jsdelivr.net; nothing is added to the app or its page policy) | Done |
| Mini apps: file list in code view, ZIP download | Single-file apps, downloaded as one HTML file | Adapted |
| Public share link for apps and spreadsheets | Local equivalent: "Share as web page" saves the conversation as one self-contained HTML file (questions, answers, code, citations and sources; light and dark; loads nothing), and apps download as single HTML pages. No hosted links: nothing leaves the device | Adapted |
| Spreadsheets: sheet tabs, `Open in Google Sheets` | Several sheets per canvas with tabs at the bottom (add one with +); .xlsx download holds every sheet, CSV the first. Google Sheets: no Google account integration | Done (Google Sheets N/A) |
| Libraries: numbered footnotes jumping to the passage, roles, org sharing | Numbered markers; the passages used are listed under the answer and a marker jumps to its passage. Roles and sharing: single user | Done (sharing N/A) |
| Workflows: tag in the input and above the sent message, `Workflow started: {name}` | Chip in the input and the message; the plan panel follows the steps | Done |
| Custom MCP connector: name, Server URL, description; auth detected (none, Bearer/Basic, OAuth 2.1 with dynamic registration) | Same | Done |
| Work Desktop: `+` → Folders → Add a folder, Remove all | Folders in the + menu | Done |
| Code Desktop right panel: Files, Git Changes, Terminal, Debug, Background processes; diff Unified / Split | Files (filter, read with line numbers), Changes with Unified / Split, Terminal with background processes, and Debug (model, folder, tokens, summary, every message's raw JSON, copy) | Done |
| Integrated browser with "Open in external browser?" | Links open in the system browser | Adapted |

## The inspiration apps

Reviewed in November 2026 feature by feature against the docs of Open WebUI, LibreChat, LobeHub,
Onyx, Osaurus and OpenBot (about 220 features). Every item of the review's shortlist is in the table
below. What was deliberately left out, and why:

- **Team and enterprise features** (channels, comments, SSO, SCIM, tenants, admin plane, shared
  marketplaces): this is a single-user desktop app.
- **A choice of vector database**: the local passage index with cached embeddings covers a person's
  libraries; a server database adds setup without benefit here.
- **Video generation, voice cloning, realtime voice models**: provider-specific and outside rig's
  current modalities; image generation, editing and voice conversation are covered.
- **Filters and pipes** (code that rewrites every message or adds providers): user tools run in the
  sandbox on demand; code in every request's path would sit outside it.

| Feature | Seen in | Here | Status |
|---|---|---|---|
| Any provider, local models | open-webui, LibreChat, lobehub, osaurus | 15 rig providers, Ollama, llama.cpp, custom base URLs | Done |
| Side-by-side model comparison | open-webui, LibreChat | Compare 2–3 models, keep the best answer | Done |
| Arena: blind comparisons and a leaderboard | open-webui (Evaluation) | A comparison can hide model names until you keep an answer ("You chose …"); each kept answer wins against the others, and `/leaderboard` (or the comparison's Leaderboard button) ranks your models by Elo with wins and losses, resettable. Verified live | Done |
| Code interpreter | open-webui, LibreChat | `run_code`: Python, JS, Bash in a sandboxed workspace (bubblewrap) | Done |
| Artifacts / runnable apps | LibreChat, lobehub | Mini apps and canvases | Done |
| Image generation | open-webui, LibreChat, lobehub | `generate_image` via rig (OpenAI, xAI, Together) | Done |
| Image editing and variations | open-webui, lobehub | `generate_image` takes a `reference`: the user's latest attached image or an earlier generated one (workspace files only). Image models that accept references (OpenAI gpt-image-1) edit them; xAI's endpoint refuses references, and the assistant is told to describe a new image instead. The reference path was verified live up to the provider; an actual edit needs an OpenAI key | Partial |
| Voice input | open-webui, lobehub | Dictation via rig transcription (OpenAI, Groq) | Done |
| Hands-free voice conversation | open-webui (call mode), onyx, osaurus, openbot.run | The wave button beside the model opens a call: it listens, sends when you pause (short sounds and lone blips are ignored), shows what it heard, reads the answer aloud and listens again; tap the orb to interrupt or pause, Esc or End to hang up. The microphone stays open for the call and never plays back through the speakers; auto-play stays quiet while a call reads answers itself. Verified live over two turns with real speech and a real speech-to-text model (Whisper): "What is the capital of France?" → "It is Paris." spoken, then "And the capital of Italy?" → "It is Rome." | Done |
| Read aloud (TTS) | open-webui, lobehub | Speaker under each answer, via rig speech models (xAI, OpenAI); verified live | Done |
| Audio provider setup | open-webui (Admin → Audio) | Settings → Audio, as Open WebUI sets it up. Speech to text: chat providers, any OpenAI-compatible server (base URL, key, model, language) or ElevenLabs. Text to speech: chat providers, OpenAI-compatible (Kokoro, LocalAI, Speaches…), xAI, ElevenLabs (account voices listed) or the system voice. Each has its voice, speed and keychain-stored key. Auto-send after dictation, auto-play of answers. OpenAI-compatible TTS and auto-play verified live | Done |
| RAG over documents | open-webui, onyx | Libraries and project files, cited | Done |
| Libraries that mirror folders | onyx (connectors), open-webui (synced knowledge) | Libraries → Mirror a folder: its documents (text, Markdown, PDF, HTML, data and code; hidden, build and dependency folders skipped) are indexed and kept in step — new files added, changed ones re-read, deleted ones removed — at start, every 10 minutes and with Sync now; unmirroring removes them. Verified live and by a unit test | Done |
| Whole document in context | open-webui (full context mode), LibreChat | `read_document` reads a library document in full (in 40,000-character parts) when search passages are not enough. Verified live: the model searched, then read the agreement end to end | Done |
| OCR: photos and scans of documents | open-webui (8 OCR engines), LibreChat | Images (PNG, JPEG, WebP, GIF) added to a library are read by the default model when it can see: an exact transcription with headings, lists and tables, figures described in one line. A PDF without a text layer says to add its pages as images. Verified live: a receipt image transcribed exactly and found by search | Done |
| Hybrid retrieval (keywords + embeddings) | open-webui, onyx, LibreChat, lobehub | Settings → Providers → Semantic search: any OpenAI-compatible embeddings server (Ollama, LM Studio, OpenAI, Mistral presets). Passages are embedded once and cached per model; BM25 and vector rankings merge by reciprocal rank fusion; if the server fails, keyword results come back with a note. Verified live against a local OpenAI-compatible test server ("cancel the contract" → the Termination section) and against a real model (BAAI bge-small-en-v1.5 served locally): "vacation allowance" finds nothing by keywords and ranks the handbook's holidays passage first in hybrid search | Done |
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
| Suggested follow-ups after each answer | open-webui (Follow-Up Prompts) | Three next questions under the last answer, written by the chat's model; one click sends. Settings → General turns them off. Verified live | Done |
| Prompt variables | open-webui, LibreChat | `{{current_date}}`, `{{current_time}}`, `{{current_weekday}}`, `{{user_name}}` fill in on send; any other `{{blank}}` opens a small form first, `{{tone|formal|casual}}` a choice. Works in typed prompts and workflows. Verified live | Done |
| Message queue beside steering | LibreChat, open-webui, openbot.run | While a task runs, Enter steers it and Alt+Enter queues the message; queued messages show above the composer with Send now, Edit and remove, and go one by one when the task ends. Verified live | Done |
| The model searches earlier chats | open-webui (History search, Reference Chats), openbot.run | `search_chats` and `read_chat`, offered while memory is on; temporary chats are never found. Verified live | Done |
| Model settings per chat | open-webui, LibreChat, onyx | Conversation menu → Chat settings: temperature, longest reply, and instructions for this chat only. Verified live | Done |
| Calendar of scheduled runs | (user request; Google Calendar-style) | Scheduled → Calendar: a month grid with planned runs from each task's schedule (solid) and past runs (dotted, from the conversations tasks started), colour per task; ‹ › and Today; the chosen day's agenda below opens a run's result or the task, and "New task on this day" pre-fills a one-time task. List or Calendar is remembered. Verified live | Done |
| Automations started by events | osaurus, openbot.run (webhook routines) | A scheduled task can run "When new files arrive in a folder": files already there are left alone; each new file, once it stops changing, starts a run with its path added to the prompt; hidden and partial downloads are skipped; the folder becomes readable by the assistant. Verified live: a bill dropped into the folder started a run that read it | Done |
| LaTeX in answers | open-webui, LibreChat, lobehub | `$$…$$`, `\[…\]`, `\(…\)` and `$…$` (amounts like "$5" left alone) render with KaTeX as MathML, drawn natively by the webview; math inside code is untouched. Verified live | Done |
| Everyday composer and message comforts | LibreChat, lobehub, open-webui | Unsent text stays with each conversation (across restarts) and attachments too; a paste over 2,500 characters becomes an attachment; attached files show as chips that open in place; long sent messages fold with Show more; code blocks download as files; "Mark as unread" in the conversation menu; a `notify` tool for desktop notifications. Verified live | Done |
| Run a code block from the message | open-webui, LibreChat | Python, JavaScript and shell blocks get a Run button: the block runs in the same sandbox as `run_code` (isolated when bubblewrap is present) and its output, errors and exit code show under it. Verified live, including a failing block | Done |
| Skill versions | lobehub, LibreChat (prompt versions) | Each save of a skill keeps the version it replaces (30 kept); the skill editor's History lists them, newest first, each readable and restorable (restoring keeps the current one too). Verified live and by a unit test | Done |
| PDF and Office files attached in chat | open-webui, LibreChat, lobehub | Upload, drag onto the composer, or paste from a file manager: PDF, Word (.docx), PowerPoint (.pptx) and OpenDocument (.odt, .odp) are read into text, paragraphs and slides kept in order; libraries and mirrored folders take the Office formats too. Verified live: a .docx and a PDF attached, and the answer quoted both | Done |
| The user's own date and time zone | open-webui (`{{CURRENT_DATE}}`, timezone), LibreChat | The system prompt names today's date in the user's time zone (e.g. Europe/Lisbon, UTC+01:00), not UTC; no clock time, so the prompt stays the same all day and provider caches stay warm | Done |
| Notify when an answer is ready | open-webui, lobehub | A desktop notification with the chat's title and the answer's opening when a reply finishes or fails while Lumo is in the background (Settings → General; on by default). Verified live: the notification call carried the title and the answer | Done |
| Edit without a new reply, and edit answers | LibreChat (Save vs Save & Submit), lobehub | Editing your message offers Save without reply (attachments kept); the pencil under an answer edits its final text, tool steps kept. The model reads the edited version from then on. Verified live: an edited answer was built on in the next reply | Done |
| Filter and group the chat list | LibreChat, open-webui, ChatGPT | The sidebar's list menu filters by tag, time (24 hours, 7 or 30 days), unread, or without scheduled runs, and under "Recent activity" groups chats as Today, Yesterday, Previous 7 days, Previous 30 days and by month. Verified live | Done |
| Links into the app | open-webui (`?q=`, `?models=`), lobehub | `lumo://new?prompt=…&model=vendor/model&mode=code` opens a new chat with the prompt typed in (never sent by the link); `lumo://chat/<id>` opens a conversation, and the conversation menu's Copy link gives it. A second launch hands the link to the running app. Verified live through a second launch | Done |
| Local OpenAI-compatible servers list their models | open-webui, osaurus, lobehub | The llama.cpp provider (also LM Studio, vLLM, LocalAI, Jan) lists what the server has loaded at `/v1/models`, like Ollama's listing; tested against a local server | Done |
| Copy error details | LibreChat, open-webui | A failed reply's card has Copy details: the error, model, mode, app version, platform, conversation and time, for a bug report | Done |
| Bookmark messages | LibreChat (bookmarks), lobehub | The bookmark under a message marks it; the chat header lists that chat's bookmarks and jumps to each, and search (Ctrl+K) lists bookmarks across all chats by their text. Bookmarks follow deleted exchanges. Verified live | Done |
| Readability: text size, zoom, contrast, motion | open-webui (UI scale, chat text size), lobehub, LibreChat (font size) | Settings → Appearance → Readability: chat text size (14–19 px), interface zoom 80–150% (also Ctrl+=, Ctrl+-, Ctrl+0, at the webview's own zoom), high contrast (stronger text and borders, visible focus, underlined links) and reduce motion. Verified live | Done |
| Search settings | open-webui, lobehub | A search box at the top of Settings finds a setting by name or other words for it ("zoom", "whisper", "pii") and opens its section with the card highlighted. Verified live | Done |
| Agent conversation starters and libraries | open-webui (model prompt suggestions, knowledge per model), lobehub (assistant opening questions and knowledge) | An agent's editor takes conversation starters (shown as one-click prompts on its new-chat page) and libraries it searches before answering and cites, with the reading tools allowed whatever its tool list. Both travel in agent export files. Verified live: a starter asked, the agent searched its library and cited the agreement | Done |
| Scheduled follow-ups in the same chat | openbot.run, osaurus (reminders) | Asked in a chat ("check this again tomorrow at 9"), `schedule_task` with `in_this_chat` makes a task whose runs continue that conversation with its context, marked "Scheduled: …"; the Scheduled page shows which chat it continues. Verified live: a one-time follow-up ran in the same chat at its minute | Done |
| Voice pause length | open-webui (silence threshold), lobehub | Settings → Audio → Voice conversation: how long a pause ends your turn, 0.8 to 4.5 seconds (1.3 by default) | Done |
| Version history for agents, workflows and tools | lobehub, open-webui (prompt and model versions) | Each save that changes an agent, workflow or your own tool keeps the version it replaces (30 per item, `chat/versions/`); History in its editor lists them with their text, and Load puts one back in the editor, the current one kept when saved. Verified live: an agent's description changed, then brought back | Done |
| Library document preview and batch removal | open-webui (knowledge file view), onyx (document explorer), lobehub | Click a document in a library to read its indexed text, with find-in-text highlighting; tick documents (or all) and remove them together. Test-a-search already shows each passage's score. Verified live | Done |
| Reranking library results | open-webui (reranking model), onyx (cross-encoder rerank) | Settings → Providers → Semantic search → Reranking: any Cohere/Jina-style `/rerank` server (llama.cpp, vLLM, Infinity, TEI, Jina, Cohere) orders a wider pool of keyword and semantic matches by how well they answer the question; on failure the search order stands, with a note. Verified live with a real cross-encoder (ms-marco-MiniLM-L-6-v2): the termination clause ranked first for "how can the contract be cancelled?" | Done |
| Agents in the local API, an Anthropic-style endpoint, a request log | osaurus (OpenAI-, Anthropic- and Ollama-compatible endpoints, agents as models) | `/v1/models` lists agents as `agent/<id>`; calling one answers with its instructions, notebook and model (no tools). `POST /v1/messages` takes Anthropic-style bodies (system apart, content blocks, `x-api-key`), plain or streamed with Anthropic's events. Settings → Local API → Recent requests shows the last 100 (time, endpoint, model, status, time to answer). Verified with curl: an agent answered through /v1/messages, a streamed reply, and the log | Done |
| Branch from a chosen answer, with or without files | LibreChat (fork options: up to here, include branches), lobehub | The branch button under an earlier answer opens a new chat that keeps the conversation up to that answer, with or without the attached files and images; notes on later messages stay behind and the new chat starts with no spend. The menu's Branch into a new chat copies all of it. Verified live | Done |
| A queue that lasts, reorders and pauses | LibreChat, lobehub | Messages queued with Alt+Enter while a reply runs are kept across restarts, can be moved up or down, paused (they wait when a turn ends) and resumed, cleared, or sent with Send next when nothing runs. Verified live: two queued, reordered and paused, the app restarted mid-reply, the queue came back and Send next sent the first | Done |
| Skills from a link | Claude (skills from GitHub), lobehub (plugin market) | Skills → Import → From a link: a skill's folder or SKILL.md on GitHub (tree, blob or raw links; a repository root tries HEAD, main and master) or any https Markdown link; the instructions are saved as your skill (scripts beside it are not downloaded). Verified live: anthropics/skills' pdf skill imported from its folder link | Done |
| Print or save a chat as PDF | open-webui (PDF export), LibreChat, ChatGPT | Conversation menu → Print or save as PDF: the conversation as it reads (Markdown, code, math) on white, with its title and date, folded messages unfolded, without the app around it; the system print dialog saves a PDF. Verified live: printed to a two-page PDF | Done |
| Save web pages to a library | onyx (web connector), open-webui (add a webpage to knowledge) | Each web source of an answer has Save to a library, and Libraries has Add a web page: the page's readable text (scripts and styles dropped) is saved under its title with its address, then searched and cited like any document. Verified live: a Wikipedia source saved (35,000 characters) | Done |
| Translate an answer | LibreChat, lobehub | The globe under an answer translates it into a listed or any language, shown below the original (Markdown and code kept) without changing the conversation. Verified live | Done |
| Quote a passage into the next message | LibreChat, lobehub | Selecting text in an answer (or a canvas) shows "Ask about this", which quotes it in the composer | Done |
| Slash commands | open-webui (built-in `/` commands), Vibe CLI | `/` lists app commands before skills: /new, /temporary, /model, /voice, /fork, /context, /settings, /export, /clear. Verified live (/context) | Done |
| Hover preview and temporary by default | open-webui, LibreChat | Hovering a chat in the sidebar shows how it left off (from the index, no file read); Settings → General can make new chats temporary by default. Preview verified live | Done |
| `@` to switch model | open-webui (@model), LibreChat | Typing `@` lists matching models (by name or id) above the skills; picking one switches the model. Verified live | Done |
| Rebindable keyboard shortcuts | open-webui | Settings → General → Keyboard shortcuts: palette, new chat, sidebar, settings, Work, Code and Agents; click, press the new keys (Ctrl/Cmd or Alt required), Escape cancels, Backspace unbinds, Reset restores; a combo moved to one action leaves the other. Verified live | Done |
| Sort the chat list | LibreChat | The Chats heading's sort button: recent activity, newest first, A–Z or unread first, remembered. Verified live | Done |
| Another chat as context | open-webui (Reference Chats) | + menu → Other chats: searchable list; the chosen conversation's text is attached as a file chip. Verified live | Done |
| Mermaid diagrams as SVG or PNG | LibreChat | SVG and PNG buttons on each diagram, saved through the system's save dialog (PNG at twice the size). Verified live | Done |
| Copy as rich text; feedback with a reason; a taller editor | LibreChat, open-webui | Copy puts formatted HTML (for Office and mail) and Markdown on the clipboard; a thumbs down asks what was wrong and keeps the reason with the conversation; the expand button gives the composer a tall editor for long prompts. Reason saving verified live | Done |
| Computer use | openbot.run, (Anthropic computer use) | Settings → Permissions → Computer use (off by default): a `computer` tool with screenshot, click, double/right click, move, type, key and scroll. Screenshots go to the model as scaled JPEGs (only the latest stays in the history); coordinates map back to the screen; every action except a screenshot passes the approval gate (Ask confirms each, Auto checks each against the request). Linux: xdotool and ImageMagick (X11), grim for Wayland screenshots; macOS: screencapture and cliclick. Verified live on Linux/X11: the model read the sidebar, clicked, saw it missed, corrected and reported the page title. The macOS path is untested here | Done |
| Your own tools | open-webui (Tools), LibreChat | Settings → Permissions → Your tools: a Python, JavaScript or Bash script with a JSON input schema; arguments arrive in `args`, the output is what it prints. Each call runs in the code sandbox (isolated with bubblewrap), never in the app, and passes the approval gate; the editor has a Test button. Verified live: a tool written and tested in the editor, then called by the assistant | Done |
| OpenAPI tool servers | open-webui (OpenAPI servers), lobehub | Settings → Permissions → API tool servers: add a spec address (and a bearer key, kept in the keychain); each operation becomes a tool (path and query parameters as arguments, the JSON body as `body`, local `$ref`s resolved). GET runs as a read; other methods go through the same approval and Auto check as connectors. Refresh, switch off or remove per server. Verified live against a local API: a weather read with the key and a note saved, from one request | Done |
| Tasks started from outside (webhook routines) | openbot.run, osaurus | Through the local API: `GET /v1/tasks` lists scheduled tasks and `POST /v1/tasks/{id}/run` starts one now, with optional `input` added to its prompt; the result arrives as a new conversation. Verified with curl | Done |
| Local OpenAI-compatible API | osaurus (OpenAI-, Anthropic- and Ollama-compatible endpoints) | Settings → General → Local API: `/v1/models` and `/v1/chat/completions` (plain and streamed) on 127.0.0.1, every request with the API key shown there; off by default, no CORS, any provider set up here. Verified with curl: refused without the key, 8 models listed, an answer and a streamed answer | Done |
| Quick ask from anywhere | osaurus | Global shortcut (Settings → General) opens a quick ask bar | Done |
| Choice of web search engine | onyx, open-webui, LibreChat | Settings → Providers → Web search: keyless (default), SearXNG (your instance, JSON on), Brave Search or Tavily, keys in the keychain; a failing engine falls back to the keyless search. Parsers unit-tested; the API engines need keys this setup does not have | Done |
| Agents exported and imported as files | osaurus (Bundles), lobehub, openbot.run | An agent's page → Export saves its settings, notebook and the user's own skills it uses as one `.lumo-agent.json`; Agents → Import adds it as a new agent (renamed if the name is taken; existing skills kept). Verified live | Done |
| Delete a message and its reply | LibreChat, lobehub | Trash under a sent message removes it and the reply; refused inside the summarized part or where other versions branch off. Verified live | Done |
| Export a conversation as JSON | open-webui, LibreChat | Conversation menu → Export as JSON: every turn, tool step and the usage totals | Done |
| Continue a cut-off reply | LibreChat, lobehub | Continue on a stopped or failed reply picks up from the work already done | Done |

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
- **Speech services that need their own keys:** dictation and voice conversation were verified with
  a real speech-to-text model (Whisper tiny.en behind an OpenAI-compatible endpoint, fed speech the
  xAI voice model spoke). ElevenLabs speech-to-text and ElevenLabs voices need keys this setup does
  not have.
- **Computer use on macOS and Wayland:** only Linux/X11 was run here. macOS uses screencapture and cliclick (no scrolling); Wayland uses grim for screenshots and ydotool (with ydotoold) for the mouse and keys, or wtype for typing. The Wayland commands are unit-tested, not run against a compositor.
- **Pull requests on unknown hosts:** a host that is not recognised and prints no link during the push gets the branch pushed, and the pull request is opened there by hand.
- **Branding:** the app still carries the Lumo name and logo; a new name is the owner's decision.
