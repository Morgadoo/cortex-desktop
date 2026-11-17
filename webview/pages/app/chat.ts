/** The home screen, a conversation, and the composer they share. */
import { api, newId, type Conversation, type Part, type Turn } from './api';
import { clear, confirmDialog, greeting, h, icon, markdown, modal, popover, promptText, timeLabel, toast } from './dom';
import { changed, currentModel, navigate, state, type LiveTurn } from './state';
import { openSettings } from './settings';
import { openInstructions } from './pages';

// ---------------------------------------------------------------------------------------------
// Tool labels

const CONNECTOR_LABELS: Record<string, string> = {
  filesystem: 'Files',
  bash: 'Terminal',
  browser: 'Web',
  proton_mail: 'Mail',
  luminance: 'Luminance',
  tool_results: 'Saved output',
};

const CONNECTOR_ICONS: Record<string, string> = {
  filesystem: 'file',
  bash: 'terminal',
  browser: 'globe',
};

/** Past-tense labels for the built-in tools, as a timeline reads them. */
const ACTIONS: Record<string, string> = {
  filesystem__fs_search: 'Searched files',
  filesystem__fs_read: 'Read a file',
  filesystem__fs_write: 'Wrote a file',
  filesystem__fs_edit: 'Edited a file',
  bash__run: 'Ran a command',
  browser__fetch: 'Fetched a page',
  browser__navigate: 'Browsed a page',
  tool_results__read: 'Read saved output',
};

/** What an approval card asks to allow. */
const REQUESTS: Record<string, string> = {
  filesystem__fs_write: 'writing a file',
  filesystem__fs_edit: 'editing a file',
  bash__run: 'running a command',
  browser__fetch: 'sending a web request',
  browser__navigate: 'opening a page',
};

function humanize(name: string): string {
  const words = name.replace(/[_-]+/g, ' ').trim();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

export function toolLabel(name: string): { connector: string; action: string; icon: string } {
  if (name === 'ask_user') return { connector: '', action: 'Asked you', icon: 'chat' };
  const [connector = '', tool = name] = name.includes('__') ? name.split(/__(.*)/s) : ['', name];
  const label = CONNECTOR_LABELS[connector] ?? humanize(connector.replace(/^mcp-/, ''));
  const known = ACTIONS[name];
  return { connector: known ? '' : label, action: known ?? humanize(tool), icon: CONNECTOR_ICONS[connector] ?? 'tool' };
}

/** The argument worth showing next to a tool step: a path, a URL, a query or a command. */
function argumentHint(args: Record<string, unknown>): string {
  for (const key of ['path', 'url', 'query', 'command', 'script', 'pattern', 'question', 'name']) {
    const value = args[key];
    // A file is known by its name; the folder is in the step's details.
    if (key === 'path' && typeof value === 'string' && value) return value.split(/[\\/]/).filter(Boolean).pop() ?? value;
    if (typeof value === 'string' && value) return value;
    if (key === 'script' && value && typeof value === 'object') {
      const program = (value as { program?: unknown }).program;
      if (typeof program === 'string') return program;
    }
  }
  return '';
}

// ---------------------------------------------------------------------------------------------
// Turn rendering

/**
 * How long each step of a conversation's latest reply took, by its position in the reply's parts.
 * Measured while the reply streams and kept for the session, so "Thought for 3s" survives the
 * reload from disk.
 */
const stepDurations = new Map<string, number[]>();

/** Whether the user opened or closed an activity group, so live re-renders keep their choice. */
const groupOpen = new Map<string, boolean>();

export function recordStepTimes(conversationId: string, startedAt: number[], endedAt: number[]) {
  stepDurations.set(
    conversationId,
    startedAt.map((start, i) => Math.max(0, (endedAt[i] ?? Date.now()) - start))
  );
}

function seconds(ms: number | undefined): string {
  if (ms === undefined || ms < 950) return '';
  const s = Math.round(ms / 1000);
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
}

function host(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, '');
  } catch {
    return url;
  }
}

interface ResultRow {
  title: string;
  source: string;
  href?: string;
}

