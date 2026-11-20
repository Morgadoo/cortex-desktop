/** What can be done with a finished turn: the buttons under it, its sources, versions and edits. */
import { api, type Conversation, type Part, type Turn } from '../core/api';
import { conversationPng } from './chatimage';
import { clear, h } from '../ui/h';
import { icon } from '../ui/icons';
import { markdown } from '../ui/markdown';
import { confirmDialog, modal, popover, promptText, toast } from '../ui/overlay';
import { timeLabel, clockHour12 } from '../core/clock';
import { changed, navigate, state } from '../core/state';
import { lineDiff } from '../canvas/canvas';
import { hostOf, siteIcon } from '../ui/favicon';
import { bytesToBase64 } from '../core/format';
import { button, iconButton, modalHeader } from '../ui/ui';
import { json } from '../core/prefs';
import { call, callQuiet, reported, shown } from '../core/call';
import { numberLibraryCitations, openSourcesPanel } from './side-panels';
import { viewCache } from './chat-view';
import { composer } from './composer/index';
import { refreshConversations, regenerate } from './send';
import { readAloudButton } from './speech';
import { host, resultRows, stepDurations, textOf, renderParts } from './transcript';

/** Bookmarked turn indexes of the open conversations, kept in step with the saved ones. */
export const bookmarked = new Map<string, Set<number>>();

/** Long answers can be folded to a few lines, to scroll past them (LobeHub). */
function foldAnswerButton(turn: Turn): HTMLElement | null {
  const length = textOf(turn.parts).length;
  if (length < 2500) return null;
  const button = h('button', { class: 'icon-btn small', title: 'Fold this answer' }, icon('chevronUp', 16));
  button.addEventListener('click', () => {
    const el = button.closest('.turn-assistant');
    if (!el) return;
    const folded = el.classList.toggle('answer-folded');
    button.title = folded ? 'Unfold this answer' : 'Fold this answer';
    button.replaceChildren(icon(folded ? 'chevron' : 'chevronUp', 16));
    if (folded) el.scrollIntoView({ block: 'nearest' });
  });
  return button;
}

/** Marks a message to find again: listed in the chat's header and in search (Ctrl+K). */
export function bookmarkButton(turn: Turn, conversationId: string, size = 16): HTMLElement {
  const on = () => bookmarked.get(conversationId)?.has(turn.index) ?? false;
  const button = h('button', { class: `icon-btn small${on() ? ' active' : ''}`, title: on() ? 'Remove bookmark' : 'Bookmark' }, icon('bookmark', size));
  button.addEventListener('click', async () => {
    const next = !on();
    try {
      await call('chat_bookmark', { id: conversationId, index: turn.index, on: next });
      const set = bookmarked.get(conversationId) ?? new Set<number>();
      if (next) set.add(turn.index);
      else set.delete(turn.index);
      bookmarked.set(conversationId, set);
      button.classList.toggle('active', next);
      button.title = next ? 'Remove bookmark' : 'Bookmark';
      toast(next ? 'Bookmarked. Find it in the chat header or with Ctrl+K.' : 'Bookmark removed.');
      await reported(refreshConversations());
    } catch {
      // Already shown.
    }
  });
  return button;
}

/**
 * Prints the open conversation as it reads on screen (Markdown, code, math, diagrams), without the
 * app around it; the system print dialog can save it as a PDF.
 */
/** The conversation as one PNG (LobeHub's screenshot export). */
export async function exportImage(id: string) {
  try {
    const c = await reported(api.conversation(id));
    const messages = c.turns.map((t) => ({ role: t.role, text: textOf(t.parts) })).filter((m) => m.text.trim());
    const png = await reported(conversationPng(c.title, messages));
    const name = `${
      (c.title || 'conversation')
        .replace(/[\\/:*?"<>|]+/g, ' ')
        .trim()
        .slice(0, 60) || 'conversation'
    }.png`;
    if (await call<boolean>('chat_save_bytes', { name, data: bytesToBase64(png) })) toast('Image saved.');
  } catch {
    // Already shown.
  }
}

