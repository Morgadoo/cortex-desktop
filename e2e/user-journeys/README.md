# User journeys

The real app, a real model, driven like a user would: through `tauri-driver` (WebDriver) under a
virtual display. Unlike `e2e/` above it, which checks the hosted bridge contract, these check what a
person sees in the native app.

```sh
npm run build:webview && (cd src-tauri && cargo build)
CORTEX_TEST_PROVIDERS=my-providers.json DEEPSEEK_API_KEY=… e2e/user-journeys/run.sh all
```

| Journey | Checks |
|---|---|
| `checklist` (light and dark) | Send and stream; stop mid-answer; regenerate (or Answer now when stopped before any text); edit a prompt; the `/`, `@`, `#` and `:` menus; Shift+Tab modes; Alt+Enter queue; the Sources panel; the six settings sections; the pages; Ctrl+K search; switching the interface language and back with no English left |
| `multitask` | An answer keeps streaming after you leave its chat, and shows its progress in the sidebar; coming back shows the text so far, still growing; an unsent draft survives switching away; two chats answer at once; a chat that finishes while you are elsewhere is marked unread; after the app is killed mid-answer, the chat shows what had streamed and Continue finishes it |
| `adapt` | Cortex adapts and grows with its user. What you tell it about yourself in one chat ("I'm vegetarian, I live in Porto") is saved as facts, listed under Context → Knowledge → Facts where you can edit or delete them, and used by a new chat that was never told (a meat-free dinner, a market in Porto). Then, with the three learning switches turned on on the Knowledge page: a ready-made agent's repeated procedure (a weekly price check) is kept as a skill, off until reviewed, and the chat says so; the weekly review finds the fact said twice and proposes forgetting one, changing nothing until asked; the daily evals score the starter suite on your model |

`my-providers.json` is a `chat/providers.json` with the provider enabled and `defaultModel` set,
for example `{"providers":{"deepseek":{"enabled":true}},"defaultModel":"deepseek/deepseek-flash"}`;
the key is read from the provider's environment variable. Each journey starts from an empty
profile. Results (`pass`/`FAIL` per step) and a screenshot after each step go to `out/`.

What a model writes varies, so a step that depends on its timing (stopping mid-answer, the kill
during streaming) can need a second run; the script waits for the state it needs rather than for
fixed times where it can.