/** Turns a structured tool result into rows for a results card; null when it has no list shape. */
function resultRows(part: Extract<Part, { kind: 'tool' }>): { heading: string; rows: ResultRow[] } | null {
  if (part.output === null || part.isError) return null;
  let value: unknown;
  try {
    value = JSON.parse(part.output);
  } catch {
    value = null;
  }
  const query = argumentHint(part.arguments);
  if (value && typeof value === 'object') {
    const record = value as Record<string, unknown>;
    // One fetched page: { title, url, content, ... }.
    if (typeof record.url === 'string' && typeof record.title === 'string' && record.title) {
      return { heading: '', rows: [{ title: record.title, source: host(record.url), href: record.url }] };
    }
    const list = ['results', 'items', 'matches', 'links', 'files'].map((k) => record[k]).find(Array.isArray) as unknown[] | undefined;
    if (list) {
      const rows = list
        .filter((x): x is Record<string, unknown> => Boolean(x) && typeof x === 'object')
        .map((x) => {
          const href = [x.url, x.href, x.link].find((v): v is string => typeof v === 'string');
          const path = typeof x.path === 'string' ? x.path : '';
          const title = [x.title, x.name, x.text, path, href].find((v): v is string => typeof v === 'string' && v.length > 0) ?? '';
          const source = href ? host(href) : path ? path.split(/[\\/]/).slice(-2, -1)[0] ?? '' : '';
          return { title, source, href };
        })
        .filter((r) => r.title);
      if (rows.length) return { heading: query, rows };
    }
  }
  // A fetched or browsed page: show the page itself as the one result.
  const url = typeof part.arguments.url === 'string' ? part.arguments.url : null;
  if (url) {
    const titleMatch = part.output.match(/^(?:#\s*|Title:\s*)(.+)$/m);
    return { heading: '', rows: [{ title: titleMatch?.[1]?.trim() || url, source: host(url), href: url }] };
  }
  return null;
}

function renderResults(results: { heading: string; rows: ResultRow[] }): HTMLElement {
  const shown = results.rows.slice(0, 8);
  return h(
    'div',
    { class: 'results-card' },
    results.heading
      ? h('div', { class: 'results-head' }, icon('search', 15), h('span', { class: 'results-query' }, results.heading), h('span', { class: 'muted' }, `${results.rows.length} ${results.rows.length === 1 ? 'result' : 'results'}`))
      : null,
    shown.map((row) =>
      h(
        row.href ? 'a' : 'div',
        { class: 'result-row', ...(row.href ? { href: row.href, target: '_blank', rel: 'noopener noreferrer' } : {}) },
        icon(row.href ? 'globe' : 'file', 15),
        h('span', { class: 'result-title' }, row.title),
        h('span', { class: 'result-source' }, row.source)
      )
    ),
    results.rows.length > shown.length ? h('div', { class: 'result-more muted' }, `and ${results.rows.length - shown.length} more`) : null
  );
}

function renderToolStep(part: Extract<Part, { kind: 'tool' }>, live: boolean, duration: number | undefined): HTMLElement {
  const label = toolLabel(part.name);
  const hint = argumentHint(part.arguments);
  const running = live && part.output === null;
  const title = label.connector ? `${label.connector} · ${label.action}` : label.action;
  const step = h('div', { class: `step${part.isError ? ' step-error' : ''}` });
  step.append(
    h(
      'div',
      { class: 'step-row' },
      h('span', { class: `step-icon${running ? ' spin' : ''}` }, icon(running ? 'wait' : label.icon, 16)),
      h('span', { class: 'step-title' }, title),
      hint && !resultRows(part)?.heading ? h('span', { class: 'step-hint' }, hint) : null,
      part.isError ? h('span', { class: 'step-badge' }, 'failed') : null,
      seconds(duration) ? h('span', { class: 'step-time' }, seconds(duration)) : null
    )
  );
  const results = resultRows(part);
  if (results) {
    step.append(h('div', { class: 'step-content' }, renderResults(results)));
  }
  // The raw call, for anyone who wants to see exactly what ran.
  const raw = h('details', { class: 'step-raw' });
  const args = JSON.stringify(part.arguments, null, 2);
  raw.append(h('summary', {}, results ? 'Details' : part.isError ? 'Error details' : 'Input and result'));
  if (args !== '{}') raw.append(h('div', { class: 'step-label' }, 'Input'), h('pre', { class: 'step-pre' }, args));
  if (part.output !== null) {
    const output = part.output.length > 6000 ? `${part.output.slice(0, 6000)}\n…` : part.output;
    raw.append(h('div', { class: 'step-label' }, part.isError ? 'Error' : 'Result'), h('pre', { class: 'step-pre' }, output));
  }
  if (args !== '{}' || part.output !== null) step.append(h('div', { class: 'step-content' }, raw));
  return step;
}

function renderReasoning(text: string, live: boolean, duration: number | undefined): HTMLElement {
  const took = seconds(duration);
  return h(
    'div',
    { class: 'step' },
    h(
      'div',
      { class: 'step-row' },
      h('span', { class: 'step-icon' }, icon(live ? 'brain' : 'checkCircle', 16)),
      h('span', { class: 'step-title' }, live ? 'Thinking' : took ? `Thought for ${took}` : 'Thought')
    ),
    h('div', { class: 'step-content reasoning-text' }, text.trim())
  );
}

/** The header of a collapsed activity group: what was done last, and how much. */
function groupSummary(parts: Part[], live: boolean): string {
  const last = parts[parts.length - 1];
  if (!last) return '';
  const tools = parts.filter((p) => p.kind === 'tool').length;
  // The last tool says most about what happened; reasoning alone is "Thought".
  const lastTool = [...parts].reverse().find((p): p is Extract<Part, { kind: 'tool' }> => p.kind === 'tool');
  let label: string;
  if (live) {
    label = last.kind === 'tool' && last.output === null ? toolLabel(last.name).action : last.kind === 'reasoning' ? 'Thinking' : 'Working';
  } else {
    label = lastTool ? toolLabel(lastTool.name).action : 'Thought';
  }
  void tools;
  const extra = parts.length > 1 ? ` · ${parts.length} steps` : '';
  return `${label}${extra}`;
}

/**
 * Text, and the activity between it, in the order the model produced them. Each run of activity
 * is one group: open while it is happening, folded under its last step once the answer follows.
 */
function renderParts(parts: Part[], live: boolean, key: string, durations: number[] = []): HTMLElement[] {
  const out: HTMLElement[] = [];
  let group: { parts: Part[]; indices: number[] } | null = null;
  const groups: { parts: Part[]; indices: number[] }[] = [];
  const flush = () => {
    if (!group) return;
    const g = group;
    const groupIndex = groups.length;
    groups.push(g);
    const isTail = live && g.indices[g.indices.length - 1] === parts.length - 1;
    const groupKey = `${key}:${groupIndex}`;
    const open = groupOpen.get(groupKey) ?? isTail;
    const details = h('details', { class: 'activity', open });
    details.addEventListener('toggle', () => groupOpen.set(groupKey, details.open));
    const timeline = h('div', { class: 'timeline' });
    g.parts.forEach((p, j) => {
      const index = g.indices[j]!;
      const stepLive = isTail && index === parts.length - 1;
      timeline.append(p.kind === 'reasoning' ? renderReasoning(p.text, stepLive, durations[index]) : p.kind === 'tool' ? renderToolStep(p, live, durations[index]) : h('span'));
    });
    details.append(h('summary', {}, h('span', { class: 'activity-label' }, groupSummary(g.parts, isTail)), icon('chevron', 14)), timeline);
    out.push(details);
    group = null;
  };
  parts.forEach((part, i) => {
    if (part.kind === 'text') {
      flush();
      const body = h('div', { class: 'markdown' });
      body.append(markdown(part.text));
      if (live && i === parts.length - 1) body.classList.add('streaming');
      out.push(body);
      return;
    }
    group ??= { parts: [], indices: [] };
    group.parts.push(part);
    group.indices.push(i);
  });
  flush();
  return out;
}

function textOf(parts: Part[]): string {
  return parts
    .filter((p): p is Extract<Part, { kind: 'text' }> => p.kind === 'text')
    .map((p) => p.text)
    .join('\n\n');
}

function renderUserTurn(turn: Turn): HTMLElement {
  const text = textOf(turn.parts);
  const copy = h('button', { class: 'icon-btn small', title: 'Copy' }, icon('copy', 15));
  copy.addEventListener('click', () => void navigator.clipboard.writeText(text));
  return h(
    'div',
    { class: 'turn turn-user' },
    h('div', { class: 'bubble' }, text),
    h('div', { class: 'turn-actions user-actions' }, copy, h('span', { class: 'turn-time' }, timeLabel(turn.time)))
  );
}

function renderAssistantTurn(conversation: Conversation, turn: Turn, isLast: boolean, running: boolean): HTMLElement {
  const durations = isLast ? stepDurations.get(conversation.id) : undefined;
  const el = h('div', { class: 'turn turn-assistant' }, ...renderParts(turn.parts, false, `${conversation.id}:${turn.index}`, durations));
  if (!isLast || !running) el.append(renderActions(conversation, turn, isLast));
  return el;
}

function renderActions(conversation: Conversation, turn: Turn, isLast: boolean): HTMLElement {
  const rate = (value: number) => async () => {
    const next = turn.rating === value ? 0 : value;
    turn.rating = next;
    await api.updateConversation({ id: conversation.id, rating: [turn.index, next] });
    changed();
    viewCache.chat?.refresh(false);
  };
  const copy = h('button', { class: 'icon-btn small', title: 'Copy' }, icon('copy', 16));
  copy.addEventListener('click', () => {
    void navigator.clipboard.writeText(textOf(turn.parts));
    copy.replaceChildren(icon('check', 16));
    setTimeout(() => copy.replaceChildren(icon('copy', 16)), 1200);
  });
  const actions = h(
    'div',
    { class: 'turn-actions' },
    h('button', { class: `icon-btn small${turn.rating > 0 ? ' active-good' : ''}`, title: 'Good reply', on: { click: rate(1) } }, icon('thumbUp', 16)),
    h('button', { class: `icon-btn small${turn.rating < 0 ? ' active-bad' : ''}`, title: 'Bad reply', on: { click: rate(-1) } }, icon('thumbDown', 16)),
    isLast ? h('button', { class: 'icon-btn small', title: 'Regenerate', on: { click: () => void regenerate(conversation.id) } }, icon('retry', 16)) : null,
    copy,
    h('span', { class: 'spacer' }),
    h('span', { class: 'turn-time' }, timeLabel(turn.time))
  );
  return actions;
}

function renderLiveTurn(conversationId: string, live: LiveTurn): HTMLElement {
  const now = Date.now();
  const durations = live.starts.map((start, i) => (live.ends[i] ?? now) - start);
  const el = h('div', { class: 'turn turn-assistant live' }, ...renderParts(live.parts, !live.error, `${conversationId}:live`, durations));
  const busy = !live.error && !live.question;
  if (live.question) {
    el.append(h('div', { class: 'status-line' }, icon('wait', 16), 'Waiting for your answer'));
  } else if (live.pending) {
    const label = toolLabel(live.pending);
    el.append(h('div', { class: 'status-line' }, h('span', { class: 'dots' }), `${label.connector ? `${label.connector} · ` : ''}${label.action}…`));
  } else if (busy && (live.parts.length === 0 || live.parts[live.parts.length - 1]?.kind !== 'text')) {
    el.append(h('div', { class: 'status-line' }, h('span', { class: 'dots' }), 'Thinking'));
  }
  if (live.error) {
    el.append(h('div', { class: 'error-card' }, icon('close', 16), h('div', {}, h('strong', {}, 'The reply failed'), h('div', {}, live.error))));
  }
  return el;
}

// ---------------------------------------------------------------------------------------------
// Sending

async function regenerate(conversationId: string) {
  const model = state.model;
  if (!model) return void toast('Choose a model first.', 'error');
  try {
    await api.send({ conversationId, text: '', model, speed: state.speed, approval: state.approval });
  } catch (e) {
    toast(String(e), 'error');
  }
}

async function send(text: string, conversationId: string | null) {
  const model = state.model;
  if (!model) {
    toast('Add a provider and choose a model first.', 'error');
    void openSettings('providers');
    return false;
  }
  const id = conversationId ?? newId();
  const projectId = state.view.kind === 'home' ? state.view.projectId ?? null : null;
  const workdir = !conversationId && state.mode === 'code' ? state.workdir : null;
  if (!conversationId) {
    pendingUser.set(id, text);
    navigate({ kind: 'chat', id });
  } else {
    viewCache.chat?.addOptimistic(text);
  }
  try {
    await api.send({ conversationId: id, text, model, speed: state.speed, approval: state.approval, projectId, workdir });
    void refreshConversations();
  } catch (e) {
    toast(String(e), 'error');
    return false;
  }
  return true;
}

/** Messages typed on the home screen, shown before the new conversation is on disk. */
const pendingUser = new Map<string, string>();

export async function refreshConversations() {
  state.conversations = await api.conversations();
  changed();
}

// ---------------------------------------------------------------------------------------------
// Composer

interface Attachment {
  name: string;
  text: string;
}

class Composer {
  readonly el: HTMLElement;
  private textarea: HTMLTextAreaElement;
  private attachments: Attachment[] = [];
  private chips: HTMLElement;
  private bar: HTMLElement;
  private conversationId: string | null = null;
  private placeholder = 'Ask anything. Type / for quick actions';

  constructor() {
    this.textarea = h('textarea', { class: 'composer-input', rows: 1, placeholder: this.placeholder });
    this.textarea.addEventListener('input', () => {
      this.autosize();
      this.renderBar();
    });
    this.textarea.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
        e.preventDefault();
        void this.submit();
      }
    });
    this.chips = h('div', { class: 'chips' });
    this.bar = h('div', { class: 'composer-bar' });
    this.el = h('div', { class: 'composer' }, h('div', { class: 'composer-box' }, this.chips, this.textarea, this.bar));
    this.renderBar();
  }

  attach(conversationId: string | null, placeholder: string) {
    this.conversationId = conversationId;
    this.placeholder = placeholder;
    this.textarea.placeholder = placeholder;
    this.renderBar();
  }

  focus() {
    this.textarea.focus();
  }

  setText(text: string) {
    this.textarea.value = text;
    this.autosize();
    this.renderBar();
  }

  private autosize() {
    this.textarea.style.height = 'auto';
    this.textarea.style.height = `${Math.min(this.textarea.scrollHeight, 260)}px`;
  }

  private running(): boolean {
    return this.conversationId !== null && state.live.has(this.conversationId) && !state.live.get(this.conversationId)?.error;
  }

  async submit() {
    if (this.running()) return;
    let text = this.textarea.value.trim();
    if (!text && this.attachments.length === 0) return;
    for (const file of this.attachments) {
      text += `\n\n<attachment name="${file.name}">\n${file.text}\n</attachment>`;
    }
    const ok = await send(text.trim(), this.conversationId);
    if (ok) {
      this.textarea.value = '';
      this.attachments = [];
      this.autosize();
      this.renderChips();
      this.renderBar();
    }
  }

  private renderChips() {
    clear(
      this.chips,
      this.attachments.map((file, i) =>
        h(
          'span',
          { class: 'chip' },
          icon('file', 14),
          file.name,
          h('button', { class: 'chip-x', title: 'Remove', on: { click: () => { this.attachments.splice(i, 1); this.renderChips(); this.renderBar(); } } }, icon('close', 12))
        )
      )
    );
  }

  private pickFiles() {
    const input = h('input', { type: 'file', multiple: true });
    input.addEventListener('change', async () => {
      for (const file of Array.from(input.files ?? [])) {
        if (file.size > 512 * 1024) {
          toast(`${file.name} is larger than 512 KB.`, 'error');
          continue;
        }
        const text = await file.text();
        if (text.includes('\u0000')) {
          toast(`${file.name} is not a text file.`, 'error');
          continue;
        }
        this.attachments.push({ name: file.name, text });
      }
      this.renderChips();
      this.renderBar();
    });
    input.click();
  }

  renderBar() {
    const running = this.running();
    const plus = h('button', { class: 'icon-btn', title: 'Add' }, icon('plus', 18));
    plus.addEventListener('click', () => this.openPlusMenu(plus));

    const approvalIcon = state.approval === 'ask' ? 'hand' : 'pencil';
    const approval = h('button', { class: 'icon-btn tinted', title: state.approval === 'ask' ? 'Ask before changes' : 'Accept edits' }, icon(approvalIcon, 17));
    approval.addEventListener('click', () => {
      const option = (value: 'ask' | 'accept_edits', name: string, desc: string, ic: string) =>
        h(
          'button',
          { class: `menu-item rich${state.approval === value ? ' selected' : ''}`, on: { click: () => { state.approval = value; close(); this.renderBar(); } } },
          icon(ic, 18),
          h('span', {}, h('strong', {}, name), h('small', {}, desc))
        );
      const close = popover(
        approval,
        h(
          'div',
          { class: 'menu' },
          option('ask', 'Ask', 'Ask before every change to your files or system', 'hand'),
          option('accept_edits', 'Accept edits', 'Approve file edits automatically; still ask for the rest', 'pencil')
        ),
        'above'
      );
    });

    const model = currentModel();
    const speed = h('button', { class: 'pill-btn', title: 'Response mode' }, state.speed === 'fast' ? 'Fast' : 'Think', icon('chevron', 14));
    speed.addEventListener('click', () => {
      const option = (value: 'fast' | 'think', name: string, desc: string, ic: string) =>
        h(
          'button',
          { class: `menu-item rich${state.speed === value ? ' selected' : ''}`, on: { click: () => { state.speed = value; close(); this.renderBar(); } } },
          icon(ic, 18),
          h('span', {}, h('strong', {}, name), h('small', {}, desc))
        );
      const close = popover(
        speed,
        h(
          'div',
          { class: 'menu' },
          option('fast', 'Fast', 'Quick answers', 'bolt'),
          option('think', 'Think', model && !model.reasoning ? 'This model does not reason; replies are unchanged' : 'Reasons longer for complex tasks', 'brain')
        ),
        'above'
      );
    });

    const modelBtn = h('button', { class: 'pill-btn model-btn', title: 'Model' }, model ? model.name : state.models.length ? 'Choose model' : 'Add a provider', icon('chevron', 14));
    modelBtn.addEventListener('click', () => (state.models.length ? openModelPicker(modelBtn, () => this.renderBar()) : void openSettings('providers')));

    const hasText = this.textarea.value.trim().length > 0 || this.attachments.length > 0;
    const action = running
      ? h('button', { class: 'send-btn stop', title: 'Stop', on: { click: () => this.conversationId && void api.cancel(this.conversationId) } }, icon('stop', 16))
      : h('button', { class: 'send-btn', title: 'Send', disabled: !hasText, on: { click: () => void this.submit() } }, icon('send', 18));

    clear(this.bar, h('div', { class: 'bar-left' }, plus, approval), h('div', { class: 'bar-right' }, speed, modelBtn, action));
  }

  private openPlusMenu(anchor: HTMLElement) {
    const item = (ic: string, label: string, onClick: () => void, sub = false) =>
      h('button', { class: 'menu-item', on: { click: () => { close(); onClick(); } } }, icon(ic, 17), h('span', {}, label), sub ? icon('chevronRight', 15) : null);
    const close = popover(
      anchor,
      h(
        'div',
        { class: 'menu' },
        item('upload', 'Attach files', () => this.pickFiles()),
        item('folder', 'Give access to a folder', () => void grantFolder()),
        item('plug', 'Connectors', () => navigate({ kind: 'page', page: 'connectors' }), true),
        item('book', 'Instructions', () => void openInstructions(), true),
        item('folder', 'Move to project', () => void moveToProject(this.conversationId), true),
        h('div', { class: 'menu-sep' }),
        item('reset', 'Clear input', () => { this.setText(''); this.attachments = []; this.renderChips(); })
      ),
      'below'
    );
  }
}