export async function printConversation(id: string, title: string) {
  if (viewCache.chat?.key !== `chat:${id}`) {
    navigate({ kind: 'chat', id });
    await new Promise((r) => setTimeout(r, 600));
  }
  // Let the menu that started this close first.
  await new Promise((r) => setTimeout(r, 120));
  const transcript = document.querySelector<HTMLElement>('.chat .transcript');
  if (!transcript) return void toast('Open the conversation first.', 'error');
  // Long messages are shown whole on paper.
  const folded = [...transcript.querySelectorAll<HTMLElement>('.folded')];
  folded.forEach((el) => el.classList.remove('folded'));
  const head = h('div', {}, h('h1', { class: 'print-title' }, title || 'Conversation'), h('p', { class: 'print-meta' }, `Cortex · ${new Date().toLocaleString(undefined, { hour12: clockHour12() })}`));
  transcript.before(head);
  try {
    window.print();
  } finally {
    // Printing returns once the dialog closes.
    setTimeout(() => {
      head.remove();
      folded.forEach((el) => el.classList.add('folded'));
    }, 500);
  }
}

/** A new chat that keeps this one up to here, with or without the files attached on the way. */
function branchHereButton(turn: Turn, conversationId: string): HTMLElement {
  const button = h('button', { class: 'icon-btn small', title: 'Branch from here' }, icon('branch', 16));
  button.addEventListener('click', () => {
    const go = async (withoutFiles: boolean) => {
      close();
      try {
        const branch = await reported(api.forkConversation(conversationId, turn.index, withoutFiles));
        await reported(refreshConversations());
        navigate({ kind: 'chat', id: branch });
        toast(withoutFiles ? 'Branched from that answer, without the attached files.' : 'Branched from that answer.');
      } catch {
        // Already shown.
      }
    };
    const close = popover(
      button,
      h(
        'div',
        { class: 'menu' },
        h('div', { class: 'menu-heading' }, 'New chat up to this answer'),
        h('button', { class: 'menu-item', on: { click: () => void go(false) } }, icon('branch', 15), h('span', {}, 'With attached files')),
        h('button', { class: 'menu-item', on: { click: () => void go(true) } }, icon('file', 15), h('span', {}, 'Without attached files'))
      )
    );
  });
  return button;
}

/** Scrolls to a turn by its index and flashes it. */
export function scrollToTurn(index: number) {
  const target = document.querySelector<HTMLElement>(`.turn[data-index="${index}"]`);
  if (!target) return false;
  target.scrollIntoView({ behavior: 'smooth', block: 'center' });
  target.classList.remove('flash');
  void target.offsetWidth;
  target.classList.add('flash');
  return true;
}

const TRANSLATE_TO = ['English', 'Português', 'Español', 'Français', 'Deutsch', 'Italiano', '日本語', '中文'];

/** Translates an answer below itself (LibreChat, LobeHub); the conversation keeps the original. */
/** Saves an edited message in place; no new reply is asked for. */
async function rewrite(conversationId: string, index: number, text: string) {
  try {
    await call('chat_rewrite_message', { id: conversationId, index, text });
    await reported(viewCache.chat?.refresh(false));
    toast('Saved.');
  } catch {
    // Already shown.
  }
}

/** Edit an answer in place, e.g. to correct it before it is built on (LibreChat, LobeHub). */
function editAnswerButton(turn: Turn, conversationId: string): HTMLElement {
  const button = h('button', { class: 'icon-btn small', title: 'Edit answer' }, icon('edit', 16));
  button.addEventListener('click', () => {
    const el = button.closest('.turn-assistant');
    const texts = turn.parts.filter((p): p is Extract<Part, { kind: 'text' }> => p.kind === 'text');
    if (!el || !texts.length) return;
    const area = h('textarea', { class: 'input textarea edit-area', rows: 8 });
    area.value = texts[texts.length - 1]!.text;
    area.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') void viewCache.chat?.refresh(false);
      if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) void rewrite(conversationId, turn.index, area.value);
    });
    el.replaceChildren(
      h(
        'div',
        { class: 'edit-box' },
        area,
        h(
          'div',
          { class: 'dialog-actions' },
          h('small', { class: 'muted' }, 'The final text of this answer; tool steps stay as they were.'),
          h('span', { class: 'spacer' }),
          h('button', { class: 'btn btn-secondary', on: { click: () => void viewCache.chat?.refresh(false) } }, 'Cancel'),
          h('button', { class: 'btn btn-primary', on: { click: () => void rewrite(conversationId, turn.index, area.value) } }, 'Save')
        )
      )
    );
    area.style.height = `${Math.min(520, Math.max(160, area.scrollHeight + 4))}px`;
    area.focus();
  });
  return button;
}

