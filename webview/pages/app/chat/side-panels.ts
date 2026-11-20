/** What opens beside a conversation or over it: sources, the reader, a worker's steps, another chat. */
import { hasPanel, openSidePanel } from '../canvas/canvas';
import { previewFor } from '../ui/favicon';
import { host, renderParts } from './transcript';
import type { Source } from './turn-actions';
import { composer } from './composer/index';
import { tauriInvoke } from '../../../shared/tauri';
import { api, type Part } from '../core/api';
import { clear, h } from '../ui/h';
import { icon } from '../ui/icons';
import { markdown } from '../ui/markdown';
import { modal, popover, toast } from '../ui/overlay';
import { navigate, state } from '../core/state';
import { closePanel } from '../canvas/canvas';
import { siteIcon } from '../ui/favicon';
import { button, iconButton, modalHeader } from '../ui/ui';
import { call, shown } from '../core/call';
import { toolLabel } from './tool-labels';

/** The Sources panel beside the conversation: each page with its site, title and what it says. */
/** Asks which library, then saves the page's text there. */
async function saveToLibrary(anchor: HTMLElement, url: string) {
  const libraries = await tauriInvoke<{ id: string; name: string }[]>('chat_list_libraries').catch(() => []);
  const save = async (id: string, name: string) => {
    close();
    toast(`Saving to ${name}…`);
    await call('chat_add_web_page', { libraryId: id, url }).then(() => toast(`Saved to ${name}. Library searches can now find and cite it.`), shown);
  };
  const close = popover(
    anchor,
    h(
      'div',
      { class: 'menu' },
      h('div', { class: 'menu-heading' }, 'Save page to'),
      libraries.length
        ? libraries.map((l) => h('button', { class: 'menu-item', on: { click: () => void save(l.id, l.name) } }, icon('library', 15), h('span', {}, l.name)))
        : h('p', { class: 'muted menu-note' }, 'No libraries yet: make one under Libraries.')
    )
  );
}

interface ReaderArticle {
  url: string;
  title: string;
  site: string;
  byline: string;
  published: string;
  words: number;
  blocks: { kind: 'h' | 'p' | 'li' | 'quote' | 'code'; text: string }[];
}

/** A source as an article in the side panel: plain text, nothing of the page runs. */
function openReader(url: string, fallbackTitle: string, back: () => void) {
  const body = h('div', { class: 'reader-body' }, h('div', { class: 'status-line' }, h('span', { class: 'dots' }), ' Loading the article…'));
  const close = iconButton({ icon: 'close', title: 'Close', size: 'sm', onClick: closePanel });
  const backBtn = iconButton({ icon: 'back', title: 'Back to the sources', size: 'sm', onClick: back });
  const external = iconButton({ icon: 'globe', title: 'Open in the browser', size: 'sm', onClick: () => window.open(url, '_blank', 'noopener') });
  openSidePanel(
    `reader:${url}`,
    h('div', { class: 'panel-inner reader-panel' }, h('div', { class: 'panel-head' }, backBtn, h('strong', { class: 'panel-title' }, host(url)), h('span', { class: 'spacer' }), external, close), body),
    fallbackTitle
  );
  void tauriInvoke<ReaderArticle>('chat_read_article', { url })
    .then((a) => {
      const when = a.published && !Number.isNaN(Date.parse(a.published)) ? new Date(a.published).toLocaleDateString(undefined, { year: 'numeric', month: 'long', day: 'numeric' }) : '';
      const meta = [a.site, a.byline, when, `${Math.max(1, Math.round(a.words / 220))} min read`].filter(Boolean).join(' · ');
      const blocks: HTMLElement[] = [];
      let list: HTMLElement | null = null;
      for (const b of a.blocks) {
        if (b.kind !== 'li') list = null;
        if (b.kind === 'li') {
          if (!list) blocks.push((list = h('ul', {})));
          list.append(h('li', {}, b.text));
        } else if (b.kind === 'h') blocks.push(h('h3', {}, b.text));
        else if (b.kind === 'quote') blocks.push(h('blockquote', {}, b.text));
        else if (b.kind === 'code') blocks.push(h('pre', {}, b.text));
        else blocks.push(h('p', {}, b.text));
      }
      clear(body, h('article', { class: 'reader-article no-translate' }, h('h2', {}, a.title || fallbackTitle), h('div', { class: 'reader-meta muted small' }, meta), ...blocks));
    })
    .catch((e) => clear(body, h('p', { class: 'error-text' }, String(e)), button({ variant: 'secondary', icon: 'globe', label: 'Open in the browser', onClick: () => window.open(url, '_blank', 'noopener') })));
}

/**
 * Another chat beside this one (Open WebUI's tabs): its conversation in a side-panel tab, kept
 * up to date while it runs, with a way to swap the two.
 */
