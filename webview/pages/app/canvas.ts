/** Canvas cards in the conversation, their editor, and quoting a selection into the composer. */
import { markdownToolbar } from './mdformat';
import { tauriInvoke } from '../../shared/tauri';
import type { Part } from './api';
import { clear, h, icon, markdown, modal, toast, clockHour12 } from './dom';
import { button, iconButton, modalHeader } from './ui';
import { improveButton } from './improve';
import { renderSheetCard } from './sheet';
import { diagramFrame, isMarp } from './diagrams';

interface Canvas {
  id: string;
  conversationId: string;
  title: string;
  versions: { content: string; author: string; at: string }[];
}

const api = {
  get: (id: string) => tauriInvoke<Canvas>('chat_get_canvas', { id }),
  save: (id: string, title: string, content: string) => tauriInvoke<Canvas>('chat_save_canvas', { id, title, content }),
  export: (id: string) => tauriInvoke<boolean>('chat_export_canvas', { id }),
};

function idOf(part: Extract<Part, { kind: 'tool' }>): string | null {
  if (typeof part.arguments.id === 'string' && part.arguments.id) return part.arguments.id;
  try {
    const out = part.output ? (JSON.parse(part.output) as { id?: string }) : null;
    return out?.id ?? null;
  } catch {
    return null;
  }
}

/** Opens an app in a window of its own, beside the main one. */
export function openAppWindow(id: string) {
  void tauriInvoke('chat_open_app_window', { id }).catch((e) => toast(String(e), 'error'));
}

/** Where a mini app is served: the `lumoapp` protocol, spelled per platform. */
export function appUrl(id: string, version = 0): string {
  const windows = navigator.userAgent.includes('Windows');
  // The trailing slash lets a multi-file app's relative links (style.css, js/app.js) resolve.
  return `${windows ? 'http://lumoapp.localhost' : 'lumoapp://localhost'}/${id}/?v=${version}`;
}

/** A sandboxed frame running a mini app: scripts run, but it has its own origin and no IPC. */
export function appFrame(id: string, version = 0): HTMLIFrameElement {
  return h('iframe', {
    class: 'app-frame',
    src: appUrl(id, version),
    sandbox: 'allow-scripts allow-forms allow-modals allow-popups',
    title: 'Mini app',
  });
}

/**
 * The side panel, as in Vibe: an app opens beside the conversation instead of inside it. The
 * open chat registers where the panel goes; cards showing the open app turn compact.
 */
type PanelHost = (content: HTMLElement | null) => void;
let panelHost: PanelHost | null = null;
let panelId: string | null = null;
/** What is open in the side panel, as tabs (Vibe opens several canvases as tabs). */
let tabs: { id: string; title: string; content: HTMLElement }[] = [];
/** Split panes: the tab shown beside the active one, when split. */
let splitId: string | null = null;

export function setPanelHost(host: PanelHost | null) {
  panelHost = host;
  panelId = null;
  splitId = null;
  tabs = [];
}

function markOpen() {
  document.querySelectorAll<HTMLElement>('.app-card[data-canvas]').forEach((card) => card.classList.toggle('compact', card.dataset.canvas === panelId));
}