export const composer = new Composer();

async function grantFolder() {
  const dir = await api.pickDirectory();
  if (!dir) return;
  const settings = await api.appSettings();
  if (!settings.filesystem_allowed_roots.includes(dir)) settings.filesystem_allowed_roots.push(dir);
  await api.saveAppSettings(settings);
  toast(`The assistant can now read ${dir}. Allow writing in Settings → Permissions.`);
}

async function moveToProject(conversationId: string | null) {
  const anchorProjects = state.projects;
  if (!conversationId) {
    toast('Send a message first, then move the conversation.');
    return;
  }
  const choices = h('div', { class: 'menu' });
  let resolveDone: () => void = () => {};
  const done = new Promise<void>((r) => (resolveDone = r));
  void modal((close) => {
    const pick = (projectId: string | null) => async () => {
      await api.updateConversation({ id: conversationId, projectId });
      await refreshConversations();
      close();
      resolveDone();
    };
    anchorProjects.forEach((p) => choices.append(h('button', { class: 'menu-item', on: { click: pick(p.id) } }, icon('folder', 17), p.name)));
    choices.append(h('button', { class: 'menu-item', on: { click: pick(null) } }, icon('close', 17), 'No project'));
    return h('div', { class: 'dialog-small' }, h('h2', {}, 'Move to project'), choices);
  });
  await done;
}