export function openChatBeside(id: string) {
  if (!hasPanel() || state.view.kind !== 'chat') return void toast('Open a chat first; the other one opens beside it.');
  if (state.view.id === id) return void toast('This chat is already open.');
  const current = state.view.id;
  const body = h('div', { class: 'chat-beside' }, h('p', { class: 'muted small' }, 'Loading…'));
  const title = h('strong', { class: 'panel-title' }, state.conversations.find((c) => c.id === id)?.title || 'Chat');
  const swap = iconButton({
    icon: 'columns',
    title: 'Swap: this one in front, the current one beside',
    size: 'sm',
    onClick: () => {
      closePanel();
      navigate({ kind: 'chat', id });
      setTimeout(() => openChatBeside(current), 500);
    },
  });
  const open = iconButton({ icon: 'expand', title: 'Open this chat', size: 'sm', onClick: () => navigate({ kind: 'chat', id }) });
  const close = iconButton({ icon: 'close', title: 'Close', size: 'sm', onClick: closePanel });
  const panel = h('div', { class: 'panel-inner' }, h('div', { class: 'panel-head' }, icon('chat', 15), title, h('span', { class: 'spacer' }), swap, open, close), body);
  openSidePanel(`chat:${id}`, panel, state.conversations.find((c) => c.id === id)?.title || 'Chat');
  let shown = '';
  const draw = async () => {
    if (!panel.isConnected && shown) return;
    try {
      const c = await api.conversation(id);
      const key = `${c.turns.length}:${JSON.stringify(c.turns[c.turns.length - 1]?.parts ?? []).length}:${c.running}`;
      if (key !== shown) {
        const first = !shown;
        shown = key;
        title.textContent = c.title || 'Chat';
        const atEnd = body.scrollHeight - body.scrollTop - body.clientHeight < 40;
        clear(
          body,
          c.turns.length
            ? c.turns.map((t) =>
                t.role === 'user'
                  ? h(
                      'div',
                      { class: 'beside-user' },
                      t.parts
                        .filter((p) => p.kind === 'text')
                        .map((p) => (p as { text: string }).text.replace(/<attachment[\s\S]*$/, '').trim())
                        .join('\n')
                    )
                  : h('div', { class: 'beside-assistant' }, renderParts(t.parts, c.running && t === c.turns[c.turns.length - 1], `${id}:${t.index}`))
              )
            : h('p', { class: 'muted small' }, 'Nothing here yet.')
        );
        if (atEnd || first) body.scrollTop = body.scrollHeight;
      }
      // Kept up to date while it is open: often while it runs, now and then otherwise.
      setTimeout(() => void draw(), c.running ? 1500 : 5000);
    } catch (e) {
      clear(body, h('p', { class: 'error-text' }, String(e)));
    }
  };
  void draw();
}

export function openSourcesPanel(sources: Source[]) {
  const close = iconButton({ icon: 'close', title: 'Close', size: 'sm', onClick: closePanel });
  openSidePanel(
    `sources:${sources.map((s) => s.url).join('|')}`,
    h(
      'div',
      { class: 'panel-inner sources-panel' },
      h('div', { class: 'panel-head' }, h('strong', { class: 'panel-title sources-title' }, 'Sources'), h('span', { class: 'spacer' }), close),
      h(
        'div',
        { class: 'sources-list' },
        [...sources.filter((s) => s.cited), ...sources.filter((s) => !s.cited)].map((s, i, list) => {
          const snippet = h('p', { class: 'source-snippet' }, s.snippet);
          const title = h('strong', { class: 'source-title' }, s.title);
          // A title that is only the site's name ("RTP"), or no description: the page's own, fetched once.
          if (!s.snippet || s.title.trim().split(/\s+/).length <= 3) {
            void previewFor(s.url).then((p) => {
              if (p?.title && s.title.trim().split(/\s+/).length <= 3) title.textContent = p.title;
              if (p?.description && !s.snippet) snippet.textContent = p.description;
            });
          }
          // Keep the page: its text goes into a library, to search and cite later.
          const save = h('button', { class: 'icon-btn tiny source-save', title: 'Save this page to a library' }, icon('library', 14));
          save.addEventListener('click', (e) => {
            e.preventDefault();
            e.stopPropagation();
            void saveToLibrary(save, s.url);
          });
          // Read it here, as plain text, without leaving the chat (Vibe's article reader).
          const read = h('button', { class: 'icon-btn tiny source-save', title: 'Read here' }, icon('book', 14));
          read.addEventListener('click', (e) => {
            e.preventDefault();
            e.stopPropagation();
            openReader(s.url, s.title, () => openSourcesPanel(sources));
          });
          // Add in full (Onyx): the page's article text goes with the next message.
          const full = h('button', { class: 'icon-btn tiny source-save', title: 'Add the whole page to your next message' }, icon('plus', 14));
          full.addEventListener('click', (e) => {
            e.preventDefault();
            e.stopPropagation();
            void tauriInvoke<{ title: string; blocks: { text: string }[] }>('chat_read_article', { url: s.url })
              .then((a) => {
                composer.attachText(`Page: ${a.title || s.title}`, `${a.title}\n${s.url}\n\n${a.blocks.map((b) => b.text).join('\n\n')}`);
                toast('The page is attached to your next message.');
              })
              .catch((err) => toast(String(err), 'error'));
          });
          const heading = i === 0 && s.cited ? h('div', { class: 'menu-heading sources-heading' }, 'Cited') : !s.cited && (i === 0 || list[i - 1]!.cited) ? h('div', { class: 'menu-heading sources-heading' }, 'Also found') : null;
          const entry = h(
            'a',
            { class: `source-entry${s.cited ? '' : ' found'}`, href: s.url, target: '_blank', rel: 'noopener noreferrer' },
            h('div', { class: 'source-site' }, siteIcon(s.url), h('span', { class: 'grow' }, host(s.url)), read, full, save),
            title,
            snippet
          );
          return heading ? h('div', {}, heading, entry) : entry;
        })
      )
    ),
    'Sources'
  );
}