interface AnswerAction {
  name: string;
  prompt: string;
}

const DEFAULT_ANSWER_ACTIONS: AnswerAction[] = [
  { name: 'Summarize', prompt: 'Summarize that answer in three short bullet points.' },
  { name: 'Explain more simply', prompt: 'Explain that answer again in plain words, as to someone new to the subject.' },
  { name: 'Make a checklist', prompt: 'Turn that answer into a checklist of concrete steps, as a Markdown task list.' },
  { name: 'Find the weak points', prompt: 'What in that answer is most likely wrong, missing or oversimplified? Be specific.' },
];

/** The actions under each answer: the built-in ones until the user changes the list. */
const answerActionsPref = json<AnswerAction[] | null>('cortex.answerActions', () => null);

function answerActions(): AnswerAction[] {
  const saved = answerActionsPref.get();
  if (Array.isArray(saved)) return saved.filter((a) => a && typeof a.name === 'string' && typeof a.prompt === 'string');
  return DEFAULT_ANSWER_ACTIONS;
}

function saveAnswerActions(actions: AnswerAction[]) {
  if (!answerActionsPref.set(actions)) toast('The actions could not be saved.', 'error');
}

/** Edits the list of answer actions: a name and the prompt each one sends. */
function editAnswerActions() {
  void modal((close) => {
    const rows = h('div', { class: 'action-rows' });
    const addRow = (a: AnswerAction) => {
      const name = h('input', { class: 'input', value: a.name, placeholder: 'Name' });
      const prompt = h('textarea', { class: 'input textarea', rows: 2, placeholder: 'What to ask about the answer' });
      prompt.value = a.prompt;
      const row = h('div', { class: 'action-row' }, h('div', { class: 'action-fields' }, name, prompt), iconButton({ icon: 'trash', title: 'Remove', size: 'sm', onClick: () => row.remove() }));
      rows.append(row);
    };
    answerActions().forEach(addRow);
    const save = () => {
      const list = [...rows.querySelectorAll('.action-row')]
        .map((r) => ({ name: (r.querySelector('input') as HTMLInputElement).value.trim(), prompt: (r.querySelector('textarea') as HTMLTextAreaElement).value.trim() }))
        .filter((a) => a.name && a.prompt);
      saveAnswerActions(list);
      close();
      toast('Actions saved.');
    };
    return h(
      'div',
      { class: 'dialog-small wide' },
      modalHeader('Answer actions', close, { flush: true }),
      h('p', { class: 'muted small' }, 'Each action asks about the answer it is under, as a new message in the chat. Write {{answer}} to put the answer’s text in the prompt yourself.'),
      rows,
      h(
        'div',
        { class: 'dialog-actions' },
        button({
          variant: 'secondary',
          icon: 'plus',
          label: 'Add an action',
          onClick: () => {
            addRow({ name: '', prompt: '' });
            const last = rows.lastElementChild;
            last?.scrollIntoView({ block: 'nearest' });
            last?.querySelector('input')?.focus();
          },
        }),
        button({
          variant: 'subtle',
          label: 'Restore the built-in ones',
          onClick: () => {
            clear(rows);
            DEFAULT_ANSWER_ACTIONS.forEach(addRow);
          },
        }),
        h('span', { class: 'spacer' }),
        button({ variant: 'secondary', label: 'Cancel', onClick: close }),
        button({ variant: 'primary', label: 'Save', onClick: save })
      )
    );
  });
}

/**
 * Actions on an answer (Open WebUI's message actions): prompts of your own about it, sent as the
 * next message. An earlier answer is quoted so the model knows which one is meant.
 */