function openModelPicker(anchor: HTMLElement, onPick: () => void) {
  const search = h('input', { class: 'input small', placeholder: 'Search models' });
  const list = h('div', { class: 'model-list' });
  const render = () => {
    const q = search.value.toLowerCase();
    const groups = new Map<string, typeof state.models>();
    for (const m of state.models) {
      if (q && !`${m.name} ${m.id} ${m.providerLabel}`.toLowerCase().includes(q)) continue;
      const group = groups.get(m.providerLabel) ?? [];
      group.push(m);
      groups.set(m.providerLabel, group);
    }
    clear(list);
    for (const [label, models] of groups) {
      list.append(h('div', { class: 'menu-heading' }, label));
      for (const m of models) {
        list.append(
          h(
            'button',
            {
              class: `menu-item model-item${m.reference === state.model ? ' selected' : ''}`,
              on: {
                click: () => {
                  state.model = m.reference;
                  void api.setDefaultModel(m.reference);
                  close();
                  onPick();
                },
              },
            },
            h('span', { class: 'model-name' }, m.name),
            m.reasoning ? h('span', { class: 'tag' }, 'reasoning') : null,
            m.reference === state.model ? icon('check', 15) : null
          )
        );
      }
    }
    if (!groups.size) list.append(h('div', { class: 'empty-small' }, 'No model matches.'));
  };
  search.addEventListener('input', render);
  render();
  const close = popover(
    anchor,
    h('div', { class: 'menu model-menu' }, search, list, h('div', { class: 'menu-sep' }), h('button', { class: 'menu-item', on: { click: () => { close(); void openSettings('providers'); } } }, icon('settings', 16), 'Manage providers')),
    'above'
  );
  setTimeout(() => search.focus());
}