/**
 * Library citations as numbered footnotes, as in Vibe: each [document] in the answer becomes a
 * number, and the passages the answer drew on are listed under it; a number jumps to its passage.
 */
export function numberLibraryCitations(el: HTMLElement, parts: Part[]) {
  const pills = [...el.querySelectorAll<HTMLElement>('.cite-doc:not(.cite-knowledge)')];
  if (!pills.length) return;
  // The passages each document gave, from the library searches of this reply.
  const passages = new Map<string, { library: string; text: string }>();
  for (const part of parts) {
    if (part.kind !== 'tool' || part.name !== 'search_library' || !part.output) continue;
    try {
      const value = JSON.parse(part.output) as { passages?: { library?: string; document?: string; text?: string }[] };
      for (const p of value.passages ?? []) {
        if (p.document && p.text && !passages.has(p.document)) passages.set(p.document, { library: p.library ?? '', text: p.text });
      }
    } catch {
      // Not JSON: no passages from this search.
    }
  }
  const numbers = new Map<string, number>();
  const notes = h('ol', { class: 'footnotes' });
  for (const pill of pills) {
    const doc = pill.textContent?.trim() ?? '';
    if (!doc) continue;
    let n = numbers.get(doc);
    if (n === undefined) {
      n = numbers.size + 1;
      numbers.set(doc, n);
      const passage = passages.get(doc);
      const note = h(
        'li',
        { class: 'footnote' },
        h('div', { class: 'footnote-head' }, h('span', { class: 'footnote-n' }, String(n)), icon('file', 13), h('strong', {}, doc), passage?.library ? h('span', { class: 'muted' }, ` · ${passage.library}`) : null),
        passage ? h('p', { class: 'footnote-text' }, passage.text.length > 420 ? `${passage.text.slice(0, 420).trim()}…` : passage.text) : null
      );
      notes.append(note);
    }
    const target = notes.children[n - 1] as HTMLElement;
    const marker = h('button', { class: 'footnote-ref', title: doc }, String(n));
    marker.addEventListener('click', () => {
      target.scrollIntoView({ behavior: 'smooth', block: 'center' });
      target.classList.remove('flash');
      void target.offsetWidth;
      target.classList.add('flash');
    });
    pill.replaceWith(marker);
  }
  const lastText = [...el.querySelectorAll('.markdown')].pop();
  (lastText ?? el).after(h('div', { class: 'footnotes-wrap' }, h('div', { class: 'step-label' }, 'Library sources'), notes));
}

/** A sub-agent's transcript: each tool it called, with what came back, then its report. */
export async function showWorkerSteps(callId: string, index: number, title: string) {
  const steps = await api.subagentTranscript(callId, index).catch(() => []);
  void modal((close) =>
    h(
      'div',
      { class: 'dialog-small wide worker-steps' },
      modalHeader(title, close, { flush: true }),
      steps.length
        ? h(
            'div',
            { class: 'timeline' },
            steps.map((s, n) =>
              s.kind === 'call'
                ? h(
                    'details',
                    { class: `worker-step${s.isError ? ' failed' : ''}` },
                    h('summary', {}, h('span', { class: 'muted' }, `${n + 1}.`), icon(toolLabel(s.tool).icon, 15), h('strong', {}, toolLabel(s.tool).action), h('span', { class: 'muted worker-step-input' }, s.input)),
                    h('pre', { class: 'step-pre' }, s.output)
                  )
                : h('div', { class: 'worker-step report' }, h('div', { class: 'step-label' }, 'Report'), markdown(s.output))
            )
          )
        : h('p', { class: 'muted' }, 'The steps of this worker were not kept (it ran before this version).')
    )
  );
}