/** Draws the active tab, with the tab strip when more than one thing is open. */
function showPanel() {
  if (!panelHost) return;
  const active = tabs.find((t) => t.id === panelId);
  if (!active) {
    panelHost(null);
    markOpen();
    return;
  }
  const beside = splitId && splitId !== panelId ? tabs.find((t) => t.id === splitId) : undefined;
  if (!beside) splitId = null;
  const splitBtn = h('button', { class: `panel-tab panel-split-btn${beside ? ' active' : ''}`, title: beside ? 'Show one tab' : 'Split: show two tabs side by side' }, icon('columns', 14));
  splitBtn.addEventListener('click', () => {
    // Beside the active tab goes the one used before it.
    splitId = beside ? null : (tabs.filter((t) => t.id !== panelId).pop()?.id ?? null);
    showPanel();
  });
  const strip =
    tabs.length > 1
      ? h(
          'div',
          { class: 'panel-tabs' },
          tabs.map((t) => {
            const close = h('span', { class: 'panel-tab-close', title: 'Close tab' }, icon('close', 12));
            close.addEventListener('click', (e) => {
              e.stopPropagation();
              closeTab(t.id);
            });
            const where = t.id === panelId ? ' active' : t.id === splitId ? ' beside' : '';
            return h('button', { class: `panel-tab${where}`, title: t.title, on: { click: () => {
              // Split: a tab clicked goes in the pane it is not already in.
              if (splitId && t.id !== panelId && t.id !== splitId) splitId = t.id;
              else if (t.id === splitId) [panelId, splitId] = [splitId, panelId];
              else panelId = t.id;
              showPanel();
            } } }, h('span', { class: 'panel-tab-title' }, t.title), close);
          }),
          h('span', { class: 'spacer' }),
          splitBtn
        )
      : null;
  const body = beside ? h('div', { class: 'panel-split' }, h('div', { class: 'panel-pane' }, active.content), h('div', { class: 'panel-pane' }, beside.content)) : active.content;
  panelHost(h('div', { class: `panel-shell${beside ? ' split' : ''}` }, strip, body));
  document.querySelector('.chat')?.classList.toggle('panel-split-wide', Boolean(beside));
  markOpen();
}

function closeTab(id: string) {
  const i = tabs.findIndex((t) => t.id === id);
  if (i < 0) return;
  tabs.splice(i, 1);
  if (splitId === id) splitId = null;
  if (panelId === id) {
    panelId = splitId ?? tabs[Math.min(i, tabs.length - 1)]?.id ?? null;
    if (panelId === splitId) splitId = null;
  }
  showPanel();
}

/** Whether a chat is showing, with a side panel to open things in. */
export function hasPanel(): boolean {
  return panelHost !== null;
}

/** Shows `content` in the side panel as a tab; `id` marks what is open there. */
export function openSidePanel(id: string, content: HTMLElement, title = 'Panel') {
  if (!panelHost) return;
  const existing = tabs.find((t) => t.id === id);
  if (existing) existing.content = content;
  else tabs.push({ id, title, content });
  panelId = id;
  showPanel();
}

/** Closes the tab in front; the panel closes with the last one. */
export function closePanel() {
  if (panelId) closeTab(panelId);
  else showPanel();
}

interface AppInfo {
  id: string;
  title: string;
  code: string;
  version: number;
}

/** Preview and code, switched by the two icon buttons. */
/** A multi-file app's files (Vibe's file list), or `null` for a one-file app. */
export function appFiles(code: string): { path: string; content: string }[] | null {
  if (!code.trimStart().startsWith('{')) return null;
  try {
    const files = (JSON.parse(code) as { lumoAppFiles?: { path: string; content: string }[] }).lumoAppFiles;
    return Array.isArray(files) && files.length ? files : null;
  } catch {
    return null;
  }
}

/** An app's code as text to copy: one file as it is, several each under its path. */
export function appCodeText(code: string): string {
  const files = appFiles(code);
  return files ? files.map((f) => `// ${f.path}\n${f.content}`).join('\n\n') : code;
}

/** The code view: the file list and the chosen file for a multi-file app, else the page. */
function codeView(code: string): HTMLElement {
  const files = appFiles(code);
  if (!files) return h('pre', { class: 'step-pre app-code' }, code);
  const pre = h('pre', { class: 'step-pre app-code' });
  const list = h('div', { class: 'app-file-list' });
  const pick = (i: number) => {
    pre.textContent = files[i]!.content;
    list.querySelectorAll('button').forEach((b, j) => b.classList.toggle('active', i === j));
  };
  files.forEach((f, i) => list.append(h('button', { class: 'app-file', title: f.path, on: { click: () => pick(i) } }, icon('file', 13), h('span', {}, f.path))));
  pick(0);
  return h('div', { class: 'app-code-files' }, list, pre);
}