function actionsButton(turn: Turn, isLast: boolean): HTMLElement {
  const b = h('button', { class: 'icon-btn small', title: 'Actions' }, icon('bolt', 16));
  b.addEventListener('click', () => {
    const answer = textOf(turn.parts);
    const run = (a: AnswerAction) => {
      close();
      const withAnswer = a.prompt.includes('{{answer}}');
      const prompt = withAnswer ? a.prompt.split('{{answer}}').join(answer) : a.prompt;
      if (!isLast && !withAnswer) composer.setQuote(answer.length > 1500 ? `${answer.slice(0, 1500)}…` : answer);
      composer.setText(prompt);
      void composer.submit();
    };
    const close = popover(
      b,
      h(
        'div',
        { class: 'menu' },
        h('div', { class: 'menu-heading' }, 'Ask about this answer'),
        ...answerActions().map((a) => h('button', { class: 'menu-item', title: a.prompt, on: { click: () => run(a) } }, icon('bolt', 15), h('span', {}, a.name))),
        h('div', { class: 'menu-sep' }),
        h(
          'button',
          {
            class: 'menu-item',
            on: {
              click: () => {
                close();
                editAnswerActions();
              },
            },
          },
          icon('settings', 15),
          h('span', {}, 'Edit actions…')
        )
      ),
      'above'
    );
  });
  return b;
}

function translateButton(turn: Turn): HTMLElement {
  const button = h('button', { class: 'icon-btn small', title: 'Translate' }, icon('globe', 16));
  button.addEventListener('click', () => {
    const show = async (language: string) => {
      const host = button.closest('.turn');
      if (!host || !state.model) return;
      host.querySelector('.translation')?.remove();
      const box = h('div', { class: 'translation' }, h('div', { class: 'translation-head' }, icon('globe', 14), `Translating into ${language}…`));
      host.querySelector('.turn-actions')?.before(box);
      try {
        const text = await call<string>('chat_translate', { model: state.model, text: textOf(turn.parts), language });
        clear(
          box,
          h('div', { class: 'translation-head' }, icon('globe', 14), language, h('span', { class: 'spacer' }), h('button', { class: 'link-btn', on: { click: () => box.remove() } }, 'Hide')),
          h('div', { class: 'markdown' }, markdown(text))
        );
      } catch {
        box.remove();
      }
    };
    const close = popover(
      button,
      h(
        'div',
        { class: 'menu' },
        h('div', { class: 'menu-heading' }, 'Translate into'),
        TRANSLATE_TO.map((l) =>
          h(
            'button',
            {
              class: 'menu-item',
              on: {
                click: () => {
                  close();
                  void show(l);
                },
              },
            },
            l
          )
        ),
        h(
          'button',
          {
            class: 'menu-item',
            on: {
              click: async () => {
                close();
                const other = await promptText('Translate into', '', 'Any language');
                if (other) void show(other);
              },
            },
          },
          'Other…'
        )
      )
    );
  });
  return button;
}

/** Removes a sent message and the reply to it, to tidy a conversation. */
export function deleteExchangeButton(turn: Turn, conversationId: string): HTMLElement {
  return h(
    'button',
    {
      class: 'icon-btn small',
      title: 'Delete this message and its reply',
      on: {
        click: async () => {
          if (!(await confirmDialog('Delete this message?', 'The message and the reply to it are removed from the conversation.'))) return;
          try {
            await call('chat_delete_exchange', { id: conversationId, index: turn.index });
            await reported(viewCache.chat?.refresh(false));
          } catch {
            // Already shown.
          }
        },
      },
    },
    icon('trash', 15)
  );
}

