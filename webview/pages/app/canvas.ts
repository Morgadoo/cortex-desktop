/** Canvas cards in the conversation, their editor, and quoting a selection into the composer. */
import { tauriInvoke } from '../../shared/tauri';
import type { Part } from './api';
import { clear, h, icon, markdown, modal, toast } from './dom';
import { button, iconButton, modalHeader } from './ui';
import { renderSheetCard } from './sheet';

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

/** Where a mini app is served: the `lumoapp` protocol, spelled per platform. */
export function appUrl(id: string, version = 0): string {
  const windows = navigator.userAgent.includes('Windows');
  return `${windows ? 'http://lumoapp.localhost' : 'lumoapp://localhost'}/${id}?v=${version}`;
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

export function setPanelHost(host: PanelHost | null) {
  panelHost = host;
  panelId = null;
}

function markOpen() {
  document.querySelectorAll<HTMLElement>('.app-card[data-canvas]').forEach((card) => card.classList.toggle('compact', card.dataset.canvas === panelId));
}

export function closePanel() {
  panelId = null;
  panelHost?.(null);
  markOpen();
}

interface AppInfo {
  id: string;
  title: string;
  code: string;
  version: number;
}

/** Preview and code, switched by the two icon buttons. */
function viewSwitch(app: AppInfo, body: HTMLElement, frameClass = '') {
  const code = iconButton({ icon: 'code', title: 'Code', size: 'sm' });
  const eye = iconButton({ icon: 'eye', title: 'Preview', size: 'sm' });
  const show = (preview: boolean) => {
    eye.classList.toggle('active', preview);
    code.classList.toggle('active', !preview);
    const frame = appFrame(app.id, app.version);
    if (frameClass) frame.classList.add(frameClass);
    clear(body, preview ? frame : h('pre', { class: 'step-pre app-code' }, app.code));
  };
  code.addEventListener('click', () => show(false));
  eye.addEventListener('click', () => show(true));
  show(true);
  return h('div', { class: 'view-switch' }, code, eye);
}

function download(id: string) {
  void api.export(id).then((saved) => saved && toast('App saved as a web page.')).catch((e) => toast(String(e), 'error'));
}

export function openAppPanel(app: AppInfo) {
  if (!panelHost) return;
  const body = h('div', { class: 'panel-body' });
  const close = iconButton({ icon: 'close', title: 'Close', size: 'sm' });
  close.addEventListener('click', closePanel);
  const copy = iconButton({ icon: 'copy', title: 'Copy code', size: 'sm' });
  copy.addEventListener('click', () => { void navigator.clipboard.writeText(app.code); toast('Code copied.'); });
  const full = iconButton({ icon: 'expand', title: 'Full screen', size: 'sm' });
  full.addEventListener('click', () => void modal((done) => h('div', { class: 'app-modal' }, modalHeader(app.title, done), appFrame(app.id, app.version)), true));
  const save = iconButton({ icon: 'download', title: 'Download as a web page', size: 'sm' });
  save.addEventListener('click', () => download(app.id));
  panelId = app.id;
  panelHost(
    h(
      'div',
      { class: 'panel-inner' },
      h(
        'div',
        { class: 'panel-head' },
        h('div', { class: 'panel-tools' }, close, viewSwitch(app, body, 'panel-frame')),
        h('strong', { class: 'panel-title' }, app.title),
        h('span', { class: 'spacer' }),
        h('div', { class: 'panel-tools' }, copy, full, save)
      ),
      body
    )
  );
  markOpen();
}

/** A mini app the model built: a preview in the reply that opens in the side panel. */
function renderAppCard(part: Extract<Part, { kind: 'tool' }>, live: boolean): HTMLElement {
  const title = typeof part.arguments.title === 'string' && part.arguments.title ? part.arguments.title : 'App';
  const code = typeof part.arguments.content === 'string' ? part.arguments.content : '';
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
  const expand = h('button', { class: 'canvas-expand', on: { click: open } }, 'Expand');
  const card = h(
    'div',
    { class: `canvas-card app-card${panelId === id ? ' compact' : ''}`, 'data-canvas': id },
    chip,
    h('div', { class: 'canvas-head' }, icon('react', 16), h('span', { class: 'canvas-title' }, title), h('span', { class: 'spacer' }), viewSwitch(app, body), save),
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
  body.append(markdown(content));
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
      body.replaceChildren(markdown(content));
      toast('Saved. The assistant sees your edit on its next reply.');
    });
  });
  const expand = h('button', { class: 'canvas-expand' }, 'Expand');
  expand.addEventListener('click', () => {
    const open = card.classList.toggle('expanded');
    expand.textContent = open ? 'Collapse' : 'Expand';
  });

  card.append(
    h('div', { class: 'canvas-head' }, icon('file', 16), titleEl, h('span', { class: 'spacer' }), writing ? h('span', { class: 'dots' }) : null, edit, copy, download),
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
      h('div', { class: 'canvas-split' }, area, preview),
      h('div', { class: 'dialog-actions canvas-actions' }, h('span', { class: 'hint' }, 'Markdown · Ctrl+S saves a new version'), h('span', { class: 'spacer' }), h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'), h('button', { class: 'btn btn-primary', on: { click: () => void save() } }, 'Save'))
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
