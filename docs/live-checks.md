# Live checks for the integrations tested against stand-ins

These integrations were tested against local stand-ins for their services, because no
accounts were available where they were built. Two of them now run against the real
servers in CI on every change (the Live job in `.github/workflows/ci.yml`), and pass there:
Qdrant 1.12.4 and Apache Tika 4.2.0. The checks below for them remain useful on your own
computer, with your own documents. Each check below takes a few minutes with a real
account. Note what you see; if a step fails, Settings → General → Error reports and updates → Export diagnostics saves the app log, which says why.

## Telegram

1. In Telegram, talk to @BotFather: `/newbot`, copy the token.
2. Cortex → Settings → General → Telegram: paste the token, Connect. Expected: "Connected as @yourbot".
3. Turn on answering, then Pair my phone; send the `/pair 123456` line shown to the bot.
   Expected: "Paired".
4. Send "What is 17 × 23?". Expected: the answer arrives in Telegram, and a "Telegram" chat appears in Cortex.
5. Send `/new`, then a question that needs the earlier one. Expected: it no longer remembers.

## Slack

1. api.slack.com/apps → Create New App → From scratch.
   - Socket Mode: on. Create an app-level token (`xapp-…`) with the `connections:write` scope.
   - OAuth & Permissions → Bot scopes: `chat:write`, `im:history`, `app_mentions:read`.
   - Event Subscriptions: on. Subscribe to the bot events `message.im` and `app_mention`.
   - Install the app to the workspace and copy the bot token (`xoxb-…`).
2. Cortex → Settings → General → Slack: paste both tokens, then Connect. Expected: "Connected to <workspace>".
3. Turn on Answer messages, then Pair my Slack account. Send `pair <code>` to the app in a DM.
4. Ask a question in the DM. Expected: an answer appears.
5. Invite the app to a channel and mention it with a question. Expected: an answer in the thread.

## Discord

1. discord.com/developers → New Application → Bot: copy the token (Reset Token).
2. OAuth2 → URL Generator: scope `bot`, permissions Send Messages and Read Message History.
   Open the URL and add the bot to a server.
3. Cortex → Settings → General → Discord: paste the token and Connect.
   Expected: "Connected as <bot>". A wrong token is refused.
4. Turn on Answer messages, then Pair my Discord account. DM the bot `pair <code>`.
5. In a server channel, write `@bot what is the capital of Portugal?`.
   Expected: a reply to your message. A message without the mention is ignored.

## Video generation

1. Settings → Providers: add a provider with video models, e.g. xAI with an API key.
2. Settings → Providers → Images, video and voice: set the video model (e.g. `xai/grok-imagine-video`).
3. In a chat: "Make a 5-second clip of waves at sunset."
   Expected: progress, then a playable video in the reply and in Outputs.

## Qdrant

1. Run Qdrant, e.g. `docker run -p 6333:6333 qdrant/qdrant`.
2. Settings → Providers → Built-in models: download MiniLM L6 (embeddings).
3. Settings → Providers → Semantic search:
   - choose Built-in (MiniLM) and turn on Use semantic search;
   - set Vector store to Qdrant at `http://localhost:6333`, then Check server. Expected: "Connected: Qdrant 1.x".
4. Embed documents now. Expected: the collection `cortex_minilm_l6` appears in Qdrant's dashboard (`:6333/dashboard`).
5. Ask in a chat with a library: a question that shares no words with the answer (e.g. "vacation days" for a "holidays" passage).
   Expected: that passage is cited.
6. Delete that document. Embed documents now again. Expected: its points are gone from the collection.

## Tika

1. Run Tika, e.g. `docker run -p 9998:9998 apache/tika:latest-full`.
2. Settings → Providers → Document extraction: choose Tika at `http://localhost:9998` and save.
3. Add a scanned PDF (an image of text) to a library.
   Expected: its text is readable in the document preview (OCR). With Tika stopped, the built-in reader takes over.

## ACP agent

1. Install an ACP agent, e.g. `npm i -g @zed-industries/claude-code-acp`, with `ANTHROPIC_API_KEY` set.
2. Restart Cortex. The model menu shows "Claude Code (ACP)" under ACP agents.
3. In Code mode, in a project folder: "List the files here and summarize the README."
   Expected: its thoughts and tool calls stream in, and its permission requests appear as approval cards.

## PR status

1. Install the GitHub CLI (`gh`) and sign in with `gh auth login`.
2. Open a Code mode chat in a repository whose current branch has an open pull request.
   Expected: the PR bar shows its number, title, review state and checks. Opening it goes to the pull request.