function viewSwitch(app: AppInfo, body: HTMLElement, frameClass = '') {
  const code = iconButton({ icon: 'code', title: 'Code', size: 'sm' });
  const eye = iconButton({ icon: 'eye', title: 'Preview', size: 'sm' });
  const show = (preview: boolean) => {
    eye.classList.toggle('active', preview);
    code.classList.toggle('active', !preview);
    const frame = appFrame(app.id, app.version);
    if (frameClass) frame.classList.add(frameClass);
    clear(body, preview ? frame : codeView(app.code));
  };
  code.addEventListener('click', () => show(false));
  eye.addEventListener('click', () => show(true));
  show(true);
  return h('div', { class: 'view-switch' }, code, eye);
}

function download(id: string) {
  void api.export(id).then((saved) => saved && toast('App saved.')).catch((e) => toast(String(e), 'error'));
}

export function openAppPanel(app: AppInfo) {
  if (!panelHost) return;
  const body = h('div', { class: 'panel-body' });
  const close = iconButton({ icon: 'close', title: 'Close', size: 'sm' });
  close.addEventListener('click', closePanel);
  const copy = iconButton({ icon: 'copy', title: 'Copy code', size: 'sm' });
  copy.addEventListener('click', () => { void navigator.clipboard.writeText(appCodeText(app.code)); toast('Code copied.'); });
  const full = iconButton({ icon: 'expand', title: 'Full screen', size: 'sm' });
  full.addEventListener('click', () => void modal((done) => h('div', { class: 'app-modal' }, modalHeader(app.title, done), appFrame(app.id, app.version)), true));
  const windowed = iconButton({ icon: 'share', title: 'Open in a window of its own', size: 'sm' });
  windowed.addEventListener('click', () => openAppWindow(app.id));
  const save = iconButton({ icon: 'download', title: 'Download as a web page', size: 'sm' });
  save.addEventListener('click', () => download(app.id));
  const shared = iconButton({ icon: 'link', title: 'Share a link on your network', size: 'sm' });
  shared.addEventListener('click', () => void import('./share').then((s) => s.shareLink(app.title, s.appPage(app.code, app.title), true)));
  openSidePanel(
    app.id,
    h(
      'div',
      { class: 'panel-inner' },
      h(
        'div',
        { class: 'panel-head' },
        h('div', { class: 'panel-tools' }, close, viewSwitch(app, body, 'panel-frame')),
        h('strong', { class: 'panel-title' }, app.title),
        h('span', { class: 'spacer' }),
        h('div', { class: 'panel-tools' }, copy, full, windowed, shared, save)
      ),
      body
    ),
    app.title
  );
}

/** A mini app the model built: a preview in the reply that opens in the side panel. */
function renderAppCard(part: Extract<Part, { kind: 'tool' }>, live: boolean): HTMLElement {
  const title = typeof part.arguments.title === 'string' && part.arguments.title ? part.arguments.title : 'App';
  // A multi-file app sends `files`; it is kept as the same bundle the app is stored as.
  const code = Array.isArray(part.arguments.files)
    ? JSON.stringify({ lumoAppFiles: part.arguments.files })
    : typeof part.arguments.content === 'string'
      ? part.arguments.content
      : '';
  const id = idOf(part);
  let version = 0;
  try {
    version = part.output ? ((JSON.parse(part.output) as { version?: number }).version ?? 0) : 0;
  } catch {
    version = 0;
  }
  const created = typeof part.arguments.id !== 'string' || !part.arguments.id;
  const label = h('div', { class: 'canvas-label' }, live && part.output === null ? 'Building an app…' : created ? 'Created an app' : 'Edited an app');
  if (!id || !part.output) {
    return h('div', { class: 'canvas-step' }, label, h('div', { class: 'canvas-card app-card' }, h('div', { class: 'status-line app-wait' }, h('span', { class: 'dots' }), `Building ${title}…`)));
  }
  const app: AppInfo = { id, title, code, version };
  const open = () => openAppPanel(app);
  // Compact: the chip shown while the app is open in the panel.
  const chip = h('button', { class: 'app-chip', on: { click: open } }, h('span', { class: 'app-chip-icon' }, icon('react', 22)), h('span', {}, h('strong', {}, title), h('small', {}, 'App')));
  const body = h('div', { class: 'app-body' });
  const save = iconButton({ icon: 'download', title: 'Download as a web page', size: 'sm' });
  save.addEventListener('click', () => download(id));
  const shared = iconButton({ icon: 'link', title: 'Share a link on your network', size: 'sm' });
  shared.addEventListener('click', () => void import('./share').then((s) => s.shareLink(title, s.appPage(code, title), true)));
  const expand = h('button', { class: 'canvas-expand', on: { click: open } }, 'Expand');
  const card = h(
    'div',
    { class: `canvas-card app-card${panelId === id ? ' compact' : ''}`, 'data-canvas': id },
    chip,
    h('div', { class: 'canvas-head' }, icon('react', 16), h('span', { class: 'canvas-title' }, title), h('span', { class: 'spacer' }), viewSwitch(app, body), shared, save),
    body,
    expand
  );
  return h('div', { class: 'canvas-step' }, label, card);
}

