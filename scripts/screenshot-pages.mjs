#!/usr/bin/env node
//
// Screenshots of the local pages, in both themes, with Tauri's invoke stubbed by sample data, for
// checking that a stylesheet change looks the same before and after (rule order decides which
// rule wins, so a reorder or a merge can change a page nobody looked at).
//
//   npm run build:webview && node scripts/screenshot-pages.mjs shots/before
//   … change the stylesheets …
//   npm run build:webview && node scripts/screenshot-pages.mjs shots/after
//   node scripts/screenshot-pages.mjs --compare shots/before shots/after
//
// --compare lists the screenshots that differ and where. A few pixels in the same place in two
// runs of an unchanged build (a spinner, a caret) are noise: shoot "before" twice to see them.
//
// Needs Playwright with Chromium (`npm i -D playwright && npx playwright install chromium`); it
// is not a dependency of the project, as only stylesheet work needs it.

import { mkdirSync, readdirSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const dist = join(repoRoot, 'src-tauri', 'dist');

let chromium;
try {
  ({ chromium } = createRequire(import.meta.url)('playwright'));
} catch {
  console.error('screenshot-pages: Playwright is not installed (npm i -D playwright)');
  process.exit(1);
}

const NOW = '2026-01-15T10:00:00Z';
const summary = (id, title, extra = {}) => ({
  id,
  title,
  projectId: null,
  workdir: null,
  agentId: null,
  taskId: null,
  unread: false,
  pinned: false,
  archived: false,
  tags: [],
  createdAt: NOW,
  updatedAt: NOW,
  tokens: 0,
  cost: 0,
  model: null,
  preview: '',
  bookmarks: [],
  ...extra,
});

// What the stubbed commands answer; any other command gets [] when its name reads as a list,
// else null.
const RESPONSES = {
  chat_list_conversations: [summary('c1', 'Planning the garden', { pinned: true, tags: ['home'] }), summary('c2', 'Rust error handling')],
  chat_get_conversation: {
    id: 'c1',
    running: false,
    usage: { inputTokens: 1200, outputTokens: 800, cost: 0.01, contextTokens: 2000 },
    compactedBefore: null,
    title: 'Planning the garden',
    projectId: null,
    workdir: null,
    agentId: null,
    model: 'openai/gpt-x',
    pinned: true,
    archived: false,
    params: {},
    bookmarks: [],
    versions: [],
    turns: [
      { role: 'user', parts: [{ kind: 'text', text: 'Help me plan a small vegetable garden.' }], index: 0, time: NOW, rating: 0 },
      {
        role: 'assistant',
        parts: [
          { kind: 'reasoning', text: 'Thinking about sun and space.' },
          { kind: 'tool', callId: 't1', name: 'web_search', arguments: { query: 'garden' }, output: 'results', isError: false },
          {
            kind: 'text',
            text: '## A plan\n\n1. **Tomatoes** in full sun\n2. Lettuce in partial shade\n\n```python\nprint("water daily")\n```\n\n> Keep the soil moist.\n\n| Plant | Spacing |\n|---|---|\n| Tomato | 60 cm |',
          },
        ],
        index: 1,
        time: NOW,
        rating: 1,
      },
    ],
  },
  chat_list_models: {
    models: [{ reference: 'openai/gpt-x', vendor: 'openai', providerLabel: 'OpenAI', id: 'gpt-x', name: 'GPT X', reasoning: true, tools: true, contextWindow: 128000 }],
    defaultModel: 'openai/gpt-x',
  },
  chat_get_instructions: { tone: '', custom: '', name: 'Alex Doe' },
  chat_emoji_status: { state: 'missing' },
  chat_emoji_install: { state: 'failed' },
  chat_take_link: null,
};

const STUB = `(() => {
  const R = ${JSON.stringify(RESPONSES)};
  window.__TAURI_INTERNALS__ = {
    invoke: (cmd) => Promise.resolve(cmd in R ? JSON.parse(JSON.stringify(R[cmd])) : /^chat_(list|search)|s$/.test(cmd) ? [] : null),
    transformCallback: () => 0,
  };
})();`;

const click = (selector) => async (page) => {
  await page.click(selector).catch(() => {});
};
// [name, page, what to do before the screenshot]
const VIEWS = [
  ['app', 'index.html', null],
  ['app-chat', 'index.html', click('text=Planning the garden')],
  ['app-scheduled', 'index.html', click('text=Scheduled')],
  ['app-context', 'index.html', click('text=Context')],
  ['app-apps', 'index.html', click('text=Apps')],
  ['app-agents', 'index.html', click('button:has-text("Agents")')],
  ['app-code', 'index.html', click('button:has-text("Code")')],
  ['app-user-menu', 'index.html', click('text=Alex Doe')],
  ['app-plus', 'index.html', click('button[title="Add"]')],
  ['app-settings', 'index.html', (page) => page.keyboard.press('Control+,')],
  [
    'app-settings-appearance',
    'index.html',
    async (page) => {
      await page.keyboard.press('Control+,');
      await page.waitForTimeout(500);
      await click('text=Appearance')(page);
    },
  ],
  ['settings', 'settings.html', null],
  ['search', 'search.html', null],
  ['login', 'login.html', null],
  ['dictate', 'dictate.html', null],
  ['record', 'record.html', null],
];

async function shoot(out) {
  mkdirSync(out, { recursive: true });
  const browser = await chromium.launch();
  for (const theme of ['dark', 'light']) {
    for (const [name, file, act] of VIEWS) {
      const context = await browser.newContext({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1, colorScheme: theme, reducedMotion: 'reduce' });
      await context.clock.setFixedTime(new Date(NOW));
      await context.addInitScript(STUB);
      const page = await context.newPage();
      const setTheme = () => page.evaluate((t) => document.documentElement.setAttribute('data-theme', t), theme);
      await page.goto(`file://${join(dist, file)}`);
      await page.waitForTimeout(800);
      await setTheme();
      if (act) {
        await act(page);
        await page.waitForTimeout(800);
        await setTheme();
      }
      await page.addStyleTag({ content: '*{animation:none!important;transition:none!important;caret-color:transparent!important}' });
      await page.waitForTimeout(200);
      await page.screenshot({ path: join(out, `${name}-${theme}.png`), fullPage: true });
      await context.close();
    }
  }
  await browser.close();
  console.log(`screenshot-pages: ${VIEWS.length * 2} screenshots in ${out}`);
}

async function compare(a, b) {
  const browser = await chromium.launch();
  const page = await browser.newPage();
  let differ = 0;
  const files = readdirSync(a).filter((f) => f.endsWith('.png'));
  for (const file of files) {
    const urls = [a, b].map((dir) => `data:image/png;base64,${readFileSync(join(dir, file)).toString('base64')}`);
    const result = await page.evaluate(async ([x, y]) => {
      const load = (src) =>
        new Promise((ok, fail) =>
          Object.assign(new Image(), {
            onload() {
              ok(this);
            },
            onerror: fail,
            src,
          })
        );
      const [ia, ib] = await Promise.all([load(x), load(y)]);
      if (ia.width !== ib.width || ia.height !== ib.height) return { size: `${ia.width}x${ia.height} / ${ib.width}x${ib.height}` };
      const pixels = (img) => {
        const canvas = Object.assign(document.createElement('canvas'), { width: img.width, height: img.height });
        const ctx = canvas.getContext('2d');
        ctx.drawImage(img, 0, 0);
        return ctx.getImageData(0, 0, img.width, img.height).data;
      };
      const [pa, pb] = [pixels(ia), pixels(ib)];
      let count = 0;
      let box = null;
      for (let i = 0; i < pa.length; i += 4) {
        if (pa[i] === pb[i] && pa[i + 1] === pb[i + 1] && pa[i + 2] === pb[i + 2]) continue;
        count++;
        const px = (i / 4) % ia.width;
        const py = Math.floor(i / 4 / ia.width);
        box = box ? [Math.min(box[0], px), Math.min(box[1], py), Math.max(box[2], px), Math.max(box[3], py)] : [px, py, px, py];
      }
      return { count, box };
    }, urls);
    if (result.size) console.log(`${file}: sizes differ (${result.size})`);
    else if (result.count) console.log(`${file}: ${result.count} pixels differ in ${result.box.join(',')}`);
    else continue;
    differ++;
  }
  await browser.close();
  console.log(`screenshot-pages: ${files.length} compared, ${differ} differ`);
}

const args = process.argv.slice(2);
if (args[0] === '--compare' && args.length === 3) await compare(resolve(args[1]), resolve(args[2]));
else if (args.length === 1 && !args[0].startsWith('-')) await shoot(resolve(args[0]));
else {
  console.error('usage: node scripts/screenshot-pages.mjs <out-dir> | --compare <dir-a> <dir-b>');
  process.exit(1);
}