/** Edit a sent message: the conversation is answered again from that point. */
export function editButton(turn: Turn, conversationId: string, text: string): HTMLElement {
  const button = h('button', { class: 'icon-btn small', title: 'Edit' }, icon('edit', 15));
  button.addEventListener('click', () => {
    const el = button.closest('.turn-user');
    if (!el) return;
    const area = h('textarea', { class: 'input textarea edit-area', rows: 3 });
    // Attached files stay attached; only the message itself is edited.
    const attached = (text.match(/\n*<attachment name="[^"]*">[\s\S]*?<\/attachment>/g) ?? []).join('');
    area.value = text.replace(/\n*<attachment name="[^"]*">[\s\S]*?<\/attachment>/g, '').trim();
    const full = () => `${area.value.trim()}${attached}`;
    const save = async () => {
      if (!state.model) return void toast('Choose a model first.', 'error');
      try {
        await reported(api.send({ conversationId, text: full(), model: state.model, speed: state.speed, approval: state.approval, editIndex: turn.index }));
      } catch {
        // Already shown.
      }
    };
    area.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        void save();
      }
      if (e.key === 'Escape') void viewCache.chat?.refresh(false);
    });
    el.replaceChildren(
      h(
        'div',
        { class: 'edit-box' },
        area,
        h(
          'div',
          { class: 'dialog-actions' },
          h('button', { class: 'btn btn-secondary', on: { click: () => void viewCache.chat?.refresh(false) } }, 'Cancel'),
          // Fix a typo or a detail for the record, keeping the answer that came back.
          h('button', { class: 'btn btn-secondary', title: 'Keep the answer below; the model reads the edited message from now on', on: { click: () => void rewrite(conversationId, turn.index, full()) } }, 'Save without reply'),
          h('button', { class: 'btn btn-primary', on: { click: () => void save() } }, 'Send')
        )
      )
    );
    area.focus();
  });
  return button;
}

export function renderAssistantTurn(conversation: Conversation, turn: Turn, isLast: boolean, running: boolean, branch: HTMLElement | null = null): HTMLElement {
  const durations = isLast ? stepDurations.get(conversation.id) : undefined;
  const el = h('div', { class: 'turn turn-assistant', 'data-index': String(turn.index) }, ...renderParts(turn.parts, false, `${conversation.id}:${turn.index}`, durations));
  numberLibraryCitations(el, turn.parts);
  // Opening a link the answer gave says it pointed somewhere useful: a signal in the run trace.
  el.addEventListener(
    'click',
    (e) => {
      if ((e.target as HTMLElement | null)?.closest('a[href]')) void callQuiet('chat_signal', { id: conversation.id, index: turn.index, kind: 'opened_source' });
    },
    { capture: true }
  );
  // A reply that stopped after its tools ran, before answering, can pick up where it left off.
  const unfinished = isLast && !running && turn.parts[turn.parts.length - 1]?.kind === 'tool' && !state.live.has(conversation.id);
  if (unfinished) {
    const resume = h('button', { class: 'btn btn-secondary' }, icon('retry', 15), 'Continue');
    resume.addEventListener('click', () => {
      if (!state.model) return void toast('Choose a model first.', 'error');
      void reported(api.send({ conversationId: conversation.id, text: '', model: state.model, speed: state.speed, approval: state.approval, resume: true })).catch(shown);
    });
    el.append(h('div', { class: 'error-card' }, icon('wait', 16), h('div', { class: 'grow' }, h('strong', {}, 'This reply stopped before it finished'), h('div', { class: 'hint' }, 'Continue picks up from the work already done.')), resume));
  }
  if (!isLast || !running) {
    const sources = renderSources(sourcesOf(turn.parts));
    if (sources) el.append(sources);
    el.append(renderActions(conversation, turn, isLast, branch));
  }
  return el;
}

export interface Source {
  url: string;
  title: string;
  snippet: string;
  /** Linked in the answer or read; false for search results it did not use (Onyx's "found"). */
  cited: boolean;
}

/** What an answer drew on: the pages it links to, then any it opened; search hits when it cites none. */
export function sourcesOf(parts: Part[]): Source[] {
  const found = new Map<string, Source>();
  // What the searches said about each page, for the Sources panel.
  const snippets = new Map<string, { title: string; snippet: string }>();
  for (const part of parts) {
    if (part.kind !== 'tool' || part.name !== 'web_search' || !part.output) continue;
    try {
      const value = JSON.parse(part.output) as { results?: { url?: string; title?: string; snippet?: string }[] };
      for (const r of value.results ?? []) if (r.url) snippets.set(r.url.replace(/\/$/, ''), { title: r.title ?? '', snippet: r.snippet ?? '' });
    } catch {
      // Not JSON: no snippets from this search.
    }
  }
  const add = (url: string, title: string, cited = true) => {
    if (!/^https?:/.test(url)) return;
    // A search engine's results page is how a source was found, not a source.
    if (/(^|\.)(duckduckgo\.com|bing\.com|search\.brave\.com)$/.test(hostOf(url)) || /google\.[a-z.]+\/search/.test(url)) return;
    const key = url.replace(/\/$/, '');
    const known = snippets.get(key);
    if (!found.has(key)) found.set(key, { url, title: known?.title || title || host(url), snippet: known?.snippet ?? '', cited });
  };
  for (const part of parts) {
    if (part.kind === 'text') {
      for (const m of part.text.matchAll(/\[([^\]]{1,120})\]\((https?:[^)\s]+)\)/g)) add(m[2]!, m[1]!);
    }
  }
  for (const part of parts) {
    if (part.kind === 'tool' && part.name.startsWith('browser__') && typeof part.arguments.url === 'string') {
      add(part.arguments.url, resultRows(part)?.rows[0]?.title ?? '');
    }
  }
  // Search results the answer did not use are listed apart; with nothing cited, the top ones
  // stand in as its sources, as before.
  const nothingCited = !found.size;
  for (const part of parts) {
    if (part.kind === 'tool' && part.name === 'web_search') {
      for (const [i, row] of (resultRows(part)?.rows ?? []).slice(0, 10).entries()) if (row.href) add(row.href, row.title, nothingCited && i < 5);
    }
  }
  return [...found.values()];
}

