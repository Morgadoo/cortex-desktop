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
| Spreadsheets (release notes, Sept 2026): analyse, clean and create Excel/CSV, edit in Canvas, download .xlsx | Sheet canvases (CSV) edited as a grid with rows/columns, saved as versions; download as .xlsx (rust_xlsxwriter) or CSV; uploaded .xlsx/.xls/.ods read into CSV (calamine); heavy analysis via run_code | Done |

### 11. Settings and appearance

| Vibe | Here | Status |
|---|---|---|
| Settings dialog (profile, permissions, general, appearance) | Same, plus Providers & models | Done |
| Light / dark / system | Same | Done |
| (none) | Accent colour (presets + custom) and background palettes: the graphite default, plus T3 Code's six | Extra |

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
| RAG over documents | open-webui, onyx | Libraries and project files, cited | Done |
| Web search with citations | open-webui, onyx, LibreChat | Keyless web search, citation pills, Sources | Done |
| Deep research | onyx | deep-research skill with parallel sub-agents | Done |
| MCP tools | LibreChat, lobehub, osaurus | MCP connectors and catalog | Done |
| Agents / presets | LibreChat, lobehub, openbot | Agents with persona, model, tools, skills and schedules | Done |
| Autonomous scheduled agents | openbot, grok-bot | Scheduled tasks with unattended / autonomous runs | Done |
| Memory | open-webui, LibreChat | Knowledge | Done |
| Prompt library | open-webui, LibreChat | Skills (`/name`) and Workflows | Done |
| Response versions and branching | LibreChat, lobehub | A branch tree: regenerate and edit branch at any message; ‹ 1 / n › switches; branch points inside a branch come back with it | Done |
| Conversation import | LibreChat, open-webui | ChatGPT and Claude exports (zip or conversations.json), visible branch only | Done |
| Archive / folders / tags | open-webui | Archive, pin, projects as folders, and tags searchable with `#tag` | Done |
| Export | all | Each conversation as Markdown; all data | Done |
| Personal-data filter | (none) | Optional redaction before cloud models | Extra |
| Context management | (best practice) | Tool-result clearing, compaction at 60%, measured tokens and cost | Done |
| Design system | T3 Code | Tokens, shared components, T3's control styling and palettes | Done |

## Known gaps

- **Reference coverage:** only 5 of the Vibe screenshots survived in the session that built this,
  so sections 1–6 were checked against images and the rest against written notes of them.
  Comparing against fresh screenshots of the current Vibe app is the way to close this.
- **Voice input against a real provider:** dictation was verified end to end in the app with a
  virtual microphone and a mock transcription endpoint. The test proxy has no transcription
  endpoint (404), so it has not run against a real speech-to-text model yet.
- **Pull requests to non-GitHub remotes:** the branch is pushed, and the pull request is opened on that host by hand.
- **Branding:** the app still carries the Lumo name and logo; a new name is the owner's decision.