// ---------------------------------------------------------------------------------------------
// Question and approval cards

function renderQuestion(conversationId: string, q: NonNullable<LiveTurn['question']>): HTMLElement {
  const answer = (text: string) => {
    void api.answer(q.callId, text);
    const live = state.live.get(conversationId);
    if (live) live.question = null;
    changed();
  };
  const input = h('input', { class: 'question-input', placeholder: 'Type your answer…' });
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && input.value.trim()) answer(input.value.trim());
  });
  const card = h(
    'div',
    { class: 'question-card' },
    h('div', { class: 'question-head' }, h('strong', {}, q.question), h('button', { class: 'icon-btn small', title: 'Dismiss', on: { click: () => answer('(The user dismissed the question; continue with your best judgement.)') } }, icon('close', 16))),
    ...q.options.map((option, i) =>
      h('button', { class: 'question-option', on: { click: () => answer(option) } }, h('span', { class: 'num' }, String(i + 1)), h('span', {}, option), icon('send', 15))
    ),
    h('div', { class: 'question-option free' }, h('span', { class: 'num' }, String(q.options.length + 1)), input)
  );
  card.addEventListener('keydown', (e) => {
    if (e.target === input) return;
    const n = Number(e.key);
    if (n >= 1 && n <= q.options.length) answer(q.options[n - 1]!);
  });
  setTimeout(() => input.focus());
  return card;
}