/** A canvas the model created or updated, shown as a document card in the reply. */
export function renderCanvasCard(part: Extract<Part, { kind: 'tool' }>, live: boolean): HTMLElement {
  if (part.arguments.kind === 'app') return renderAppCard(part, live);
  if (part.arguments.kind === 'sheet') return renderSheetCard(part, live);
  const created = typeof part.arguments.id !== 'string' || !part.arguments.id;
  let title = typeof part.arguments.title === 'string' && part.arguments.title ? part.arguments.title : 'Untitled';
  let content = typeof part.arguments.content === 'string' ? part.arguments.content : '';
  const writing = live && part.output === null;
  const body = h('div', { class: 'markdown canvas-body' });
  body.append(canvasView(content));
  const titleEl = h('span', { class: 'canvas-title' }, title);
  const card = h('div', { class: 'canvas-card' });

  const copy = button({ variant: 'subtle', size: 'sm', icon: 'copy', label: 'Copy', title: 'Copy' });
  copy.addEventListener('click', () => {
    void navigator.clipboard.writeText(content);
    toast('Canvas copied.');
  });
  const download = iconButton({ icon: 'download', title: 'Download as Markdown', size: 'sm' });
  download.addEventListener('click', () => {
    const id = idOf(part);
    if (id) void api.export(id).then((saved) => saved && toast('Canvas saved.')).catch((e) => toast(String(e), 'error'));
  });
  const edit = button({ variant: 'subtle', size: 'sm', icon: 'edit', label: 'Edit', title: 'Edit' });
  edit.addEventListener('click', async () => {
    const id = idOf(part);
    if (!id) return;
    // Edit the newest version, which may hold the user's or a later reply's changes.
    const latest = await api.get(id).catch(() => null);
    const current = latest?.versions[latest.versions.length - 1]?.content ?? content;
    void openEditor(latest?.title ?? title, current, async (newTitle, newContent) => {
      const saved = await api.save(id, newTitle, newContent);
      title = saved.title;
      content = newContent;
      titleEl.textContent = title;
      body.replaceChildren(canvasView(content));
      toast('Saved. The assistant sees your edit on its next reply.');
    });
  });
  // History: step through versions, see what changed, restore one (Vibe's canvas versions).
  const historyBar = h('div', { class: 'canvas-history' });
  const history = button({ variant: 'subtle', size: 'sm', icon: 'clock', label: 'History', title: 'Versions and changes' });
  history.addEventListener('click', async () => {
    if (historyBar.childElementCount) {
      clear(historyBar);
      body.replaceChildren(canvasView(content));
      return;
    }
    const id = idOf(part);
    const canvas = id ? await api.get(id).catch(() => null) : null;
    if (!id || !canvas || !canvas.versions.length) return toast('No earlier versions yet.');
    let at = canvas.versions.length - 1;
    let showChanges = false;
    const render = () => {
      const version = canvas.versions[at]!;
      const previous = canvas.versions[at - 1];
      const prev = iconButton({ icon: 'back', title: 'Older version', size: 'sm', onClick: () => { at -= 1; render(); } });
      const next = iconButton({ icon: 'forward', title: 'Newer version', size: 'sm', onClick: () => { at += 1; render(); } });
      prev.disabled = at === 0;
      next.disabled = at === canvas.versions.length - 1;
      const changes = button({ variant: showChanges ? 'secondary' : 'subtle', size: 'sm', label: 'Changes', onClick: () => { showChanges = !showChanges; render(); } });
      changes.disabled = !previous;
      const restore = at < canvas.versions.length - 1
        ? button({
            variant: 'secondary',
            size: 'sm',
            icon: 'reset',
            label: 'Restore',
            onClick: async () => {
              const saved = await api.save(id, canvas.title, version.content);
              canvas.versions = saved.versions;
              content = version.content;
              at = canvas.versions.length - 1;
              showChanges = false;
              render();
              toast('Restored as a new version. The assistant sees it on its next reply.');
            },
          })
        : null;
      const who = version.author === 'user' ? 'you' : 'the assistant';
      clear(
        historyBar,
        prev,
        h('span', { class: 'history-label' }, `Version ${at + 1} of ${canvas.versions.length}`, h('small', { class: 'muted' }, ` · by ${who}, ${new Date(version.at).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', hour12: clockHour12() })}`)),
        next,
        h('span', { class: 'spacer' }),
        changes,
        restore
      );
      body.replaceChildren(showChanges && previous ? diffView(previous.content, version.content) : canvasView(version.content));
      card.classList.add('expanded');
      expand.textContent = 'Collapse';
    };
    render();
  });
  const expand = h('button', { class: 'canvas-expand' }, 'Expand');
  expand.addEventListener('click', () => {
    const open = card.classList.toggle('expanded');
    expand.textContent = open ? 'Collapse' : 'Expand';
  });

  // Preview or the Markdown source, as the user chooses.
  const previewBtn = iconButton({ icon: 'eye', title: 'Preview', size: 'sm', active: true });
  const codeBtn = iconButton({ icon: 'code', title: 'Markdown source', size: 'sm' });
  const showSource = (source: boolean) => {
    previewBtn.classList.toggle('active', !source);
    codeBtn.classList.toggle('active', source);
    clear(historyBar);
    body.replaceChildren(source ? h('pre', { class: 'step-pre canvas-source' }, content) : canvasView(content));
  };
  previewBtn.addEventListener('click', () => showSource(false));
  codeBtn.addEventListener('click', () => showSource(true));
  card.append(
    h('div', { class: 'canvas-head' }, icon('file', 16), titleEl, h('div', { class: 'view-switch' }, previewBtn, codeBtn), h('span', { class: 'spacer' }), writing ? h('span', { class: 'dots' }) : null, history, edit, copy, download),
    historyBar,
    body,
    expand
  );
  return h('div', { class: 'canvas-step' }, h('div', { class: 'canvas-label' }, writing ? 'Writing a canvas…' : created ? 'Created a canvas' : 'Edited a canvas'), card);
}