function renderSources(all: Source[]): HTMLElement | null {
  const sources = all.filter((s) => s.cited);
  if (!sources.length) return null;
  // The sites' icons, overlapping, as on Vibe's Sources chip.
  const distinct = [...new Map(sources.map((s) => [hostOf(s.url), s])).values()];
  const chip = h(
    'button',
    { class: 'sources-chip' },
    'Sources',
    h(
      'span',
      { class: 'source-dots' },
      distinct.slice(0, 3).map((s) => siteIcon(s.url, 'source-dot'))
    )
  );
  chip.addEventListener('click', () => openSourcesPanel(all));
  return h('div', { class: 'sources-row' }, chip);
}

function renderActions(conversation: Conversation, turn: Turn, isLast: boolean, branch: HTMLElement | null = null): HTMLElement {
  const rate = (value: number) => async () => {
    const next = turn.rating === value ? 0 : value;
    turn.rating = next;
    await api.updateConversation({ id: conversation.id, rating: [turn.index, next] });
    changed();
    viewCache.chat?.refresh(false);
    // A thumbs down can say what went wrong; kept with the conversation for later review.
    if (next < 0) {
      const reason = await promptText('What was wrong with this reply? (optional)', '', 'Inaccurate, too long, missed the point…');
      if (reason) await callQuiet('chat_rate_reason', { id: conversation.id, index: turn.index, reason });
    }
  };
  const copy = h('button', { class: 'icon-btn small', title: 'Copy' }, icon('copy', 16));
  copy.addEventListener('click', () => {
    // Formatted for Office and mail apps, plain Markdown for editors (LibreChat's rich copy).
    const text = textOf(turn.parts);
    const html = h('div', {}, markdown(text)).innerHTML;
    const rich =
      typeof ClipboardItem !== 'undefined'
        ? navigator.clipboard.write([new ClipboardItem({ 'text/html': new Blob([html], { type: 'text/html' }), 'text/plain': new Blob([text], { type: 'text/plain' }) })])
        : Promise.reject(new Error('no rich clipboard'));
    void rich.catch(() => navigator.clipboard.writeText(text));
    // Copying an answer says it was useful; it goes into the run trace as a signal.
    void callQuiet('chat_signal', { id: conversation.id, index: turn.index, kind: 'copied' });
    copy.replaceChildren(icon('check', 16));
    setTimeout(() => copy.replaceChildren(icon('copy', 16)), 1200);
  });
  const actions = h(
    'div',
    { class: 'turn-actions' },
    branch,
    h('button', { class: `icon-btn small${turn.rating > 0 ? ' active-good' : ''}`, title: 'Good reply', on: { click: rate(1) } }, icon('thumbUp', 16)),
    h('button', { class: `icon-btn small${turn.rating < 0 ? ' active-bad' : ''}`, title: 'Bad reply', on: { click: rate(-1) } }, icon('thumbDown', 16)),
    isLast ? h('button', { class: 'icon-btn small', title: 'Regenerate', on: { click: () => void regenerate(conversation.id) } }, icon('retry', 16)) : null,
    copy,
    foldAnswerButton(turn),
    bookmarkButton(turn, conversation.id),
    editAnswerButton(turn, conversation.id),
    isLast ? null : branchHereButton(turn, conversation.id),
    translateButton(turn),
    actionsButton(turn, isLast),
    readAloudButton(textOf(turn.parts)),
    h('span', { class: 'spacer' }),
    h('span', { class: 'turn-time' }, timeLabel(turn.time))
  );
  return actions;
}