function renderApprovals(): HTMLElement | null {
  const request = state.approvals[0];
  if (!request) return null;
  const fullName = request.connector_id ? `${request.connector_id}__${request.tool_name}` : request.tool_name;
  const tool = toolLabel(fullName);
  const connector = CONNECTOR_LABELS[request.connector_id ?? ''] ?? humanize((request.connector_id ?? '').replace(/^mcp-/, ''));
  const asking = REQUESTS[fullName] ? `Allow ${REQUESTS[fullName]}?` : `Allow ${connector ? `${connector} to use ` : ''}${humanize(request.tool_name)}?`;
  const respond = (approved: boolean, always = false) => {
    void api.respondApproval(request.request_id, approved, always);
    state.approvals.shift();
    changed();
  };
  const reason = request.reason
    ? h('p', { class: 'muted' }, `This conversation has read content from ${request.reason.sources.join(', ') || 'an untrusted source'}, so the call needs your approval.`)
    : null;
  return h(
    'div',
    { class: 'approval-card' },
    h('div', { class: 'approval-head' }, icon(tool.icon, 18), h('strong', {}, asking)),
    reason,
    h('pre', { class: 'step-pre' }, JSON.stringify(request.input, null, 2)),
    h(
      'div',
      { class: 'dialog-actions' },
      h('button', { class: 'btn btn-ghost', on: { click: () => respond(false) } }, 'Deny'),
      h('button', { class: 'btn btn-ghost', on: { click: () => respond(true, true) } }, 'Always allow'),
      h('button', { class: 'btn btn-primary', on: { click: () => respond(true) } }, 'Allow once')
    )
  );
}

