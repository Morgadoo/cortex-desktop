/** Canvas cards in the conversation, their editor, and quoting a selection into the composer. */
import { tauriInvoke } from '../../shared/tauri';
import type { Part } from './api';
import { clear, h, icon, markdown, modal, toast } from './dom';

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

/** A mini app the model built: a live preview, its code, and a way to open it larger. */
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
  const body = h('div', { class: 'app-body' });
  const showPreview = () => clear(body, id && part.output ? appFrame(id, version) : h('div', { class: 'status-line app-wait' }, h('span', { class: 'dots' }), 'Building the app…'));
  const showCode = () => {
    const pre = h('pre', { class: 'step-pre app-code' }, code);
    clear(body, pre);
  };
  const preview = h('button', { class: 'seg active' }, 'Preview');
  const source = h('button', { class: 'seg' }, 'Code');
  preview.addEventListener('click', () => { preview.classList.add('active'); source.classList.remove('active'); showPreview(); });
  source.addEventListener('click', () => { source.classList.add('active'); preview.classList.remove('active'); showCode(); });
  const copy = h('button', { class: 'canvas-btn', title: 'Copy code' }, icon('copy', 15), 'Copy');
  copy.addEventListener('click', () => { void navigator.clipboard.writeText(code); toast('Code copied.'); });
  const full = h('button', { class: 'canvas-btn icon-only', title: 'Open larger' }, icon('share', 16));
  full.addEventListener('click', () => {
    if (!id) return;
    void modal((close) => h('div', { class: 'app-modal' }, h('div', { class: 'modal-head' }, h('h2', {}, title), h('button', { class: 'icon-btn', on: { click: close } }, icon('close', 18))), appFrame(id, version)), true);
  });
  showPreview();
  const card = h('div', { class: 'canvas-card app-card' }, h('div', { class: 'canvas-head' }, icon('apps', 16), h('span', { class: 'canvas-title' }, title), h('div', { class: 'segmented' }, preview, source), h('span', { class: 'spacer' }), copy, full), body);
  const created = typeof part.arguments.id !== 'string' || !part.arguments.id;
  return h('div', { class: 'canvas-step' }, h('div', { class: 'canvas-label' }, live && part.output === null ? 'Building an app…' : created ? 'Built an app' : 'Updated an app'), card);
}

/** A canvas the model created or updated, shown as a document card in the reply. */
export function renderCanvasCard(part: Extract<Part, { kind: 'tool' }>, live: boolean): HTMLElement {
  if (part.arguments.kind === 'app') return renderAppCard(part, live);
  const created = typeof part.arguments.id !== 'string' || !part.arguments.id;
  let title = typeof part.arguments.title === 'string' && part.arguments.title ? part.arguments.title : 'Untitled';
  let content = typeof part.arguments.content === 'string' ? part.arguments.content : '';
  const writing = live && part.output === null;
  const body = h('div', { class: 'markdown canvas-body' });
  body.append(markdown(content));
  const titleEl = h('span', { class: 'canvas-title' }, title);
  const card = h('div', { class: 'canvas-card' });

  const copy = h('button', { class: 'canvas-btn', title: 'Copy' }, icon('copy', 15), 'Copy');
  copy.addEventListener('click', () => {
    void navigator.clipboard.writeText(content);
    toast('Canvas copied.');
  });
  const download = h('button', { class: 'canvas-btn icon-only', title: 'Download as Markdown' }, icon('download', 16));
  download.addEventListener('click', () => {
    const id = idOf(part);
    if (id) void api.export(id).then((saved) => saved && toast('Canvas saved.')).catch((e) => toast(String(e), 'error'));
  });
  const edit = h('button', { class: 'canvas-btn', title: 'Edit' }, icon('edit', 15), 'Edit');
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
      h('div', { class: 'modal-head' }, titleInput, h('button', { class: 'icon-btn', title: 'Close', on: { click: close } }, icon('close', 18))),
      h('div', { class: 'canvas-split' }, area, preview),
      h('div', { class: 'dialog-actions canvas-actions' }, h('span', { class: 'hint' }, 'Markdown · Ctrl+S saves a new version'), h('span', { class: 'spacer' }), h('button', { class: 'btn btn-ghost', on: { click: close } }, 'Cancel'), h('button', { class: 'btn btn-primary', on: { click: () => void save() } }, 'Save'))
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