/** ‹ 2 / 3 ›: earlier answers to the last prompt, kept when it was regenerated. */
export function versionSwitcher(conversationId: string, versions: { from: number; current: number; total: number; kind: string }): HTMLElement {
  const go = (index: number) => async () => {
    try {
      await reported(api.selectVersion(conversationId, versions.from, index));
      await reported(viewCache.chat?.refresh(false));
    } catch {
      // Already shown.
    }
  };
  const prev = iconButton({ icon: 'back', title: 'Previous answer', size: 'xs', onClick: go(versions.current - 1) });
  const next = iconButton({ icon: 'forward', title: 'Next answer', size: 'xs', onClick: go(versions.current + 1) });
  prev.disabled = versions.current === 0;
  next.disabled = versions.current >= versions.total - 1;
  const compare = iconButton({ icon: 'columns', title: 'Compare two versions', size: 'xs', onClick: () => void compareVersions(conversationId, versions.from, versions.current) });
  return h(
    'span',
    { class: 'versions', title: versions.kind === 'prompt' ? 'Versions of this message and what followed' : 'Answers to this message' },
    prev,
    h('span', { class: 'versions-label' }, `${versions.current + 1} / ${versions.total}`),
    next,
    compare
  );
}

/** Two versions of an answer side by side as one text, words removed and added marked (Open WebUI's Compare Text). */
async function compareVersions(conversationId: string, from: number, current: number) {
  let texts: string[];
  try {
    texts = await call<string[]>('chat_version_texts', { id: conversationId, from });
  } catch {
    return;
  }
  if (texts.length < 2) return;
  void modal((close) => {
    const pick = (value: number) =>
      h(
        'select',
        { class: 'input select-sm' },
        texts.map((_, i) => h('option', { value: String(i), selected: i === value }, `Version ${i + 1}`))
      );
    const left = pick(current > 0 ? current - 1 : 1);
    const right = pick(current);
    const out = h('div', { class: 'word-diff' });
    const render = () => {
      const a = texts[Number(left.value)] ?? '';
      const b = texts[Number(right.value)] ?? '';
      // Words, with line breaks kept as their own token; runs of one kind join into one mark.
      const NL = '\u2028';
      const tokens = (t: string) =>
        t
          .replace(/\n+/g, ` ${NL} `)
          .split(/[ \t]+/)
          .filter((x) => x !== '');
      const diff = lineDiff(tokens(a).join('\n'), tokens(b).join('\n'));
      let removed = 0;
      let added = 0;
      const runs: { kind: string; words: string[] }[] = [];
      for (const d of diff) {
        if (d.text !== NL) {
          if (d.kind === '-') removed++;
          if (d.kind === '+') added++;
        }
        const last = runs[runs.length - 1];
        if (last && last.kind === d.kind) last.words.push(d.text);
        else runs.push({ kind: d.kind, words: [d.text] });
      }
      const show = (words: string[]) =>
        words
          .join(' ')
          .split(NL)
          .flatMap((part, i) => (i ? [h('br'), part.trim()] : [part.trim()]));
      clear(
        out,
        runs.flatMap((r, i) => {
          const body = r.kind === ' ' ? show(r.words) : [h(r.kind === '-' ? 'del' : 'ins', {}, ...show(r.words))];
          return i ? [' ', ...body] : body;
        })
      );
      stats.textContent = `${removed} words removed, ${added} added`;
    };
    const stats = h('small', { class: 'muted' });
    left.addEventListener('change', render);
    right.addEventListener('change', render);
    render();
    return h('div', { class: 'dialog-small wide' }, modalHeader('Compare versions', close, { flush: true }), h('div', { class: 'freq-row' }, left, h('span', { class: 'muted' }, '\u2192'), right, h('span', { class: 'spacer' }), stats), out);
  });
}