// ---------------------------------------------------------------------------------------------
// Views

const SUGGESTIONS = [
  { icon: 'sparkle', text: 'Explain a concept like I am new to it' },
  { icon: 'globe', text: 'Summarize this web page for me: ' },
  { icon: 'file', text: 'Look through my documents folder and tell me what is there' },
  { icon: 'plug', text: 'Connect your apps for better answers', page: true },
];

export class HomeView {
  readonly el = h('div', { class: 'home' });
  readonly key: string;
  constructor(private projectId: string | null) {
    this.key = `home:${projectId ?? ''}:${state.mode}`;
    this.build();
  }

  build() {
    const project = state.projects.find((p) => p.id === this.projectId);
    const code = state.mode === 'code';
    const title = code
      ? h(
          'h1',
          { class: 'greeting' },
          'Ready for a session in ',
          h('button', { class: 'folder-pick', on: { click: () => void pickWorkdir() } }, icon('folder', 26), state.workdir ? state.workdir.split(/[\\/]/).pop() ?? state.workdir : 'Choose a folder'),
          '?'
        )
      : h('h1', { class: 'greeting' }, project ? `New chat in ${project.name}` : greeting(state.instructions.name));
    composer.attach(null, code ? 'What do you want to build?' : 'Ask anything. Type / for quick actions');
    const banner =
      state.models.length === 0
        ? h('div', { class: 'composer-banner' }, icon('key', 16), h('span', {}, 'Add an API key or a local server to start chatting'), h('button', { class: 'link-btn', on: { click: () => void openSettings('providers') } }, 'Add provider'))
        : null;
    const suggestions = code
      ? null
      : h(
          'div',
          { class: 'suggestions' },
          SUGGESTIONS.map((s) =>
            h(
              'button',
              {
                class: 'suggestion',
                on: { click: () => (s.page ? navigate({ kind: 'page', page: 'connectors' }) : (composer.setText(s.text), composer.focus())) },
              },
              icon(s.icon, 17),
              s.text.replace(/: $/, '')
            )
          )
        );
    clear(
      this.el,
      h('div', { class: 'home-inner' }, h('div', { class: 'brand-mark' }), title, h('div', { class: 'composer-wrap' }, banner, composer.el), suggestions)
    );
    setTimeout(() => composer.focus());
  }

  update() {
    composer.renderBar();
  }
}

async function pickWorkdir() {
  const dir = await api.pickDirectory();
  if (!dir) return;
  const settings = await api.appSettings();
  if (!settings.filesystem_allowed_roots.includes(dir)) settings.filesystem_allowed_roots.push(dir);
  settings.filesystem_write_enabled = true;
  settings.bash_enabled = true;
  await api.saveAppSettings(settings);
  state.workdir = dir;
  try {
    localStorage.setItem('workdir', dir);
  } catch {
    /* per-device convenience only */
  }
  viewCache.home?.build();
}

export class ChatView {
  readonly el = h('div', { class: 'chat' });
  readonly key: string;
  private conversation: Conversation | null = null;
  private header = h('header', { class: 'chat-header' });
  private scroller = h('div', { class: 'chat-scroll' });
  private transcript = h('div', { class: 'transcript' });
  private liveSlot = h('div', {});
  private overlay = h('div', { class: 'composer-overlay' });
  private bottom = h('div', { class: 'chat-bottom' });
  private jump = h('button', { class: 'jump-btn', title: 'Scroll to bottom' }, icon('arrowDown', 18));
  private stickToBottom = true;

  constructor(private id: string) {
    this.key = `chat:${id}`;
    this.scroller.append(h('div', { class: 'chat-column' }, this.transcript, this.liveSlot));
    this.scroller.addEventListener('scroll', () => {
      const distance = this.scroller.scrollHeight - this.scroller.scrollTop - this.scroller.clientHeight;
      this.stickToBottom = distance < 80;
      this.jump.classList.toggle('visible', distance > 300);
    });
    this.jump.addEventListener('click', () => this.scrollToBottom(true));
    composer.attach(id, 'Reply…');
    this.bottom.append(this.jump, this.overlay, composer.el, h('div', { class: 'disclaimer' }, 'AI can make mistakes. Check important information.'));
    this.el.append(this.header, this.scroller, this.bottom);
    void this.refresh(true);
  }

