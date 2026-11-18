# Inspiration review: coverage

Every feature the November 2026 review of Open WebUI, LibreChat, LobeHub, Onyx, Osaurus and
OpenBot marked missing or partial (223 rows), re-checked against the code, then brought up to
date after the work that followed (the "Now" column). DONE needs evidence in the code or a row
in [feature-parity.md](feature-parity.md); OUT OF SCOPE rows are team, server or
platform-only features, explained there under "deliberately left out".

| Status | At the re-audit | Now |
|---|---|---|
| DONE | 58 | 165 |
| PARTIAL | 86 | 26 |
| MISSING | 48 | 1 |
| OUT OF SCOPE | 31 | 31 |
| **Total** | **223** | **223** |

Per app now (done / partial / missing / out of scope): Open WebUI 42 / 3 / 0 / 9 · LibreChat 39 / 3 / 0 / 4 · LobeHub 24 / 5 / 0 / 5 · Onyx 18 / 3 / 0 / 5 · Osaurus 20 / 5 / 1 / 4 · OpenBot 22 / 7 / 0 / 4.

| App | Feature | Was | Now | Evidence, or what is missing |
|---|---|---|---|---|
| Open WebUI | Message queue (send after the reply) | PARTIAL | DONE | parity L270; chat.ts:1521–1528 (Alt+Enter queues) |
| Open WebUI | Autocomplete as you type | MISSING | DONE | parity: Autocomplete as you type (chat_complete_prompt; composer hint, Tab accepts) |
| Open WebUI | Follow-up prompts after each reply | MISSING | DONE | parity L268; `chat_follow_ups` command |
| Open WebUI | Expand the message box | MISSING | DONE | parity L287 (expand button, tall editor) |
| Open WebUI | Compare Text (prompt vs revision) | MISSING | DONE | parity: Compare answer versions (word diff between any two versions of a reply, from a regenerated answer or an edited prompt) |
| Open WebUI | Trigger characters `/ @ # $ :` | PARTIAL | DONE | `/` commands and skills, `@` models and agents, `#` libraries, knowledge pages, canvases and chats, `:` emoji (parity: Emoji shortcodes). Open WebUI's `$` (tools) is covered by `/` skills |
| Open WebUI | Built-in slash commands | PARTIAL | DONE | parity L280: /new, /temporary, /model, /voice, /fork, /context, /settings, /export, /clear |
| Open WebUI | Default upload mode (search vs full) | MISSING | DONE | Settings → Profile → Uploaded files: every file in full, every file searched, or (default) files over 20k characters searched. A searched file goes to the Uploads library and the message tells the assistant to search it; the bubble shows it as a 'searched' chip (uploads.ts, chat_upload_to_library). Verified live: a 33k-character log was searched and the one failure in it found |
| Open WebUI | LaTeX, tables, task lists, Mermaid | PARTIAL | DONE | LaTeX parity L275; GFM tables and task lists dom.ts:149; Mermaid parity L200 |
| Open WebUI | `:::writing` blocks | MISSING | DONE | parity: Writing blocks (dom.ts `extractWriting`; the system prompt asks for them) |
| Open WebUI | Inline visualizations | PARTIAL | DONE | Apps preview inline in the reply with an Expand bar (parity L129); Mermaid drawn inline (L200) |
| Open WebUI | Response auto-scroll | PARTIAL | DONE | Follows while at the bottom, and a setting turns following off (parity: Sound and stream following) |
| Open WebUI | Chat parameters (prompt, params per chat/account/model) | PARTIAL | DONE | Per chat: temperature, length, top-p, instructions; defaults for every chat (parity: Defaults for every chat, and top-p) |
| Open WebUI | `{{USER_NAME}}`, `{{CURRENT_DATE}}` | MISSING | DONE | parity L269 |
| Open WebUI | Temporal awareness | MISSING | DONE | The prompt states the date in the user's time zone (e.g. Europe/Lisbon, UTC+01:00); clock time is left out so prompt caches stay warm; date math runs in `run_code` |
| Open WebUI | URL parameters / deep links | MISSING | DONE | `lumo://new?prompt=&model=&mode=` and `lumo://chat/<id>`; Copy link in the chat menu (parity: Links into the app) |
| Open WebUI | Rebindable shortcuts | PARTIAL | DONE | parity L283; shortcuts.ts:9–16 |
| Open WebUI | Global defaults for every model | PARTIAL | DONE | Defaults for every chat: temperature, length, top-p (parity: Defaults for every chat) |
| Open WebUI | Version history of model configs | MISSING | DONE | Agents, workflows and tools keep 30 earlier versions each, with History and Load (parity: Version history) |
| Open WebUI | Evaluation: arena, leaderboard | PARTIAL | DONE | parity L230 (blind compare, Elo `/leaderboard`) |
| Open WebUI | Open Terminal (shell, files, site preview) | PARTIAL | DONE | Terminal and Files panels (parity L122, L208); a running command that prints a local address gets a preview button that opens the page in Lumo's browser window (terminal.ts, devsite.ts, chat_open_preview). Verified live with python -m http.server |
| Open WebUI | Custom tools with editor, versions, Tool Search | PARTIAL | DONE | Editor with Test, `find_tools`, and versions (parity: Version history) |
| Open WebUI | Filters (Functions) | MISSING | OUT OF SCOPE | Left out: "Filters and pipes" (parity L223) |
| Open WebUI | Actions: custom message-toolbar buttons | MISSING | DONE | parity: Answer actions (your own prompts under each answer, editable) |
| Open WebUI | Pipes (providers from code) | MISSING | OUT OF SCOPE | Left out: "Filters and pipes" (parity L223) |
| Open WebUI | OpenAPI tool servers | MISSING | DONE | parity L290 |
| Open WebUI | Skills with supporting files and versions | PARTIAL | DONE | Versions (parity L278) and supporting files: a skill's folder holds templates, notes and scripts; the editor lists, views, edits, adds and removes them; `use_skill` lists them and reads one with `file` (skills.rs add_file/read_file/loaded). Verified live: the model loaded a skill, then read refs/temperatures.md for the answer |
| Open WebUI | Prompts with variables and versions | PARTIAL | DONE | Variables in prompts and workflows; skills and workflows keep versions |
| Open WebUI | One-shot timer back into this chat | PARTIAL | DONE | `schedule_task` with `in_this_chat` continues the chat it was asked in (parity: Scheduled follow-ups) |
| Open WebUI | Webhooks on completion; `notify` tool | MISSING | DONE | `notify` tool and task webhooks with Slack/Discord-friendly fields (parity: Webhooks when a task finishes) |
| Open WebUI | Save web search results to knowledge | MISSING | DONE | Each web source has Save to a library; Libraries has Add a web page (parity: Save web pages to a library) |
| Open WebUI | Hybrid search with reranking | PARTIAL | DONE | BM25 + embeddings fused, then an optional Cohere/Jina-style reranker (parity: Reranking) |
| Open WebUI | Choice of vector databases | MISSING | OUT OF SCOPE | Left out: "A choice of vector database" (parity L219) |
| Open WebUI | Extraction engines (Tika, Docling, OCR…) | MISSING | PARTIAL | PDF, Word, PowerPoint, OpenDocument, HTML, text and OCR of images by a vision model. Missing: Tika or Docling as engines |
| Open WebUI | Always use full text | MISSING | DONE | parity L241 (`read_document`) |
| Open WebUI | Knowledge file browser with preview | PARTIAL | DONE | Click a document to read its indexed text with find-in-text (parity: Library document preview) |
| Open WebUI | Knowledge base sync of folders and repos | PARTIAL | DONE | parity L240 (Mirror a folder, incremental, every 10 min) |
| Open WebUI | Tabs side panel, split panes | PARTIAL | PARTIAL | Apps, canvases and Sources as panel tabs (parity L200; canvas.ts:55). Missing: chats and files as tabs, split panes |
| Open WebUI | Hands-free voice and video calls, realtime | PARTIAL | DONE | Hands-free call (parity L236). Video and realtime models are left out (L221) |
| Open WebUI | Voice cloning | MISSING | OUT OF SCOPE | Left out (parity L221) |
| Open WebUI | Image generation and editing, ComfyUI | PARTIAL | PARTIAL | Editing through a reference image (parity L234, itself Partial). Missing: ComfyUI or any local image backend |
| Open WebUI | Video generation | MISSING | OUT OF SCOPE | Left out (parity L221) |
| Open WebUI | Chat overview (map with compaction marks) | MISSING | DONE | Outline in the chat header with the summarized point marked (parity: Chat outline) |
| Open WebUI | Models search past chats | PARTIAL | DONE | parity L271 (`search_chats`, `read_chat`) |
| Open WebUI | Reference chats | MISSING | DONE | parity L285 |
| Open WebUI | Sidebar hover previews | MISSING | DONE | parity L281; main.ts:212–215 |
| Open WebUI | Chat sharing links | MISSING | OUT OF SCOPE | Hosted links are server-only; the local equivalent "Share as web page" exists (parity L202) |
| Open WebUI | Comments | MISSING | OUT OF SCOPE | Team feature (parity L217) |
| Open WebUI | Data controls: uploaded-file management | PARTIAL | DONE | Archive, trash, export (parity L308), and Uploaded files in Settings → Profile: every file and image sent in any chat, filterable, with Open chat and Remove (the message keeps a note in its place) (uploads.rs). Verified live |
| Open WebUI | Notes: rich editor with toolbar | PARTIAL | DONE | Canvas document editor: a formatting toolbar (bold, italic, strikethrough, code, H1–H3, lists, checklist, quote, link, table, code block) with Ctrl+B / Ctrl+I / Ctrl+K, undoable, toggling off what is already applied (`mdformat.ts`, checked by npm run check:i18n) |
| Open WebUI | Notes: AI Enhance in place | PARTIAL | DONE | parity: Improve with AI in editors (selection or whole page rewritten in place, Undo) |
| Open WebUI | Notes: attach a note to a chat | PARTIAL | DONE | parity: Knowledge pages and canvases as context (`#` in the composer) |
| Open WebUI | Administration (RBAC, SSO…) | MISSING | OUT OF SCOPE | Team/enterprise (parity L217) |
| Open WebUI | Channels | MISSING | OUT OF SCOPE | Team (parity L217) |
| LibreChat | Favorites pinned in the palette | MISSING | DONE | Ctrl+K opens with pinned chats and agents first (parity: Pinned chats and agents in the palette) |
| LibreChat | Recently used files to attach again | MISSING | DONE | parity: Recent files (recent_files.rs) |
| LibreChat | Long paste becomes a file | MISSING | DONE | parity L276 |
| LibreChat | Quote a passage into the next message | PARTIAL | DONE | parity L279 (answers and canvases) |
| LibreChat | Enter: Queue, Steer or Interrupt | PARTIAL | DONE | parity L270 (Enter steers, Alt+Enter queues), Stop (L191) |
| LibreChat | Queue list: send now, edit, reorder, new chat, remove | MISSING | DONE | Queued messages: send now, edit, move up/down, remove, pause, resume, clear, send next (parity: A queue that lasts) |
| LibreChat | `@` switches model, preset or agent | PARTIAL | DONE | `@` lists models and agents; an agent opens a new chat with it (parity: @ starts a chat with an agent) |
| LibreChat | Switch model mid-chat | PARTIAL | DONE | `@model` and `/model` (parity L280, L282); the conversation stores its model (api.ts `Conversation.model`) |
| LibreChat | Drafts saved locally | MISSING | DONE | parity L276 (text and attachments, across restarts) |
| LibreChat | Collapse long user messages | MISSING | DONE | parity L276; chat.ts:710–720 |
| LibreChat | Smooth streaming (fade-in, elapsed time) | PARTIAL | DONE | Elapsed seconds while working, and new characters fade in once as they arrive (`fadeInFresh`, chat.ts), respecting reduced motion. Verified live: the newest words of a streamed reply faded in while earlier text stayed still |
| LibreChat | Resumable streams | MISSING | OUT OF SCOPE | Server-only: a web client reconnecting to a server stream. Here the turn runs in the Rust process, not the page |
| LibreChat | Fork options (visible, related, all; leave out files) | PARTIAL | DONE | Branch from a chosen answer, with or without attached files (parity: Branch from a chosen answer) |
| LibreChat | Continue a cut-off reply | MISSING | DONE | parity L298; chat.ts:836–841 |
| LibreChat | Save an edit without a new reply | MISSING | DONE | Save without reply on your message; edit an answer in place (parity: Edit without a new reply) |
| LibreChat | Rating with a reason and a comment | PARTIAL | DONE | parity L287; chat.ts:915–918 (free-text reason stored with `chat_rate_reason`) |
| LibreChat | Download a code block | MISSING | DONE | parity L276 |
| LibreChat | Run a code block | PARTIAL | DONE | parity L277 |
| LibreChat | Copy as rich text | MISSING | DONE | parity L287; chat.ts:921–928 |
| LibreChat | Agent marketplace | MISSING | OUT OF SCOPE | Shared marketplaces left out (parity L217) |
| LibreChat | Agent plugins (skills, MCP, hooks in one package) | MISSING | DONE | Claude plugins import skills (with their supporting files), agents, MCP servers and now hooks: hooks/hooks.json PreToolUse, PostToolUse and Stop map to Lumo's hooks, arrive turned off, and get Claude's input fields (plugins.rs install, hooks.rs from_claude). Verified live: a plugin folder imported, its after-call hook read tool_name and annotated a web search, its Stop hook ran the plugin's own script |
| LibreChat | Agents API | MISSING | DONE | Agents answer through the local API as `agent/<id>` (parity: Agents in the local API) |
| LibreChat | Prompt variables | MISSING | DONE | parity L269 |
| LibreChat | Prompt versions with Production | MISSING | DONE | Workflows (Lumo's saved prompts) keep versions, and one can be marked production: that version runs from the + menu while saved edits stay a draft (versions.rs set_production, workflows.rs views). Verified live: a 5th step added after marking stayed out of the sent run |
| LibreChat | PR status and CI for a chat's workspace | PARTIAL | PARTIAL | PR opened (parity L121). Missing: PR status and CI checks |
| LibreChat | Jina / Cohere reranking | MISSING | DONE | Any Cohere/Jina-style `/rerank` server (parity: Reranking) |
| LibreChat | Upload files as text | PARTIAL | DONE | Chat attachments go in full as text (chat.ts:1694–1700) |
| LibreChat | OCR | MISSING | DONE | parity L242 |
| LibreChat | Mermaid as SVG/PNG | MISSING | DONE | parity L286 |
| LibreChat | Artifact in a new window | MISSING | DONE | parity: Apps in a window of their own (chat_open_app_window; same sandbox) |
| LibreChat | Pinned section (chats, models, agents, presets) | PARTIAL | DONE | Pinned chats, and agents pinned from their page (Pin), which start a new chat with the agent from the sidebar; models are favourites in the model picker and command palette. Verified live: EU AI Brief appeared under Pinned |
| LibreChat | Filter and sort chats | MISSING | DONE | Filter by tag, time, unread, scheduled runs; sort four ways (parity: Filter and group the chat list) |
| LibreChat | Mark as unread | PARTIAL | DONE | parity L276; chat.ts:2811 |
| LibreChat | Chat options incl. duplicate | PARTIAL | DONE | Rename, move, branch, archive, delete (chat.ts:2790–2831); "Branch into a new chat" copies the whole chat (commands.rs:1655) |
| LibreChat | Shareable links / QR | MISSING | OUT OF SCOPE | Server-only; local "Share as web page" (parity L202) |
| LibreChat | Temporary by default | PARTIAL | DONE | parity L281 |
| LibreChat | Import from apps; export in several formats | PARTIAL | DONE | Import ChatGPT and Claude (parity L262); export Markdown, JSON (L297), web page (L202), everything (L264) |
| LibreChat | Query parameters | MISSING | DONE | `lumo://new?prompt=…&model=…` (parity: Links into the app) |
| LibreChat | UI scale, font size, RTL, clock format | MISSING | PARTIAL | Chat text size, interface zoom (parity: Readability) and clock format (parity: Clock format) done. Missing: right-to-left layout (no right-to-left interface language is offered yet) |
| LibreChat | High-contrast theme | MISSING | DONE | High contrast in Settings → Appearance → Readability |
| LibreChat | Multilingual UI | MISSING | DONE | parity: Interface languages (`i18n.ts`, `ui_language.rs`, `src-tauri/i18n/`; Settings → Appearance → Language). Verified live: Portuguese switched the whole interface (menus, composer, settings, greeting and numeric templates, toasts) while chat titles and project names stayed as written; Dutch, which does not ship, was translated by the chosen model (1787 strings in 15 requests) and applied |
| LibreChat | Desktop notification and sound for finished replies | MISSING | DONE | Notification with an optional chime (parity: Sound and stream following) |
| LibreChat | Every shortcut rebindable | PARTIAL | DONE | Settings → General → Keyboard shortcuts: every shortcut can be rebound, including prompt history (Ctrl+R), folding activity (Ctrl+O), the mode cycle (Shift+Tab) and the nine chat shortcuts (Ctrl+1–9); prompt history and the mode cycle act only in the message box |
| LibreChat | Settings search | MISSING | DONE | Search box at the top of Settings (parity: Search settings) |
| LibreChat | Security / admin | MISSING | OUT OF SCOPE | Team/enterprise (parity L217) |
| LibreChat | Image generation and editing, local server | PARTIAL | PARTIAL | Editing by reference (parity L234). Missing: a local image server |
| LobeHub | LaTeX in streaming answers | PARTIAL | DONE | parity L275 |
| LobeHub | Translate a message | MISSING | DONE | chat.ts:730–760 (`chat_translate`; not in the parity doc) |
| LobeHub | Fold a long reply | MISSING | DONE | Fold button on long answers (parity: Fold a long answer) |
| LobeHub | Delete a message | MISSING | DONE | parity L296 |
| LobeHub | Select several messages, delete or forward | MISSING | DONE | parity: Select messages (copy, forward to a chat or agent, delete with re-keyed history) |
| LobeHub | Agent market | MISSING | OUT OF SCOPE | Shared marketplaces (parity L217) |
| LobeHub | Skills marketplace (browse, publish) | PARTIAL | OUT OF SCOPE | Shared marketplaces (parity L217); SKILL.md import exists (parity L82) |
| LobeHub | Agent groups (sequential, parallel, debate…) | PARTIAL | DONE | An agent can host a group: members picked in order in the agent editor, and a mode (parallel, sequential, debate). The host hands each request to them with `delegate` and answers from their reports; sequential members see the earlier reports, a debate adds a second round where each answers the others (subagents.rs Mode, agents.rs group_prompt). Verified live: a two-member debate ran both rounds and the host summed up |
| LobeHub | External agents (Claude Code, Codex) | MISSING | DONE | parity: Coding agent programs as models (agent_cli.rs) |
| LobeHub | Tasks with delegation, status, comments | PARTIAL | DONE | Scheduled tasks and sub-agents (parity L165, L248), and comments on a task (the speech-bubble on its row): every run gets them as notes to follow, newest last (`tasks::run_prompt`). Verified live: a comment was saved on a task; unit test checks the run prompt |
| LobeHub | Agent builder as a chat | PARTIAL | DONE | New agent is a conversation: the first message drafts it, each next one revises the draft shown beside the exchange, then Review and save (agents.rs refine_prompt, agents.ts newAgent). Verified live |
| LobeHub | Greeting and conversation starters per agent | MISSING | DONE | Conversation starters on the agent's new-chat page (parity: Agent conversation starters) |
| LobeHub | Stats: cost, cache, tokens, models per agent | PARTIAL | DONE | Spend per agent (parity L253), and the profile menu's month gauge opens Usage this month: tokens, cost and chats by model and by agent, with bars. Verified live with this month's real usage (grok-4.7, deepseek-flash… by model; News Briefer, EU AI Brief… by agent) |
| LobeHub | Export or copy an agent | MISSING | DONE | parity L295 |
| LobeHub | Resource library (uploads, images, drafts) | PARTIAL | PARTIAL | Libraries hold documents; Apps page lists apps (apps.ts). Missing: generated images and canvases in one store |
| LobeHub | Semantic search | MISSING | DONE | parity L243 |
| LobeHub | Batch operations (folder upload, move, delete) | MISSING | DONE | Folders mirrored, several documents removed or moved at once (parity: Move documents between libraries) |
| LobeHub | Relevance scores, excerpt preview | PARTIAL | DONE | Passage scores in Test a search; document preview (parity: Library document preview) |
| LobeHub | Notion ZIP import | MISSING | DONE | parity: Import an export into a library (libraries.rs `import_archive`) |
| LobeHub | Pages: long documents with a page list | PARTIAL | DONE | Apps → Documents and sheets lists every document and sheet canvas, newest first, with search, Download and Open (in its conversation). Verified live |
| LobeHub | Image page (ratio, size, count, seed…) | PARTIAL | PARTIAL | `generate_image` with a reference (parity L234). Missing: a page with those controls |
| LobeHub | Video generation | MISSING | OUT OF SCOPE | Left out (parity L221) |
| LobeHub | Prompt Extend before generation | MISSING | DONE | parity: Improve prompt (Ctrl+E, Ctrl+Z undoes) |
| LobeHub | Topic panel: status tabs, card/list views | PARTIAL | PARTIAL | Archive. Missing: status tabs and views |
| LobeHub | Sort and group chats | MISSING | DONE | Sort four ways and date groups Today, Yesterday, 7 and 30 days, months |
| LobeHub | Smart rename / rename | PARTIAL | DONE | Automatic title (engine.rs:804–808), Rename (chat.ts:2790) |
| LobeHub | Open in new tab, right panel, new window | MISSING | DONE | Apps and canvases in the right panel or their own window; any chat opens in a window of its own from its menu (chat_open_chat_window), sidebar folded, both windows streaming the same chat. Verified live |
| LobeHub | Duplicate, move, forward to another agent | MISSING | DONE | Branch (duplicate), move to project, Continue with an agent (parity: Continue a chat with an agent) |
| LobeHub | Export as screenshot, text, PDF, JSON | PARTIAL | PARTIAL | Markdown, JSON and PDF (print) export. Missing: export as a screenshot image |
| LobeHub | Share link | MISSING | OUT OF SCOPE | Server-only (parity L202) |
| LobeHub | Memory categories, weight, confidence, links | MISSING | DONE | parity: Memory categories, importance, confidence, links (memory.rs `Filing`) |
| LobeHub | Filter memories | PARTIAL | DONE | Find, by source and by date (parity: Filter saved facts) |
| LobeHub | Memory agent | PARTIAL | PARTIAL | `sanitize-my-knowledge` skill (parity L60). Missing: an automatic reviewer |
| LobeHub | Multi-user, SSO, PWA, IM channels | MISSING | OUT OF SCOPE | parity L256–257 |
| Onyx | Auto model router | MISSING | DONE | parity: Auto model (router.rs) |
| Onyx | Reasoning level and temperature | PARTIAL | DONE | Fast/Think (parity L74), temperature per chat (L272) |
| Onyx | Agents with starters and knowledge | PARTIAL | DONE | Agents carry starters and libraries they search and cite (parity: Agent conversation starters and libraries) |
| Onyx | Per-turn reminder; system prompt replaced | MISSING | DONE | Agent instructions replace the persona; "Reminder on every turn" rides on each request (parity: A reminder on every turn) |
| Onyx | Knowledge cutoff date | MISSING | DONE | parity: Time cutoff for library search (`since` on search_library) |
| Onyx | MCP and OpenAPI actions | PARTIAL | DONE | parity L290 |
| Onyx | Craft: Outputs / Files / Apps tabs | PARTIAL | PARTIAL | Canvases, apps, Files panel. Missing: an outputs tab per session |
| Onyx | Queue a follow-up | PARTIAL | DONE | parity L270 |
| Onyx | User library across sessions | PARTIAL | DONE | Libraries usable from any chat (parity L49, L86) |
| Onyx | Sandboxed browser with screenshots | PARTIAL | PARTIAL | Browser reads pages; screenshots only of the whole screen through computer use (parity L288). Missing: browser screenshots |
| Onyx | Agentic hybrid RAG | PARTIAL | DONE | parity L243, L241 |
| Onyx | Connectors indexed and synced | PARTIAL | PARTIAL | Folders are indexed (parity L240). Missing: MCP connectors indexed into a library |
| Onyx | Search mode with source cards | MISSING | DONE | parity: Search mode (searchmode.ts, search_all.rs) |
| Onyx | Time range / source filter | MISSING | DONE | `web_search` recency (parity: Recent results only) and `sites` (parity: Web search on chosen sites) |
| Onyx | Cited vs found sources; add one in full | PARTIAL | DONE | Sources panel lists Cited, then Also found (search results the answer did not use); + attaches the page's full article text to the next message (chat.ts sourcesOf/openSourcesPanel). Verified live |
| Onyx | Web search providers | PARTIAL | DONE | parity L294 |
| Onyx | Document sets | PARTIAL | DONE | Libraries per project (`Project.libraryId`) and per agent (agent editor → Libraries, `Agent.libraries`, searched before answering) |
| Onyx | Share a Craft app | MISSING | OUT OF SCOPE | Server-only (parity L202) |
| Onyx | Auto-send on pause | PARTIAL | DONE | parity L236 |
| Onyx | Read, then listen again | PARTIAL | DONE | parity L236 |
| Onyx | Collaboration | MISSING | OUT OF SCOPE | Team (parity L217) |
| Onyx | Memory reference / update switches | PARTIAL | DONE | Context → Knowledge: Use knowledge and Save new facts are separate (`Memories.update`); with saving off, knowledge is read but remember, forget and knowledge_write are neither offered nor run (test `knowledge_can_be_used_without_being_changed`) |
| Onyx | Enterprise features | MISSING | OUT OF SCOPE | parity L217 |
| Onyx | Custom code for PII and query rejection | PARTIAL | OUT OF SCOPE | Filters left out (parity L223); personal-data filter exists (L265) |
| Onyx | Served as an MCP server | MISSING | DONE | `/mcp` on the local API: search_libraries, ask, list_models, list_tasks, run_task (parity: Lumo as an MCP server) |
| Onyx | Extension, widget, Slack/Discord bots, CLI | MISSING | OUT OF SCOPE | Channels N/A (parity L256); scripts can use the local API (L292) |
| Osaurus | Global hotkey | MISSING | DONE | parity L293 |
| Osaurus | App Intents (Shortcuts, Spotlight, Siri) | MISSING | MISSING |  |
| Osaurus | Built-in local runtime, model downloads | PARTIAL | PARTIAL | Ollama, llama.cpp (parity L228). Missing: built-in runtime, downloads |
| Osaurus | Local API: OpenAI, Responses, Anthropic, Ollama | MISSING | DONE | `/v1/chat/completions`, `/v1/responses` (plain and streamed events), `/v1/messages`, and Ollama's `/api/tags`, `/api/chat` (NDJSON stream), `/api/generate`, `/api/version`; verified live with curl against claude-haiku-5-5 (streamed chat counted 1–5, generate answered Lisbon, Responses answered Paris and streamed created/delta/done/completed events; no token → refused) |
| Osaurus | Inference runtime (scheduler, batching) | MISSING | OUT OF SCOPE | An inference server; the app uses Ollama or llama.cpp for that |
| Osaurus | Orchestrator | PARTIAL | DONE | Route new chats to agents (Agents page): a new chat that clearly fits an agent goes to it, decided by the fast model from the agents' descriptions, and the chat says so; general questions stay with the assistant (engine.rs route_to_agent). Groups orchestrate their members. Verified live both ways |
| Osaurus | Built-in app agents | PARTIAL | DONE | MCP connectors, and ready-made specialist agents for the apps and data on this computer: Calendar Assistant (calendar and contacts), Data Analyst (run_code, sheets), Code Reviewer (read-only), Translator and Meeting Prep, besides Researcher, News Briefer, File Assistant and Writer |
| Osaurus | External agent runtimes | MISSING | DONE | parity: Coding agent programs as models (Claude Code, Codex, Gemini CLI) |
| Osaurus | Typed workflows, designer agent | PARTIAL | DONE | Inputs typed as {{name:number}} (or date, time, url, email, long), optional with ?, choices as {{name&#124;a&#124;b}}, each with its own control and checked before sending (promptvars.ts); a designer in the workflow editor drafts or revises the workflow from a description (chat_draft_workflow). Verified live |
| Osaurus | Triggers: schedule, folder, webhook, feed | PARTIAL | DONE | Schedules, new files (parity L274), webhooks through the local API (L291) and feeds: a task can start when an RSS or Atom feed has new items (`Frequency::NewFeedItems`, `parse_feed`). Verified live: the first check recorded the feed's two posts; after a third was added, one run started with its title and link in the prompt |
| Osaurus | VM per agent | PARTIAL | PARTIAL | bubblewrap sandbox (parity L231). Missing: a separate environment per agent |
| Osaurus | Browser session per agent | PARTIAL | PARTIAL | Missing: per-agent sessions |
| Osaurus | Computer use with rules per app | MISSING | DONE | Settings → Permissions → Computer use: never act in these apps, and only act in these apps, matched in the front window's title; a refused action tells the model to ask the user (`computer::app_allowed`, tested) |
| Osaurus | Calendar, Mail, Contacts… integrations | MISSING | DONE | parity: Calendars and contacts (calendar.rs: ICS links/files with repeat rules, vCard); mail through the Proton Mail connector |
| Osaurus | Agent channels | MISSING | OUT OF SCOPE | parity L256 |
| Osaurus | Import Claude plugins | MISSING | DONE | parity: Import Claude plugins (plugins.rs: skills, commands, agents, MCP servers) |
| Osaurus | Plugin registry | MISSING | DONE | A Claude plugin marketplace on GitHub lists its plugins in the app to tick and import (parity: Import Claude plugins); verified with Anthropic's official marketplace |
| Osaurus | MCP server | MISSING | DONE | `/mcp` on the local API: search_libraries, ask, list_models, list_tasks, run_task (parity: Lumo as an MCP server) |
| Osaurus | Agent run endpoint | MISSING | DONE | `POST /v1/tasks/{id}/run` and agents as models |
| Osaurus | Bundles: agents, skills, MCP servers | MISSING | DONE | Agent files carry the agent, notebook, its skills and the MCP servers it uses, without secrets |
| Osaurus | Developer tools, request log | PARTIAL | DONE | Recent requests log for the local API |
| Osaurus | On-device transcription | PARTIAL | PARTIAL | Any OpenAI-compatible server (parity L238). Missing: a built-in model |
| Osaurus | Wake word | MISSING | DONE | parity: Wake word (`wakeword.ts`, `wakematch.ts`; Settings → Audio). Verified live with a virtual microphone: an unrelated phrase was transcribed and ignored; "Hey Lumo, what is two plus two? …" opened a new chat that asked it (answer: 4); with the window hidden, "Hey Lumo" alone brought Lumo forward into a voice conversation |
| Osaurus | System-wide dictation hotkey | MISSING | DONE | parity: Dictation anywhere (`dictation.rs`, `dictate.ts`; Settings → Audio). Verified live: Ctrl+Alt+D over an xterm opened the Listening pill, speech played into a virtual microphone was recorded (7.4 s WAV) and transcribed, and the text was typed into the xterm; changing the shortcut released the old one; auto-finish on pause and paste mode also ran |
| Osaurus | On-device images | PARTIAL | PARTIAL | Cloud only (parity L233) |
| Osaurus | Encrypted sync between devices | MISSING | DONE | parity: Sync between devices (`sync.rs`; Settings → Profile). Verified live with two configurations sharing one folder: device A sent 105 encrypted items (no plaintext in the folder); device B refused a wrong passphrase, then received A's 5 extra conversations and sent its renamed chat, which reached A; a chat deleted on A was moved to B's trash on B's next sync |
| Osaurus | Workspaces | MISSING | OUT OF SCOPE | Team (parity L217) |
| Osaurus | Encryption at rest | PARTIAL | DONE | parity: Encrypt data on this computer (`at_rest.rs`; Settings → Profile). Verified live: 338 files were sealed (no plaintext left outside sandboxes), the app restarted, unlocked from the keychain, listed and searched chats and saved a new one sealed; decrypting restored every file in the clear |
| Osaurus | Telemetry toggles | MISSING | DONE | Error reports opt-in (absent from builds without a reporting key), update check at start can be turned off (parity: Error reports and update checks) |
| Osaurus | Agent identity, encrypted relay | MISSING | OUT OF SCOPE | Team |
| OpenBot | FIFO queue: pause, resume, cancel, survives a crash | PARTIAL | DONE | Queue kept across restarts, paused and resumed, cleared |
| OpenBot | Pinned messages | MISSING | DONE | Bookmarked messages, listed per chat and in search (parity: Bookmark messages) |
| OpenBot | Audio, video, ZIP attachments | PARTIAL | DONE | Audio transcribed, ZIP read file by file, PDFs and Office read (parity: Audio and ZIP attachments), and videos (`video.rs`): six spread frames as images plus the soundtrack's transcript, with ffmpeg. Verified live: a 4-second clip showing "LISBON 2026" with a spoken question; the model read the text from the frames and answered the question from the transcript |
| OpenBot | Bubble colours from the avatar | MISSING | DONE | parity: Agent colours from the avatar (chat.ts `avatarHue`) |
| OpenBot | Backdrop | MISSING | DONE | parity: Backdrop (appearance.ts `applyBackdrop`) |
| OpenBot | Copy an error's details | PARTIAL | DONE | Copy details on a failed reply (parity: Copy error details) |
| OpenBot | Codex, Claude Code, Gemini CLIs as providers | MISSING | DONE | parity: Coding agent programs as models |
| OpenBot | ACP agents as providers | PARTIAL | PARTIAL | OpenAI-compatible and Ollama. Missing: ACP |
| OpenBot | Change provider, keep the agent | PARTIAL | DONE | An agent's model is editable (agents.rs:27–29); its notebook is kept per agent (`chat/agents/<id>/`) |
| OpenBot | Models listed for a custom provider | PARTIAL | DONE | OpenAI-compatible local servers list their loaded models (parity: Local OpenAI-compatible servers) |
| OpenBot | Agents message each other, share files | PARTIAL | PARTIAL | `delegate` and reports. Missing: messages and files between agents |
| OpenBot | Workflow canvas with approval steps | PARTIAL | PARTIAL | Step lists. Missing: canvas, approval steps |
| OpenBot | Browser control, profiles | PARTIAL | PARTIAL | Missing: an app browser the agent drives, profiles |
| OpenBot | Computer use | MISSING | DONE | parity L288 |
| OpenBot | Routines: schedule or webhook | PARTIAL | DONE | parity L291, L196 |
| OpenBot | Teach a task by recording | MISSING | DONE | parity: Teach a task by recording (`recorder.rs`, `record.ts`; Skills → Record a task). Verified live: a terminal task (cd, mkdir reports, date +%F > reports/today.txt, ls, cat) was recorded as 10 steps in 25 s with Lumo out of the way and the Recording pill on top; from the screenshots alone, the model drafted a skill with the right steps and checks, which was reviewed and saved as SKILL.md |
| OpenBot | Agents learn from work into skills | PARTIAL | PARTIAL | Notebook and `save_skill` (parity L250, L62). Missing: learning without being asked |
| OpenBot | Earlier-message lookup | MISSING | DONE | parity L271 |
| OpenBot | Marketplace | MISSING | OUT OF SCOPE | Shared marketplaces (parity L217) |
| OpenBot | Add skills from a GitHub link | MISSING | DONE | Skills → Import → From a link (parity: Skills from a link) |
| OpenBot | Vault access per agent | MISSING | DONE | parity: Vault (`vault.rs`; Settings → Permissions → Vault; agent editor → Secrets from the vault). Verified live: DEMO_TOKEN saved through Settings (value only in the keychain), granted to one agent; that agent's run_code saw it (length 20) while a plain chat's did not (False); a user tool that printed the key had it masked as [secret DEMO_TOKEN] everywhere in the saved chat |
| OpenBot | Library views, favourites | PARTIAL | DONE | Libraries: favourite libraries (★, listed first) and documents (★), a document filter and sorting by favourites, newest, name or size, remembered on this device. Verified live |
| OpenBot | Local dictation (Parakeet, Whisper) | PARTIAL | PARTIAL | Missing: a built-in model |
| OpenBot | Pause length before sending; keeps listening | PARTIAL | DONE | Pause length 0.8–4.5 s in Settings → Audio; the mic stays open for the next turn |
| OpenBot | Image and video workflow steps | PARTIAL | PARTIAL | Image through `generate_image`; video left out (parity L221) |
| OpenBot | Sort by latest activity | MISSING | DONE | parity L284 |
| OpenBot | Name and last message on hover | MISSING | DONE | main.ts:212–215 (chat rows) |
| OpenBot | Agent templates, everything exports | PARTIAL | DONE | Agent export (parity L295), export everything (settings.ts:186) |
| OpenBot | Import agents | MISSING | DONE | parity L295 |
| OpenBot | Audit log export | MISSING | OUT OF SCOPE | Team |
| OpenBot | Privacy-safe diagnostics export | PARTIAL | DONE | Export diagnostics: versions, platform, settings and counts, no content |
| OpenBot | SSO, SCIM, tenants, governance | MISSING | OUT OF SCOPE | parity L257 |
| OpenBot | Slack, Teams, mobile, servers | MISSING | OUT OF SCOPE | parity L256 |