function openEditor(title: string, content: string, onSave: (title: string, content: string) => Promise<void>): Promise<void> {
  return modal((close) => {
    const titleInput = h('input', { class: 'input canvas-title-input', value: title });
    const area = h('textarea', { class: 'input textarea mono canvas-editor' });
    area.value = content;
    const preview = h('div', { class: 'markdown canvas-preview' });
    const renderPreview = () => preview.replaceChildren(markdown(area.value));
    area.addEventListener('input', renderPreview);
    renderPreview();
    const save = async () => {
      try {
        await onSave(titleInput.value, area.value);
        close();
      } catch (e) {
        toast(String(e), 'error');
      }
    };
    area.addEventListener('keydown', (e) => {
      if ((e.ctrlKey || e.metaKey) && e.key === 's') {
        e.preventDefault();
        void save();
      }
    });
    return h(
      'div',
      { class: 'canvas-modal' },
      modalHeader(titleInput, close),
      markdownToolbar(area, renderPreview),
      h('div', { class: 'canvas-split' }, area, preview),
      h('div', { class: 'dialog-actions canvas-actions' }, improveButton(area, renderPreview), h('span', { class: 'hint' }, 'Markdown · Ctrl+S saves a new version'), h('span', { class: 'spacer' }), h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'), h('button', { class: 'btn btn-primary', on: { click: () => void save() } }, 'Save'))
    );
  }, true);
}

/**
 * Selecting text in the conversation offers "Ask about this", which quotes the selection into
 * the composer. `scope` is the scrolling transcript.
 */
export function enableAskAboutSelection(scope: HTMLElement, onAsk: (quote: string) => void) {
  let button: HTMLElement | null = null;
  const hide = () => {
    button?.remove();
    button = null;
  };
  scope.addEventListener('mouseup', () => {
    setTimeout(() => {
      const selection = window.getSelection();
      const text = selection?.toString().trim() ?? '';
      if (!selection || selection.rangeCount === 0 || text.length < 3 || !scope.contains(selection.anchorNode)) {
        hide();
        return;
      }
      const rect = selection.getRangeAt(0).getBoundingClientRect();
      hide();
      button = h('button', { class: 'ask-about' }, icon('send', 14), 'Ask about this');
      button.style.left = `${Math.max(8, rect.left)}px`;
      button.style.top = `${Math.max(8, rect.top - 40)}px`;
      button.addEventListener('mousedown', (e) => {
        e.preventDefault();
        onAsk(text);
        selection.removeAllRanges();
        hide();
      });
      document.body.append(button);
    });
  });
  scope.addEventListener('scroll', hide);
  document.addEventListener('mousedown', (e) => {
    if (button && e.target !== button && !button.contains(e.target as Node)) hide();
  });
}

/** Lines removed and added between two versions, in order (a longest-common-subsequence diff). */
export function lineDiff(before: string, after: string): { kind: ' ' | '-' | '+'; text: string }[] {
  const a = before.split('\n');
  const b = after.split('\n');
  // Too large to compare line by line: show the whole change as replace.
  if (a.length * b.length > 4_000_000) return [...a.map((t) => ({ kind: '-' as const, text: t })), ...b.map((t) => ({ kind: '+' as const, text: t }))];
  const n = a.length;
  const m = b.length;
  const lcs: Uint32Array[] = Array.from({ length: n + 1 }, () => new Uint32Array(m + 1));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) lcs[i]![j] = a[i] === b[j] ? lcs[i + 1]![j + 1]! + 1 : Math.max(lcs[i + 1]![j]!, lcs[i]![j + 1]!);
  }
  const out: { kind: ' ' | '-' | '+'; text: string }[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      out.push({ kind: ' ', text: a[i]! });
      i++;
      j++;
    } else if (lcs[i + 1]![j]! >= lcs[i]![j + 1]!) out.push({ kind: '-', text: a[i++]! });
    else out.push({ kind: '+', text: b[j++]! });
  }
  while (i < n) out.push({ kind: '-', text: a[i++]! });
  while (j < m) out.push({ kind: '+', text: b[j++]! });
  return out;
}