  async refresh(scroll: boolean) {
    try {
      this.conversation = await api.conversation(this.id);
      pendingUser.delete(this.id);
    } catch {
      this.conversation = { id: this.id, title: '', projectId: null, workdir: null, model: null, pinned: false, turns: [], running: true };
    }
    const optimistic = pendingUser.get(this.id);
    if (optimistic && this.conversation.turns.length === 0) {
      this.conversation.turns.push({ role: 'user', parts: [{ kind: 'text', text: optimistic }], index: -1, time: new Date().toISOString(), rating: 0 });
    }
    this.renderTranscript();
    this.update();
    if (scroll) this.scrollToBottom(false);
  }

  addOptimistic(text: string) {
    this.conversation?.turns.push({ role: 'user', parts: [{ kind: 'text', text }], index: -1, time: new Date().toISOString(), rating: 0 });
    this.renderTranscript();
    this.scrollToBottom(true);
  }

  renderTranscript() {
    const c = this.conversation;
    if (!c) return;
    const running = state.live.has(this.id);
    const last = c.turns.length - 1;
    clear(
      this.transcript,
      c.turns.map((turn, i) => (turn.role === 'user' ? renderUserTurn(turn) : renderAssistantTurn(c, turn, i === last, running)))
    );
  }

  private renderHeader() {
    const c = this.conversation;
    const summary = state.conversations.find((s) => s.id === this.id);
    const title = summary?.title || c?.title || 'New chat';
    const project = state.projects.find((p) => p.id === (summary?.projectId ?? c?.projectId));
    const workdir = summary?.workdir ?? c?.workdir;
    const titleBtn = h('button', { class: 'crumb-title' }, title, icon('chevron', 14));
    titleBtn.addEventListener('click', () => conversationMenu(titleBtn, this.id, title, summary?.pinned ?? false));
    const pin = h('button', { class: `icon-btn${summary?.pinned ? ' active' : ''}`, title: summary?.pinned ? 'Unpin' : 'Pin' }, icon('pin', 18));
    pin.addEventListener('click', async () => {
      await api.updateConversation({ id: this.id, pinned: !summary?.pinned });
      await refreshConversations();
    });
    const exportBtn = h('button', { class: 'icon-btn', title: 'Export as Markdown' }, icon('share', 18));
    exportBtn.addEventListener('click', () => void api.exportConversation(this.id).then((saved) => saved && toast('Conversation exported.')));
    clear(
      this.header,
      h(
        'div',
        { class: 'crumbs' },
        project ? h('button', { class: 'crumb', on: { click: () => navigate({ kind: 'home', projectId: project.id }) } }, icon('folder', 15), project.name) : null,
        workdir ? h('span', { class: 'crumb' }, icon('terminal', 15), workdir.split(/[\\/]/).pop() ?? workdir) : null,
        project || workdir ? h('span', { class: 'crumb-sep' }, '/') : null,
        titleBtn
      ),
      h('div', { class: 'header-actions' }, pin, exportBtn)
    );
  }

  update() {
    this.renderHeader();
    const live = state.live.get(this.id);
    clear(this.liveSlot, live ? renderLiveTurn(this.id, live) : null);
    const approval = renderApprovals();
    const question = live?.question ? renderQuestion(this.id, live.question) : null;
    clear(this.overlay, approval, question);
    composer.el.classList.toggle('hidden', Boolean(question));
    composer.renderBar();
    if (this.stickToBottom) this.scrollToBottom(false);
  }

  /** The turn ended: the stored conversation replaces the live one. */
  async finished() {
    await this.refresh(false);
  }

  private scrollToBottom(smooth: boolean) {
    requestAnimationFrame(() => this.scroller.scrollTo({ top: this.scroller.scrollHeight, behavior: smooth ? 'smooth' : 'auto' }));
  }
}

export function conversationMenu(anchor: HTMLElement, id: string, title: string, pinned: boolean) {
  const item = (ic: string, label: string, action: () => void, danger = false) =>
    h('button', { class: `menu-item${danger ? ' danger' : ''}`, on: { click: () => { close(); action(); } } }, icon(ic, 16), label);
  const close = popover(
    anchor,
    h(
      'div',
      { class: 'menu' },
      item('edit', 'Rename', async () => {
        const next = await promptText('Rename conversation', title);
        if (next) {
          await api.updateConversation({ id, title: next });
          await refreshConversations();
        }
      }),
      item('pin', pinned ? 'Unpin' : 'Pin', async () => {
        await api.updateConversation({ id, pinned: !pinned });
        await refreshConversations();
      }),
      item('folder', 'Move to project', () => void moveToProject(id)),
      item('download', 'Export as Markdown', () => void api.exportConversation(id).then((saved) => saved && toast('Conversation exported.'))),
      h('div', { class: 'menu-sep' }),
      item('trash', 'Delete', async () => {
        if (await confirmDialog('Delete conversation?', `“${title}” will be deleted from this device.`)) {
          await api.deleteConversation(id);
          if (state.view.kind === 'chat' && state.view.id === id) navigate({ kind: 'home' });
          await refreshConversations();
        }
      }, true)
    )
  );
}

/** The views on screen, so streamed events update them in place. */
export const viewCache: { chat: ChatView | null; home: HomeView | null } = { chat: null, home: null };