/** The change view: added lines green, removed lines struck through, unchanged runs folded. */
function diffView(before: string, after: string): HTMLElement {
  const lines = lineDiff(before, after);
  const el = h('div', { class: 'canvas-diff' });
  let same: string[] = [];
  const flushSame = () => {
    if (same.length > 6) {
      same.slice(0, 2).forEach((t) => el.append(h('div', { class: 'diff-line same' }, t || ' ')));
      el.append(h('div', { class: 'diff-fold muted' }, `… ${same.length - 4} unchanged lines`));
      same.slice(-2).forEach((t) => el.append(h('div', { class: 'diff-line same' }, t || ' ')));
    } else same.forEach((t) => el.append(h('div', { class: 'diff-line same' }, t || ' ')));
    same = [];
  };
  for (const line of lines) {
    if (line.kind === ' ') {
      same.push(line.text);
      continue;
    }
    flushSame();
    el.append(h('div', { class: `diff-line ${line.kind === '+' ? 'added' : 'removed'}` }, line.text || ' '));
  }
  flushSame();
  const added = lines.filter((l) => l.kind === '+').length;
  const removed = lines.filter((l) => l.kind === '-').length;
  const count = (n: number) => `${n} ${n === 1 ? 'line' : 'lines'}`;
  return h('div', {}, h('p', { class: 'muted small' }, `${count(added)} added, ${count(removed)} removed`), el);
}

/** A document canvas as shown: Marp decks as slides, everything else as Markdown. */
function canvasView(content: string): Node {
  return isMarp(content) ? diagramFrame(content, 'marp') : markdown(content, { breaks: true });
}
