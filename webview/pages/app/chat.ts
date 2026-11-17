/** The home screen, a conversation, and the composer they share. */
import { tauriInvoke } from '../../shared/tauri';
import { api, newId, type Conversation, type Part, type Turn, type UsageTotals } from './api';
import { clear, confirmDialog, greeting, h, icon, markdown, modal, popover, promptText, timeLabel, toast } from './dom';
import { changed, currentModel, navigate, state, type LiveTurn } from './state';
import { openSettings } from './settings';
import { openInstructions } from './pages';
import { closePanel, enableAskAboutSelection, renderCanvasCard, setPanelHost } from './canvas';
import { renderCodeRun, renderImage } from './coderun';
import { startRecording, toBase64, type Recording } from './dictation';
import { audioSettings, speakText, type Playback } from './audio';
import { editWorkflow, workflowPrompt, workflowsApi } from './workflows';
import type { Workflow } from './api';
import { diffLines, loadChanges, openChanges } from './changes';
import { openTerminal } from './terminal';
import { button, chip, iconButton, modalHeader, trigger } from './ui';

// ---------------------------------------------------------------------------------------------
// Tool labels

const CONNECTOR_LABELS: Record<string, string> = {
  filesystem: 'Files',
  bash: 'Terminal',
  sandbox: 'Code',
  terminal: 'Terminal',
  browser: 'Web',
  proton_mail: 'Mail',
  luminance: 'Luminance',
  tool_results: 'Saved output',
};

const CONNECTOR_ICONS: Record<string, string> = {
  filesystem: 'file',
  bash: 'terminal',
  sandbox: 'terminal',
  terminal: 'terminal',
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
  web_search: 'Searched the web',
  remember: 'Saved to knowledge',
  forget: 'Removed from knowledge',
  use_skill: 'Used a skill',
  update_plan: 'Updated the plan',
  knowledge_write: 'Saved to knowledge',
  terminal_start: 'Started a background command',
  terminal_read: 'Read command output',
  terminal_stop: 'Stopped a command',
  knowledge_read: 'Read a knowledge page',
  knowledge_search: 'Searched knowledge',
  save_skill: 'Saved a skill',
  canvas: 'Wrote a canvas',
  delegate: 'Ran sub-agents',
  schedule_task: 'Scheduled a task',
  search_library: 'Searched your libraries',
};

const BUILTIN_ICONS: Record<string, string> = {
  web_search: 'search',
  remember: 'book',
  forget: 'book',
  use_skill: 'skill',
  update_plan: 'check',
  canvas: 'file',
  delegate: 'workflow',
  schedule_task: 'clock',
  search_library: 'library',
  run_code: 'terminal',
  knowledge_write: 'book',
  terminal_start: 'terminal',
  terminal_read: 'terminal',
  terminal_stop: 'terminal',
  knowledge_read: 'book',
  knowledge_search: 'book',
  save_skill: 'skill',
  generate_image: 'sparkle',
};

/** What an approval card asks to allow. */
const REQUESTS: Record<string, string> = {
  filesystem__fs_write: 'writing a file',
  filesystem__fs_edit: 'editing a file',
  bash__run: 'running a command',
  sandbox__run_code: 'running this code',
  terminal__start: 'starting a background command',
  browser__fetch: 'sending a web request',
  browser__navigate: 'opening a page',
};

function humanize(name: string): string {
  const words = name.replace(/[_-]+/g, ' ').trim();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

export function toolLabel(name: string): { connector: string; action: string; icon: string } {
  if (name === 'ask_user') return { connector: '', action: 'Asked you', icon: 'chat' };
  if (BUILTIN_ICONS[name]) return { connector: '', action: ACTIONS[name] ?? humanize(name), icon: BUILTIN_ICONS[name]! };
  const [connector = '', tool = name] = name.includes('__') ? name.split(/__(.*)/s) : ['', name];
  const label = CONNECTOR_LABELS[connector] ?? humanize(connector.replace(/^mcp-/, ''));
  const known = ACTIONS[name];
  return { connector: known ? '' : label, action: known ?? humanize(tool), icon: CONNECTOR_ICONS[connector] ?? 'tool' };
}

/** The argument worth showing next to a tool step: a path, a URL, a query or a command. */
function argumentHint(args: Record<string, unknown>): string {
  for (const key of ['path', 'url', 'query', 'fact', 'command', 'script', 'pattern', 'question', 'name']) {
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

/**
 * Whether the user opened or closed an activity group, and whether the group was still running
 * when they did. A choice made while the model works lasts until its result arrives, so finished
 * steps always fold away under the answer; a choice made afterwards sticks.
 */
const groupOpen = new Map<string, { open: boolean; running: boolean }>();

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

/** A page known only by its URL reads as its path; the host is in its own column. */
function pagePath(url: string): string {
  try {
    const u = new URL(url);
    const path = decodeURIComponent(u.pathname).replace(/\/$/, '');
    return path ? `${path}${u.search}` : u.hostname;
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
    const list = ['results', 'passages', 'items', 'matches', 'links', 'files'].map((k) => record[k]).find(Array.isArray) as unknown[] | undefined;
    if (list) {
      const rows = list
        .filter((x): x is Record<string, unknown> => Boolean(x) && typeof x === 'object')
        .map((x) => {
          const href = [x.url, x.href, x.link].find((v): v is string => typeof v === 'string');
          const path = typeof x.path === 'string' ? x.path : '';
          const title = [x.title, x.document, x.name, x.text, path, href].find((v): v is string => typeof v === 'string' && v.length > 0) ?? '';
          const source = href ? host(href) : typeof x.library === 'string' ? x.library : path ? path.split(/[\\/]/).slice(-2, -1)[0] ?? '' : '';
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
    return { heading: '', rows: [{ title: titleMatch?.[1]?.trim() || pagePath(url), source: host(url), href: url }] };
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

/** How a run of the same tool reads, by count. */
const RUN_LABELS: Record<string, (n: number) => string> = {
  browser__navigate: (n) => `Browsed ${n} pages`,
  browser__fetch: (n) => `Fetched ${n} pages`,
  web_search: (n) => `Searched the web · ${n} queries`,
  filesystem__fs_read: (n) => `Read ${n} files`,
  filesystem__fs_search: (n) => `Searched files · ${n} searches`,
  filesystem__fs_write: (n) => `Wrote ${n} files`,
  filesystem__fs_edit: (n) => `Edited ${n} files`,
  bash__run: (n) => `Ran ${n} commands`,
  remember: (n) => `Saved ${n} facts to knowledge`,
};

/** One table row per call: the page or file it touched, or the query and its top hits. */
function runRows(part: Extract<Part, { kind: 'tool' }>, running: boolean): HTMLElement[] {
  const results = resultRows(part);
  const status = running ? h('span', { class: 'step-icon spin inline' }, icon('wait', 14)) : part.isError ? h('span', { class: 'step-badge' }, 'failed') : null;
  if (results?.heading) {
    return [
      h('div', { class: 'results-head' }, icon('search', 15), h('span', { class: 'results-query' }, results.heading), h('span', { class: 'muted' }, `${results.rows.length} results`)),
      ...results.rows.slice(0, 3).map((row) =>
        h(row.href ? 'a' : 'div', { class: 'result-row', ...(row.href ? { href: row.href, target: '_blank', rel: 'noopener noreferrer' } : {}) }, icon(row.href ? 'globe' : 'file', 15), h('span', { class: 'result-title' }, row.title), h('span', { class: 'result-source' }, row.source))
      ),
    ];
  }
  const row = results?.rows[0];
  const hint = argumentHint(part.arguments) || toolLabel(part.name).action;
  const href = row?.href ?? (typeof part.arguments.url === 'string' ? part.arguments.url : undefined);
  return [
    h(
      href ? 'a' : 'div',
      { class: 'result-row', ...(href ? { href, target: '_blank', rel: 'noopener noreferrer' } : {}) },
      icon(href ? 'globe' : toolLabel(part.name).icon, 15),
      h('span', { class: `result-title${running ? ' shimmer' : ''}` }, row?.title ?? hint),
      status,
      h('span', { class: 'result-source' }, row?.source ?? (href ? host(href) : ''))
    ),
  ];
}

function renderToolRun(run: Extract<Part, { kind: 'tool' }>[], live: boolean, duration: number | undefined): HTMLElement {
  const first = run[0]!;
  const label = toolLabel(first.name);
  const running = live && run.some((p) => p.output === null);
  const failed = run.filter((p) => p.isError).length;
  const title = RUN_LABELS[first.name]?.(run.length) ?? `${label.connector ? `${label.connector} · ` : ''}${label.action} ×${run.length}`;
  const raw = h('details', { class: 'step-raw' }, h('summary', {}, 'Details'));
  run.forEach((p, i) => {
    raw.append(h('div', { class: 'step-label' }, `Call ${i + 1}`), h('pre', { class: 'step-pre' }, JSON.stringify(p.arguments, null, 2)));
    if (p.output !== null) raw.append(h('pre', { class: 'step-pre' }, p.output.length > 3000 ? `${p.output.slice(0, 3000)}\n…` : p.output));
  });
  return h(
    'div',
    { class: 'step' },
    h(
      'div',
      { class: 'step-row' },
      h('span', { class: `step-icon${running ? ' spin' : ''}` }, icon(running ? 'wait' : label.icon, 16)),
      h('span', { class: 'step-title' }, title),
      failed ? h('span', { class: 'step-badge' }, `${failed} failed`) : null,
      seconds(duration) ? h('span', { class: 'step-time' }, seconds(duration)) : null
    ),
    h('div', { class: 'step-content' }, h('div', { class: 'results-card' }, run.flatMap((p) => runRows(p, live && p.output === null)))),
    h('div', { class: 'step-content' }, raw)
  );
}

interface PlanStep {
  text: string;
  status: string;
}

function planOf(part: Extract<Part, { kind: 'tool' }>): PlanStep[] {
  const steps = part.arguments.steps;
  return Array.isArray(steps) ? steps.filter((s): s is PlanStep => Boolean(s) && typeof (s as PlanStep).text === 'string') : [];
}

export function renderChecklist(steps: PlanStep[]): HTMLElement {
  return h(
    'div',
    { class: 'checklist' },
    steps.map((s) =>
      h(
        'div',
        { class: `check-item ${s.status}` },
        h('span', { class: 'check-box' }, s.status === 'done' ? icon('check', 12) : s.status === 'in_progress' ? h('span', { class: 'check-dot' }) : null),
        h('span', {}, s.text)
      )
    )
  );
}

function renderPlanStep(part: Extract<Part, { kind: 'tool' }>): HTMLElement {
  const steps = planOf(part);
  const done = steps.filter((s) => s.status === 'done').length;
  return h(
    'div',
    { class: 'step' },
    h('div', { class: 'step-row' }, h('span', { class: 'step-icon' }, icon('check', 16)), h('span', { class: 'step-title' }, `Plan · ${done}/${steps.length} done`)),
    h('div', { class: 'step-content' }, renderChecklist(steps))
  );
}

interface WorkerReport {
  title: string;
  status: string;
  report: string;
  steps: number;
}

/** A `delegate` call: one row per sub-agent, live while they work, with their reports after. */
function renderDelegate(part: Extract<Part, { kind: 'tool' }>, live: boolean, duration: number | undefined): HTMLElement {
  const tasks = Array.isArray(part.arguments.tasks) ? (part.arguments.tasks as { title?: string; instructions?: string }[]) : [];
  let reports: WorkerReport[] = [];
  try {
    reports = part.output ? (JSON.parse(part.output) as { reports: WorkerReport[] }).reports ?? [] : [];
  } catch {
    reports = [];
  }
  const progress = state.subagents.get(part.callId) ?? [];
  const running = live && part.output === null;
  const doneCount = reports.filter((r) => r.status === 'done').length || progress.filter((p) => p?.status === 'done').length;
  const rows = tasks.map((task, i) => {
    const report = reports[i];
    const now = progress[i];
    const status = report?.status ?? now?.status ?? (running ? 'running' : 'failed');
    const statusIcon = status === 'done' ? icon('checkCircle', 16) : status === 'failed' ? icon('close', 16) : h('span', { class: 'step-icon spin inline' }, icon('wait', 15));
    const row = h('details', { class: `worker worker-${status}` });
    row.append(
      h(
        'summary',
        {},
        h('span', { class: 'worker-status' }, statusIcon),
        h('span', { class: 'worker-title' }, task.title ?? `Task ${i + 1}`),
        h('span', { class: 'worker-activity muted' }, report ? `${report.steps} tool steps` : now?.activity ?? (running ? 'Waiting' : ''))
      )
    );
    const body = h('div', { class: 'worker-body' }, h('div', { class: 'step-label' }, 'Task'), h('p', { class: 'muted' }, task.instructions ?? ''));
    if (report) {
      const md = h('div', { class: 'markdown small' });
      md.append(markdown(report.report));
      body.append(h('div', { class: 'step-label' }, 'Report'), md);
    }
    row.append(body);
    return row;
  });
  return h(
    'div',
    { class: 'step' },
    h(
      'div',
      { class: 'step-row' },
      h('span', { class: `step-icon${running ? ' spin' : ''}` }, icon(running ? 'wait' : 'workflow', 16)),
      h('span', { class: 'step-title' }, running ? `Running ${tasks.length} sub-agents in parallel` : `Ran ${tasks.length} sub-agents · ${doneCount} reported`),
      seconds(duration) ? h('span', { class: 'step-time' }, seconds(duration)) : null
    ),
    h('div', { class: 'step-content' }, h('div', { class: 'workers' }, rows))
  );
}

function renderToolStep(part: Extract<Part, { kind: 'tool' }>, live: boolean, duration: number | undefined): HTMLElement {
  if (part.name === 'delegate') return renderDelegate(part, live, duration);
  if (part.name === 'update_plan') return renderPlanStep(part);
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
      h('span', { class: `step-title${running ? ' shimmer' : ''}` }, title),
      hint && !resultRows(part)?.heading ? h('span', { class: 'step-hint' }, hint) : null,
      part.isError ? h('span', { class: 'step-badge' }, 'failed') : null,
      seconds(duration) ? h('span', { class: 'step-time' }, seconds(duration)) : null
    )
  );
  const results = resultRows(part);
  if (results) {
    step.append(h('div', { class: 'step-content' }, renderResults(results)));
  }
  // A file edit reads best as the lines it took out and put in.
  const before = part.arguments.old_string;
  const after = part.arguments.new_string;
  if (part.name === 'filesystem__fs_edit' && typeof before === 'string' && typeof after === 'string') {
    const lines = [...before.split('\n').map((l) => `-${l}`), ...after.split('\n').map((l) => `+${l}`)];
    step.append(h('div', { class: 'step-content' }, diffLines(lines.length > 80 ? [...lines.slice(0, 80), ` … ${lines.length - 80} more lines`] : lines)));
  }
  // A whole-file write shows what it changed (the tool reports the diff), or a new file's lines.
  if (part.name === 'filesystem__fs_write' && part.output && !part.isError) {
    let written: { diff?: string; created?: boolean } = {};
    try {
      written = JSON.parse(part.output) as typeof written;
    } catch {
      written = {};
    }
    const content = typeof part.arguments.content === 'string' ? part.arguments.content : '';
    const lines = written.diff && written.diff !== 'unchanged'
      ? written.diff.split('\n').filter((l) => !l.startsWith('---') && !l.startsWith('+++'))
      : written.created
        ? content.split('\n').map((l) => `+${l}`)
        : [];
    if (lines.length) step.append(h('div', { class: 'step-content' }, diffLines(lines.length > 80 ? [...lines.slice(0, 80), ` … ${lines.length - 80} more lines`] : lines)));
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

/** What each kind of tool call counts as in a group's header. */
const KINDS: [RegExp, string, string][] = [
  [/^web_search$/, 'search', 'searches'],
  [/^browser__/, 'page', 'pages'],
  [/^filesystem__fs_(read|search)$/, 'file lookup', 'file lookups'],
  [/^filesystem__fs_(write|edit)$/, 'file change', 'file changes'],
  [/^bash__/, 'command', 'commands'],
  [/^(remember|forget)$/, 'memory update', 'memory updates'],
  [/^use_skill$/, 'skill', 'skills'],
  [/^delegate$/, 'sub-agent run', 'sub-agent runs'],
  [/^update_plan$/, 'plan update', 'plan updates'],
  [/^search_library$/, 'library search', 'library searches'],
];

/** The header of a collapsed activity group: how much of what was done, e.g. "12 searches · 30 pages". */
function groupSummary(parts: Part[], live: boolean): string {
  const last = parts[parts.length - 1];
  if (!last) return '';
  if (live) {
    const running = parts.filter((p) => p.kind === 'tool' && p.output === null).length;
    if (running > 1) return `Running ${running} actions`;
    return last.kind === 'tool' && last.output === null ? toolLabel(last.name).action : last.kind === 'reasoning' ? 'Thinking' : 'Working';
  }
  const tools = parts.filter((p): p is Extract<Part, { kind: 'tool' }> => p.kind === 'tool');
  if (!tools.length) return 'Thought';
  if (tools.length === 1) return toolLabel(tools[0]!.name).action;
  const counts = new Map<string, number>();
  let other = 0;
  for (const t of tools) {
    const kind = KINDS.find(([re]) => re.test(t.name));
    if (kind) counts.set(kind[1], (counts.get(kind[1]) ?? 0) + 1);
    else other += 1;
  }
  const labels = KINDS.filter(([, one]) => counts.has(one)).map(([, one, many]) => {
    const n = counts.get(one)!;
    return `${n} ${n === 1 ? one : many}`;
  });
  if (other) labels.push(`${other} ${other === 1 ? 'tool call' : 'tool calls'}`);
  return labels.slice(0, 3).join(' · ');
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
    const choice = groupOpen.get(groupKey);
    const open = choice && choice.running === isTail ? choice.open : isTail;
    const details = h('details', { class: 'activity', open });
    details.addEventListener('toggle', () => groupOpen.set(groupKey, { open: details.open, running: isTail }));
    const timeline = h('div', { class: 'timeline' });
    // Consecutive calls to the same tool read as one step with a table, not a repeated label.
    for (let j = 0; j < g.parts.length; ) {
      const p = g.parts[j]!;
      const index = g.indices[j]!;
      if (p.kind !== 'tool') {
        const stepLive = isTail && index === parts.length - 1;
        if (p.kind === 'reasoning') timeline.append(renderReasoning(p.text, stepLive, durations[index]));
        j += 1;
        continue;
      }
      let end = j + 1;
      while (end < g.parts.length) {
        const next = g.parts[end]!;
        if (next.kind !== 'tool' || next.name !== p.name) break;
        end += 1;
      }
      const run = g.parts.slice(j, end) as Extract<Part, { kind: 'tool' }>[];
      const runDuration = g.indices.slice(j, end).reduce((sum, k) => sum + (durations[k] ?? 0), 0);
      timeline.append(run.length === 1 ? renderToolStep(p, live, durations[index]) : renderToolRun(run, live, runDuration || undefined));
      j = end;
    }
    details.append(h('summary', {}, h('span', { class: 'activity-label' }, groupSummary(g.parts, isTail)), icon('chevron', 14)), timeline);
    out.push(details);
    group = null;
  };
  parts.forEach((part, i) => {
    if (part.kind === 'tool' && part.name === 'canvas') {
      flush();
      out.push(renderCanvasCard(part, live));
      return;
    }
    if (part.kind === 'tool' && part.name === 'generate_image') {
      flush();
      out.push(renderImage(part, live, key.split(':')[0] ?? ''));
      return;
    }
    if (part.kind === 'tool' && part.name === 'run_code') {
      flush();
      out.push(renderCodeRun(part, live, key.split(':')[0] ?? ''));
      return;
    }
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

function renderUserTurn(turn: Turn, conversationId: string, branch: HTMLElement | null = null): HTMLElement {
  const text = textOf(turn.parts);
  const copy = h('button', { class: 'icon-btn small', title: 'Copy' }, icon('copy', 15));
  copy.addEventListener('click', () => void navigator.clipboard.writeText(text));
  // A skill invoked with /name reads as a chip ahead of the message.
  let rest = text;
  const chips: HTMLElement[] = [];
  const steered = rest.match(/^\(Sent while you were working\) /);
  if (steered) {
    chips.push(h('span', { class: 'skill-chip' }, icon('forward', 14), 'Added while working'));
    rest = rest.slice(steered[0].length);
  }
  const workflow = rest.match(/^Run the "(.+?)" workflow\. Go through these steps[\s\S]*?\n\n/);
  if (workflow) {
    chips.push(h('span', { class: 'skill-chip' }, icon('workflow', 14), workflow[1]!));
    rest = rest.slice(workflow[0].length);
  }
  const library = rest.match(/^Search the "(.+?)" library \(search_library\) for this, and cite what you use\.\s*/);
  if (library) {
    chips.push(h('span', { class: 'skill-chip' }, icon('library', 14), library[1]!));
    rest = rest.slice(library[0].length);
  }
  const folder = rest.match(/^Work with the files in the folder `(.+?)`\.\s*/);
  if (folder) {
    chips.push(h('span', { class: 'skill-chip' }, icon('folder', 14), folder[1]!.split(/[\\/]/).pop() || folder[1]!));
    rest = rest.slice(folder[0].length);
  }
  const canvasAsk = rest.match(/^Use a canvas for this\.\s*/);
  if (canvasAsk) {
    chips.push(h('span', { class: 'skill-chip' }, icon('file', 14), 'Canvas'));
    rest = rest.slice(canvasAsk[0].length);
  }
  const skill = rest.match(/^Use the "([a-z0-9-]+)" skill\.\s*/);
  if (skill) {
    chips.push(h('span', { class: 'skill-chip' }, icon('skill', 14), skill[1]!));
    rest = rest.slice(skill[0].length);
  }
  const quote = rest.match(/^((?:> .*(?:\n|$))+)\n*/);
  let quoteEl: HTMLElement | null = null;
  if (quote) {
    quoteEl = h('div', { class: 'bubble-quote' }, icon('forward', 13), quote[1]!.replace(/^> /gm, '').trim());
    rest = rest.slice(quote[0].length);
  }
  const images = turn.parts.filter((p): p is Extract<Part, { kind: 'image' }> => p.kind === 'image');
  const gallery = images.length
    ? h('div', { class: 'bubble-images' }, images.map((i) => h('img', { src: `data:${i.mediaType};base64,${i.data}`, alt: 'Attached image' })))
    : null;
  const bubble = h('div', { class: 'bubble' }, gallery, quoteEl, chips, rest);
  return h(
    'div',
    { class: 'turn turn-user' },
    bubble,
    h('div', { class: `turn-actions user-actions${branch ? ' pinned' : ''}` }, branch, turn.index >= 0 ? editButton(turn, conversationId, text) : null, copy, h('span', { class: 'turn-time' }, timeLabel(turn.time)))
  );
}

/** Edit a sent message: the conversation is answered again from that point. */
function editButton(turn: Turn, conversationId: string, text: string): HTMLElement {
  const button = h('button', { class: 'icon-btn small', title: 'Edit' }, icon('edit', 15));
  button.addEventListener('click', () => {
    const el = button.closest('.turn-user');
    if (!el) return;
    const area = h('textarea', { class: 'input textarea edit-area', rows: 3 });
    area.value = text;
    const save = async () => {
      if (!state.model) return void toast('Choose a model first.', 'error');
      try {
        await api.send({ conversationId, text: area.value, model: state.model, speed: state.speed, approval: state.approval, editIndex: turn.index });
      } catch (e) {
        toast(String(e), 'error');
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
        h('div', { class: 'dialog-actions' }, h('button', { class: 'btn btn-secondary', on: { click: () => void viewCache.chat?.refresh(false) } }, 'Cancel'), h('button', { class: 'btn btn-primary', on: { click: () => void save() } }, 'Send'))
      )
    );
    area.focus();
  });
  return button;
}

function renderAssistantTurn(conversation: Conversation, turn: Turn, isLast: boolean, running: boolean, branch: HTMLElement | null = null): HTMLElement {
  const durations = isLast ? stepDurations.get(conversation.id) : undefined;
  const el = h('div', { class: 'turn turn-assistant' }, ...renderParts(turn.parts, false, `${conversation.id}:${turn.index}`, durations));
  // A reply that stopped after its tools ran, before answering, can pick up where it left off.
  const unfinished = isLast && !running && turn.parts[turn.parts.length - 1]?.kind === 'tool' && !state.live.has(conversation.id);
  if (unfinished) {
    const resume = h('button', { class: 'btn btn-secondary' }, icon('retry', 15), 'Continue');
    resume.addEventListener('click', () => {
      if (!state.model) return void toast('Choose a model first.', 'error');
      void api
        .send({ conversationId: conversation.id, text: '', model: state.model, speed: state.speed, approval: state.approval, resume: true })
        .catch((e) => toast(String(e), 'error'));
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

interface Source {
  url: string;
  title: string;
}

/** What an answer drew on: the pages it links to, then any it opened; search hits when it cites none. */
function sourcesOf(parts: Part[]): Source[] {
  const found = new Map<string, Source>();
  const add = (url: string, title: string) => {
    if (!/^https?:/.test(url)) return;
    const key = url.replace(/\/$/, '');
    if (!found.has(key)) found.set(key, { url, title: title || host(url) });
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
  if (!found.size) {
    for (const part of parts) {
      if (part.kind === 'tool' && part.name === 'web_search') {
        for (const row of resultRows(part)?.rows.slice(0, 5) ?? []) if (row.href) add(row.href, row.title);
      }
    }
  }
  return [...found.values()];
}

function renderSources(sources: Source[]): HTMLElement | null {
  if (!sources.length) return null;
  const chip = h(
    'button',
    { class: 'sources-chip' },
    'Sources',
    h('span', { class: 'source-dots' }, sources.slice(0, 3).map((s) => h('span', { class: 'source-dot' }, host(s.url).charAt(0).toUpperCase())))
  );
  chip.addEventListener('click', () =>
    popover(
      chip,
      h(
        'div',
        { class: 'menu sources-menu' },
        h('div', { class: 'menu-heading' }, `${sources.length} ${sources.length === 1 ? 'source' : 'sources'}`),
        sources.map((s) =>
          h('a', { class: 'menu-item source-item', href: s.url, target: '_blank', rel: 'noopener noreferrer' }, h('span', { class: 'source-dot' }, host(s.url).charAt(0).toUpperCase()), h('span', {}, h('strong', {}, s.title), h('small', { class: 'muted' }, host(s.url))))
        )
      )
    )
  );
  return h('div', { class: 'sources-row' }, chip);
}

function renderActions(conversation: Conversation, turn: Turn, isLast: boolean, branch: HTMLElement | null = null): HTMLElement {
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
    branch,
    h('button', { class: `icon-btn small${turn.rating > 0 ? ' active-good' : ''}`, title: 'Good reply', on: { click: rate(1) } }, icon('thumbUp', 16)),
    h('button', { class: `icon-btn small${turn.rating < 0 ? ' active-bad' : ''}`, title: 'Bad reply', on: { click: rate(-1) } }, icon('thumbDown', 16)),
    isLast ? h('button', { class: 'icon-btn small', title: 'Regenerate', on: { click: () => void regenerate(conversation.id) } }, icon('retry', 16)) : null,
    copy,
    readAloudButton(textOf(turn.parts)),
    h('span', { class: 'spacer' }),
    h('span', { class: 'turn-time' }, timeLabel(turn.time))
  );
  return actions;
}

/** ‹ 2 / 3 ›: earlier answers to the last prompt, kept when it was regenerated. */
function versionSwitcher(conversationId: string, versions: { from: number; current: number; total: number; kind: string }): HTMLElement {
  const go = (index: number) => async () => {
    try {
      await api.selectVersion(conversationId, versions.from, index);
      await viewCache.chat?.refresh(false);
    } catch (e) {
      toast(String(e), 'error');
    }
  };
  const prev = iconButton({ icon: 'back', title: 'Previous answer', size: 'xs', onClick: go(versions.current - 1) });
  const next = iconButton({ icon: 'forward', title: 'Next answer', size: 'xs', onClick: go(versions.current + 1) });
  prev.disabled = versions.current === 0;
  next.disabled = versions.current >= versions.total - 1;
  return h('span', { class: 'versions', title: versions.kind === 'prompt' ? 'Versions of this message and what followed' : 'Answers to this message' }, prev, h('span', { class: 'versions-label' }, `${versions.current + 1} / ${versions.total}`), next);
}

/** What is being read aloud, so starting another stops it. */
let speaking: { playback: Playback | null; button: HTMLButtonElement | null } | null = null;

function stopSpeaking() {
  if (!speaking) return;
  speaking.playback?.stop();
  speaking.button?.classList.remove('active');
  speaking.button?.replaceChildren(icon('speaker', 16));
  speaking = null;
}

/** Reads a finished answer aloud when auto-play is on. */
export function autoPlay(text: string) {
  if (!audioSettings?.tts.autoPlay || audioSettings.tts.engine === 'off' || !text.trim()) return;
  stopSpeaking();
  void speakText(text)
    .then((playback) => {
      speaking = { playback, button: null };
    })
    .catch((e) => toast(String(e), 'error'));
}

/** Reads an answer aloud with the voice model; a second click stops it. */
function readAloudButton(text: string): HTMLButtonElement {
  const button = iconButton({ icon: 'speaker', title: 'Read aloud', size: 'sm' });
  button.addEventListener('click', async () => {
    if (speaking?.button === button) return stopSpeaking();
    stopSpeaking();
    button.classList.add('active');
    button.replaceChildren(h('span', { class: 'dots' }));
    speaking = { playback: null, button };
    try {
      const playback = await speakText(text);
      if (speaking?.button !== button) return playback.stop();
      speaking.playback = playback;
      button.replaceChildren(icon('stop', 14));
      void playback.done.then(() => {
        if (speaking?.button === button) stopSpeaking();
      });
    } catch (e) {
      stopSpeaking();
      button.classList.remove('active');
      button.replaceChildren(icon('speaker', 16));
      toast(String(e), 'error');
    }
  });
  return button;
}

function renderLiveTurn(conversationId: string, live: LiveTurn): HTMLElement {
  const now = Date.now();
  const durations = live.starts.map((start, i) => (live.ends[i] ?? now) - start);
  const el = h('div', { class: 'turn turn-assistant live' }, ...renderParts(live.parts, !live.error, `${conversationId}:live:${live.startedAt}`, durations));
  for (const text of live.steers) {
    el.append(h('div', { class: 'steer-note' }, icon('forward', 14), h('span', {}, h('strong', {}, 'You, while it worked: '), text)));
  }
  const busy = !live.error && !live.question;
  if (live.compacting) {
    el.append(h('div', { class: 'status-line' }, h('span', { class: 'dots' }), 'Summarizing earlier messages to make room'));
  } else if (live.question) {
    el.append(h('div', { class: 'status-line' }, icon('wait', 16), 'Waiting for your answer'));
  } else if (live.pending) {
    const label = toolLabel(live.pending);
    el.append(h('div', { class: 'status-line' }, h('span', { class: 'dots' }), `${label.connector ? `${label.connector} · ` : ''}${label.action}…`));
  } else if (busy && (live.parts.length === 0 || live.parts[live.parts.length - 1]?.kind !== 'text')) {
    el.append(h('div', { class: 'status-line' }, h('span', { class: 'dots' }), 'Thinking'));
  }
  if (live.error) {
    const resume = h('button', { class: 'btn btn-secondary' }, icon('retry', 15), 'Continue');
    resume.addEventListener('click', () => {
      if (!state.model) return void toast('Choose a model first.', 'error');
      void api
        .send({ conversationId, text: '', model: state.model, speed: state.speed, approval: state.approval, resume: true })
        .catch((e) => toast(String(e), 'error'));
    });
    el.append(
      h(
        'div',
        { class: 'error-card' },
        icon('close', 16),
        h('div', { class: 'grow' }, h('strong', {}, 'The reply failed'), h('div', {}, live.error), h('p', { class: 'hint' }, 'Continue picks up from the work already done.')),
        resume
      )
    );
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

async function send(text: string, conversationId: string | null, images: { mediaType: string; data: string }[] = []) {
  const model = state.model;
  if (!model) {
    toast('Add a provider and choose a model first.', 'error');
    void openSettings('providers');
    return false;
  }
  if (!conversationId && compareModels.length > 1) return sendComparison(text, images);
  const id = conversationId ?? newId();
  const projectId = state.view.kind === 'home' ? state.view.projectId ?? null : null;
  const agentId = !conversationId && state.view.kind === 'home' ? state.view.agentId ?? null : null;
  const agent = agentId ? state.agents.find((a) => a.id === agentId) : undefined;
  const workdir = !conversationId && state.mode === 'code' ? state.workdir : null;
  const temporary = !conversationId && temporaryNext;
  if (!conversationId) {
    pendingUser.set(id, { text, images });
    navigate({ kind: 'chat', id });
  } else {
    viewCache.chat?.addOptimistic(text, images);
  }
  try {
    await api.send({
      conversationId: id,
      text,
      model: agent?.model ?? model,
      speed: agent?.think ? 'think' : state.speed,
      approval: state.approval,
      projectId,
      workdir,
      agentId,
      temporary,
      images,
      connectors: connectorChoice(),
    });
    if (temporary) temporaryChats.add(id);
    temporaryNext = false;
    void refreshConversations();
  } catch (e) {
    toast(String(e), 'error');
    return false;
  }
  return true;
}

/** Connectors switched off from the + menu; every ready one is on unless listed here. */
const connectorsOff = new Set<string>();
let knownConnectors: string[] = [];

/** The connectors a turn may use, or null for all of them. */
function connectorChoice(): string[] | null {
  return connectorsOff.size ? knownConnectors.filter((c) => !connectorsOff.has(c)) : null;
}

/** Models a new chat is sent to side by side; comparing when there are two or more. */
let compareModels: string[] = [];

/** Sends one prompt to every compared model, each in a temporary conversation of its own. */
async function sendComparison(text: string, images: { mediaType: string; data: string }[]) {
  const projectId = state.view.kind === 'home' ? state.view.projectId ?? null : null;
  const columns = compareModels.map((model) => ({ id: newId(), model }));
  navigate({ kind: 'compare', prompt: text, columns });
  const results = await Promise.allSettled(
    columns.map((c) =>
      api.send({ conversationId: c.id, text, model: c.model, speed: state.speed, approval: state.approval, projectId, temporary: true, images })
    )
  );
  const failed = results.find((r): r is PromiseRejectedResult => r.status === 'rejected');
  if (failed) toast(String(failed.reason), 'error');
  return results.some((r) => r.status === 'fulfilled');
}

function modelName(reference: string): string {
  return state.models.find((m) => m.reference === reference)?.name ?? reference.split('/').pop() ?? reference;
}

/** Picks the two or three models a new chat is compared across. */
function openCompareDialog(onDone: () => void) {
  void modal((close) => {
    const chosen = new Set(compareModels.length ? compareModels : state.model ? [state.model] : []);
    const list = h('div', { class: 'compare-pick' });
    const render = () =>
      clear(
        list,
        state.models.map((m) => {
          const box = h('input', { type: 'checkbox', checked: chosen.has(m.reference), disabled: !chosen.has(m.reference) && chosen.size >= 3 });
          box.addEventListener('change', () => {
            if (box.checked) chosen.add(m.reference);
            else chosen.delete(m.reference);
            render();
          });
          return h('label', { class: 'compare-option' }, box, h('span', { class: 'model-name' }, m.name), h('span', { class: 'muted' }, m.providerLabel));
        })
      );
    render();
    const start = () => {
      if (chosen.size < 2) return void toast('Choose at least two models.', 'error');
      compareModels = [...chosen];
      close();
      onDone();
    };
    return h(
      'div',
      { class: 'dialog-small' },
      modalHeader('Compare models', close, { flush: true }),
      h('p', { class: 'muted' }, 'Your next message goes to each model side by side. Keep the answer you like best and carry on with that model.'),
      list,
      h('div', { class: 'dialog-actions' }, h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'), h('button', { class: 'btn btn-primary', on: { click: start } }, 'Compare'))
    );
  });
}

/** The next new conversation is temporary: kept out of the history and deleted on restart. */
let temporaryNext = false;
const temporaryChats = new Set<string>();

/** Messages typed on the home screen, shown before the new conversation is on disk. */
const pendingUser = new Map<string, { text: string; images: { mediaType: string; data: string }[] }>();

export async function refreshConversations() {
  state.conversations = await api.conversations();
  changed();
}

// ---------------------------------------------------------------------------------------------
// Composer

interface Attachment {
  name: string;
  text: string;
  /** Set for images, which travel as image content rather than text. */
  image?: { mediaType: string; data: string };
}

function userParts(text: string, images: { mediaType: string; data: string }[]): Part[] {
  return [...images.map((i) => ({ kind: 'image' as const, mediaType: i.mediaType, data: i.data })), { kind: 'text' as const, text }];
}

const IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/webp', 'image/gif'];

/** Reads an image file as base64, without the `data:` prefix. */
function readImage(file: File): Promise<{ mediaType: string; data: string }> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve({ mediaType: file.type, data: String(reader.result).split(',')[1] ?? '' });
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}

class Composer {
  readonly el: HTMLElement;
  private textarea: HTMLTextAreaElement;
  private attachments: Attachment[] = [];
  /** Text the user selected and asked about. */
  private quote: string | null = null;
  /** Ask for the answer in a canvas. */
  private useCanvas = false;
  private chips: HTMLElement;
  private bar: HTMLElement;
  private conversationId: string | null = null;
  private placeholder = 'Type / for quick access';

  constructor() {
    this.textarea = h('textarea', { class: 'composer-input', rows: 1, placeholder: this.placeholder });
    this.textarea.addEventListener('input', () => {
      this.autosize();
      this.renderBar();
      this.updateSlashMenu();
    });
    this.textarea.addEventListener('keydown', (e) => {
      if (this.slashMenu && (e.key === 'ArrowDown' || e.key === 'ArrowUp' || e.key === 'Enter' || e.key === 'Tab' || e.key === 'Escape')) {
        e.preventDefault();
        this.slashKey(e.key);
        return;
      }
      if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
        e.preventDefault();
        void this.submit();
      }
    });
    this.textarea.addEventListener('paste', async (e) => {
      // WebKitGTK exposes pasted images through `items` only; other engines through `files` too.
      const fromItems = Array.from(e.clipboardData?.items ?? [])
        .filter((i) => i.kind === 'file' && IMAGE_TYPES.includes(i.type))
        .map((i) => i.getAsFile())
        .filter((f): f is File => f !== null);
      const files = fromItems.length ? fromItems : Array.from(e.clipboardData?.files ?? []).filter((f) => IMAGE_TYPES.includes(f.type));
      if (!files.length) return;
      e.preventDefault();
      for (const file of files) this.attachments.push({ name: file.name || 'pasted image', text: '', image: await readImage(file) });
      this.renderChips();
      this.renderBar();
    });
    this.chips = h('div', { class: 'chips' });
    this.bar = h('div', { class: 'composer-bar' });
    this.el = h('div', { class: 'composer' }, h('div', { class: 'composer-box' }, this.chips, this.textarea, this.bar));
    this.renderBar();
  }

  private slashMenu: { close: () => void; items: string[]; index: number; list: HTMLElement } | null = null;

  /** Typing `/` at the start lists the enabled skills; picking one inserts `/name `. */
  private updateSlashMenu() {
    const match = this.textarea.value.match(/^\/([a-z0-9-]*)$/);
    const skills = match ? state.skills.filter((s) => s.enabled && s.name.startsWith(match[1]!)) : [];
    if (!skills.length) {
      this.slashMenu?.close();
      this.slashMenu = null;
      return;
    }
    const list = h('div', { class: 'menu slash-menu' }, h('div', { class: 'menu-heading' }, 'Skills'));
    const items = skills.map((s) => s.name);
    const index = Math.min(this.slashMenu?.index ?? 0, items.length - 1);
    skills.forEach((skill, i) =>
      list.append(
        h(
          'button',
          { class: `menu-item rich${i === index ? ' selected' : ''}`, on: { mousedown: (e: MouseEvent) => { e.preventDefault(); this.pickSkill(skill.name); } } },
          icon('skill', 17),
          h('span', {}, h('strong', {}, `/${skill.name}`), h('small', {}, skill.description))
        )
      )
    );
    this.slashMenu?.close();
    const close = popover(this.textarea, list, 'above');
    this.slashMenu = { close, items, index, list };
  }

  private slashKey(key: string) {
    const menu = this.slashMenu!;
    if (key === 'Escape') {
      menu.close();
      this.slashMenu = null;
      return;
    }
    if (key === 'ArrowDown' || key === 'ArrowUp') {
      menu.index = (menu.index + (key === 'ArrowDown' ? 1 : -1) + menu.items.length) % menu.items.length;
      this.updateSlashMenu();
      return;
    }
    this.pickSkill(menu.items[menu.index]!);
  }

  private pickSkill(name: string) {
    this.slashMenu?.close();
    this.slashMenu = null;
    this.setText(`/${name} `);
    this.textarea.focus();
  }

  attach(conversationId: string | null, placeholder: string) {
    this.conversationId = conversationId;
    this.placeholder = placeholder;
    this.textarea.placeholder = placeholder;
    this.renderChips();
    this.renderBar();
    // Measured while detached the box collapses; size it once it is on screen again.
    requestAnimationFrame(() => this.autosize());
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
    if (this.running()) {
      // Steer: what the user types while a task runs joins it at its next step.
      const text = this.textarea.value.trim();
      if (!text || !this.conversationId) return;
      if (await api.steer(this.conversationId, text).catch(() => false)) {
        this.textarea.value = '';
        this.autosize();
        this.renderBar();
      } else {
        toast('The task finished before your message arrived; send it as a new message.');
      }
      return;
    }
    let text = this.textarea.value.trim();
    if (!text && this.attachments.length === 0 && !this.quote && !this.workflow) return;
    if (this.quote) {
      const quoted = this.quote.split('\n').map((l) => `> ${l}`).join('\n');
      text = `${quoted}\n\n${text || 'Tell me more about this.'}`;
    }
    if (this.useCanvas) text = `Use a canvas for this.\n\n${text}`;
    if (this.folder) text = `Work with the files in the folder \`${this.folder}\`.\n\n${text}`;
    if (this.library) text = `Search the "${this.library}" library (search_library) for this, and cite what you use.\n\n${text}`;
    if (this.workflow) text = workflowPrompt(this.workflow, text);
    // `/skill-name rest` asks for that skill explicitly.
    const slash = text.match(/^\/([a-z0-9-]+)\s*([\s\S]*)$/);
    if (slash && state.skills.some((s) => s.name === slash[1])) {
      text = `Use the "${slash[1]}" skill.${slash[2] ? `\n\n${slash[2]}` : ''}`;
    }
    for (const file of this.attachments.filter((a) => !a.image)) {
      text += `\n\n<attachment name="${file.name}">\n${file.text}\n</attachment>`;
    }
    const images = this.attachments.flatMap((a) => (a.image ? [a.image] : []));
    const ok = await send(text.trim(), this.conversationId, images);
    if (ok) {
      this.textarea.value = '';
      this.attachments = [];
      this.quote = null;
      this.useCanvas = false;
      this.folder = null;
      this.library = null;
      this.workflow = null;
      this.autosize();
      this.renderChips();
      this.renderBar();
    }
  }

  setQuote(text: string) {
    this.quote = text;
    this.renderChips();
    this.focus();
  }

  renderChips() {
    const extra: HTMLElement[] = [];
    if (this.quote) {
      extra.push(
        h(
          'div',
          { class: 'quote-chip' },
          icon('forward', 15),
          h('span', {}, this.quote),
          iconButton({ icon: 'close', title: 'Remove quote', size: 'xs', class: 'chip-x', onClick: () => { this.quote = null; this.renderChips(); } })
        )
      );
    }
    if (!this.conversationId && compareModels.length > 1) {
      extra.push(chip({ icon: 'apps', tone: 'accent', label: `Compare · ${compareModels.map(modelName).join(' vs ')}`, onRemove: () => { compareModels = []; this.renderChips(); } }));
    }
    const tokenChip = (ic: string, label: string, clearIt: () => void) =>
      chip({ icon: ic, label, tone: 'accent', onRemove: () => { clearIt(); this.renderChips(); this.renderBar(); } });
    if (this.workflow) extra.push(tokenChip('workflow', `Workflow · ${this.workflow.name}`, () => (this.workflow = null)));
    if (this.library) extra.push(tokenChip('library', `Library · ${this.library}`, () => (this.library = null)));
    if (this.folder) extra.push(tokenChip('folder', this.folder.split(/[\\/]/).pop() || this.folder, () => (this.folder = null)));
    if (this.useCanvas) {
      extra.push(chip({ icon: 'file', tone: 'accent', label: 'Canvas', onRemove: () => { this.useCanvas = false; this.renderChips(); } }));
    }
    clear(
      this.chips,
      extra,
      this.attachments.map((file, i) =>
        chip({
          leading: file.image ? h('img', { class: 'chip-thumb', src: `data:${file.image.mediaType};base64,${file.image.data}`, alt: '' }) : icon('file', 14),
          label: file.name,
          onRemove: () => { this.attachments.splice(i, 1); this.renderChips(); this.renderBar(); },
        })
      )
    );
  }

  private pickFiles() {
    const input = h('input', { type: 'file', multiple: true, class: 'hidden' });
    // WebKitGTK only reports the choice for an input that is in the document.
    document.body.append(input);
    input.addEventListener('change', async () => {
      input.remove();
      for (const file of Array.from(input.files ?? [])) {
        if (IMAGE_TYPES.includes(file.type)) {
          if (file.size > 8 * 1024 * 1024) {
            toast(`${file.name} is larger than 8 MB.`, 'error');
            continue;
          }
          this.attachments.push({ name: file.name, text: '', image: await readImage(file) });
          continue;
        }
        // Workbooks are read into CSV, one block per sheet.
        if (/\.(xlsx|xlsm|xls|ods)$/i.test(file.name)) {
          try {
            const bytes = new Uint8Array(await file.arrayBuffer());
            let binary = '';
            for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
            const sheets = await tauriInvoke<[string, string][]>('chat_read_spreadsheet', { data: btoa(binary) });
            const text = sheets.map(([name, csv]) => `Sheet "${name}" (CSV):\n${csv}`).join('\n\n');
            this.attachments.push({ name: file.name, text: text.length > 400_000 ? `${text.slice(0, 400_000)}\n… (cut; analyse the full file with run_code)` : text });
          } catch (e) {
            toast(String(e), 'error');
          }
          continue;
        }
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

    const APPROVALS = {
      ask: { icon: 'hand', name: 'Ask', desc: 'Ask before every change to your files or system' },
      accept_edits: { icon: 'pencil', name: 'Accept edits', desc: 'Approve file edits automatically; still ask for the rest' },
      plan: { icon: 'check', name: 'Plan', desc: 'Investigate and propose a plan; change nothing yet' },
      auto: { icon: 'bolt', name: 'Full auto', desc: 'Make changes without asking; you can stop it at any time' },
    } as const;
    const approvalIcon = APPROVALS[state.approval].icon;
    const approval = h('button', { class: `icon-btn tinted${state.approval === 'auto' ? ' warn' : ''}`, title: APPROVALS[state.approval].name }, icon(approvalIcon, 17));
    approval.addEventListener('click', () => {
      const option = (value: keyof typeof APPROVALS, name: string, desc: string, ic: string) =>
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
          (Object.keys(APPROVALS) as (keyof typeof APPROVALS)[]).map((k) => option(k, APPROVALS[k].name, APPROVALS[k].desc, APPROVALS[k].icon))
        ),
        'above'
      );
    });

    const model = currentModel();
    const speed = trigger({ label: state.speed === 'fast' ? 'Fast' : 'Think', title: 'Response mode' });
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

    const modelBtn = trigger({ label: model ? model.name : state.models.length ? 'Choose model' : 'Add a provider', title: 'Model', class: 'model-btn' });
    modelBtn.addEventListener('click', () => (state.models.length ? openModelPicker(modelBtn, () => this.renderBar()) : void openSettings('providers')));

    const hasText = this.textarea.value.trim().length > 0 || this.attachments.length > 0 || Boolean(this.workflow);
    const action = running && hasText
      ? h('button', { class: 'send-btn', title: 'Send guidance to the running task', on: { click: () => void this.submit() } }, icon('send', 18))
      : running
      ? h('button', { class: 'send-btn stop', title: 'Stop', on: { click: () => this.conversationId && void api.cancel(this.conversationId) } }, icon('stop', 16))
      : hasText || this.recorder || this.transcribing
        ? h('button', { class: 'send-btn', title: 'Send', disabled: !hasText, on: { click: () => void this.submit() } }, icon('send', 18))
        : h('button', { class: 'send-btn mic', title: 'Dictate', on: { click: () => void this.toggleDictation() } }, icon('mic', 18));

    const mic = h('button', { class: `icon-btn${this.recorder ? ' recording' : ''}`, title: this.recorder ? 'Stop and transcribe' : 'Dictate' }, icon(this.recorder ? 'stop' : 'mic', 17));
    mic.addEventListener('click', () => void this.toggleDictation());

    const dictation = this.transcribing ? h('span', { class: 'dots' }) : this.recorder ? mic : null;
    clear(this.bar, h('div', { class: 'bar-left' }, plus, approval), h('div', { class: 'bar-right' }, speed, modelBtn, dictation, action));
  }

  private recorder: Recording | null = null;
  private transcribing = false;
  private startingDictation = false;

  /** Records from the microphone; stopping sends the audio to the speech model. */
  private async toggleDictation() {
    if (this.recorder) return void this.finishDictation();
    if (this.startingDictation) return;
    this.startingDictation = true;
    try {
      const media = await tauriInvoke<{ speechModel: string | null }>('chat_media_models');
      if (!media.speechModel) {
        toast('Voice input needs a provider with speech-to-text, such as OpenAI or Groq.', 'error');
        return void openSettings('providers');
      }
      this.recorder = await startRecording();
      this.renderBar();
    } catch (e) {
      toast(`The microphone is not available: ${String(e)}`, 'error');
    } finally {
      this.startingDictation = false;
    }
  }

  private async finishDictation() {
    const recording = this.recorder;
    if (!recording) return;
    this.recorder = null;
    this.transcribing = true;
    this.renderBar();
    try {
      const audio = await recording.stop();
      if (audio.size < 1000) throw new Error('Nothing was recorded.');
      const text = await tauriInvoke<string>('chat_transcribe', { audio: await toBase64(audio), mime: audio.type });
      const current = this.textarea.value;
      this.setText(current ? `${current.trimEnd()} ${text}` : text);
      this.focus();
      // Open WebUI's "instant auto-send": dictation goes straight out.
      if (audioSettings?.stt.autoSend && text.trim()) void this.submit();
    } catch (e) {
      toast(String(e), 'error');
    } finally {
      this.transcribing = false;
      this.renderBar();
    }
  }

  folder: string | null = null;
  library: string | null = null;
  workflow: Workflow | null = null;

  /** Vibe's + menu: upload, then a flyout for each kind of context. */
  private openPlusMenu(anchor: HTMLElement) {
    const flyout = h('div', { class: 'flyout' });
    const menu = h('div', { class: 'menu plus-menu' });
    let close: () => void = () => {};
    const done = (f: () => void) => () => { close(); f(); };
    const item = (ic: string, label: string, onClick: () => void) =>
      h('button', { class: 'menu-item', on: { click: done(onClick), mouseenter: () => flyout.classList.remove('open') } }, icon(ic, 17), h('span', {}, label));
    const sub = (ic: string, label: string, fill: () => Promise<(HTMLElement | null)[]>) => {
      const row = h('button', { class: 'menu-item has-sub' }, icon(ic, 17), h('span', { class: 'grow' }, label), icon('chevronRight', 15));
      const show = async () => {
        menu.querySelectorAll('.has-sub.active').forEach((el) => el.classList.remove('active'));
        row.classList.add('active');
        flyout.style.top = `${row.offsetTop - 6}px`;
        clear(flyout, h('div', { class: 'status-line' }, h('span', { class: 'dots' })));
        flyout.classList.add('open');
        const content = await fill().catch((e) => [h('div', { class: 'empty-small' }, String(e))]);
        if (row.classList.contains('active')) clear(flyout, content);
      };
      row.addEventListener('mouseenter', () => void show());
      row.addEventListener('click', () => void show());
      return row;
    };
    const entry = (ic: string, label: string, hint: string | null, onClick: () => void, selected = false) =>
      h('button', { class: `menu-item${selected ? ' selected' : ''}`, on: { click: done(onClick) } }, icon(ic, 16), h('span', { class: 'grow ellipsis' }, label), hint ? h('small', { class: 'muted' }, hint) : null, selected ? icon('check', 14) : null);
    const heading = (text: string) => h('div', { class: 'menu-heading' }, text);
    const empty = (text: string) => h('div', { class: 'empty-small' }, text);
    const sep = () => h('div', { class: 'menu-sep' });

    const folders = async () => {
      const settings = await api.appSettings();
      const roots = settings.filesystem_allowed_roots;
      return [
        heading('Folders the assistant can read'),
        ...(roots.length ? roots.map((r: string) => entry('folder', r.split(/[\\/]/).pop() || r, null, () => { this.folder = r; this.renderChips(); this.renderBar(); }, this.folder === r)) : [empty('No folders yet.')]),
        sep(),
        entry('plus', 'Add a folder…', null, () => void grantFolder()),
      ];
    };
    const skills = async () => {
      const all = state.skills.length ? state.skills : await api.skills();
      const search = h('input', { class: 'input small', placeholder: 'Search' });
      const list = h('div', { class: 'flyout-list' });
      let tab: 'all' | 'user' | 'builtin' = 'all';
      const tabs = h('div', { class: 'mini-tabs' });
      const render = () => {
        clear(tabs, (['all', 'user', 'builtin'] as const).map((t) => h('button', { class: `mini-tab${tab === t ? ' active' : ''}`, on: { click: () => { tab = t; render(); } } }, t === 'all' ? 'All' : t === 'user' ? 'Mine' : 'Built-in')));
        const q = search.value.toLowerCase();
        const shown = all.filter((sk) => sk.enabled && (tab === 'all' || sk.origin === tab) && (!q || `${sk.name} ${sk.description}`.toLowerCase().includes(q)));
        clear(list, shown.length ? shown.map((sk) => entry('skill', sk.name, null, () => { this.setText(`/${sk.name} ${this.textarea.value.replace(/^\/[a-z0-9-]+\s*/, '')}`); this.focus(); })) : empty('No skill matches.'));
      };
      search.addEventListener('input', render);
      render();
      const create = button({ variant: 'link', size: 'sm', icon: 'plus', label: 'New skill', onClick: done(() => navigate({ kind: 'page', page: 'skills' })) });
      setTimeout(() => search.focus());
      return [h('div', { class: 'flyout-head' }, search, create), tabs, list];
    };
    const knowledge = async () => {
      const memories = await api.memories();
      const toggle = h('input', { type: 'checkbox', class: 'switch', checked: memories.enabled });
      toggle.addEventListener('change', () => void api.setMemoryEnabled(toggle.checked).then(() => toast(toggle.checked ? 'Knowledge is on.' : 'Knowledge is off for every chat.')));
      return [
        h('label', { class: 'menu-item static' }, icon('book', 16), h('span', { class: 'grow' }, 'Use knowledge'), toggle),
        h('div', { class: 'muted small flyout-note' }, `${memories.items.length} things remembered`),
        sep(),
        entry('settings', 'Open Knowledge', null, () => navigate({ kind: 'page', page: 'knowledge' })),
      ];
    };
    const connectors = async () => {
      const manifests = (await api.connectors()).filter((m) => m.enabled && m.readiness.state === 'ready');
      knownConnectors = manifests.map((m) => m.id);
      const rows = manifests.map((m) => {
        const on = h('input', { type: 'checkbox', class: 'switch', checked: !connectorsOff.has(m.id) });
        on.addEventListener('change', () => (on.checked ? connectorsOff.delete(m.id) : connectorsOff.add(m.id)));
        return h('label', { class: 'menu-item static' }, icon(CONNECTOR_ICONS[m.id] ?? 'plug', 16), h('span', { class: 'grow ellipsis' }, m.display_name), on);
      });
      return [heading('Use in this chat'), ...(rows.length ? rows : [empty('No connectors are ready.')]), sep(), entry('plug', 'Manage connectors', null, () => navigate({ kind: 'page', page: 'connectors' }))];
    };
    const projects = async () => {
      const current = this.conversationId ? state.conversations.find((c) => c.id === this.conversationId)?.projectId : state.view.kind === 'home' ? state.view.projectId : null;
      const pick = (id: string | null) => () => {
        if (this.conversationId) void api.updateConversation({ id: this.conversationId, projectId: id }).then(() => refreshConversations());
        else navigate({ kind: 'home', projectId: id });
      };
      return [
        heading(this.conversationId ? 'Move this chat to' : 'Start this chat in'),
        ...(state.projects.length ? state.projects.map((pr) => entry('folder', `${pr.icon ? `${pr.icon} ` : ''}${pr.name}`, null, pick(pr.id), current === pr.id)) : [empty('No projects yet.')]),
        current ? entry('close', 'No project', null, pick(null)) : null,
      ];
    };
    const libraries = async () => {
      const libs = await tauriInvoke<{ id: string; name: string; documents: unknown[] }[]>('chat_list_libraries');
      return [
        heading('Search a library'),
        ...(libs.length ? libs.map((l) => entry('library', l.name, `${l.documents.length} docs`, () => { this.library = l.name; this.renderChips(); this.renderBar(); }, this.library === l.name)) : [empty('No libraries yet.')]),
        sep(),
        entry('settings', 'Manage libraries', null, () => navigate({ kind: 'page', page: 'libraries' })),
      ];
    };
    const workflows = async () => {
      const all = await workflowsApi.list();
      return [
        heading('Run a workflow'),
        ...all.map((w) => entry('workflow', w.name, `${w.steps.length} steps`, () => { this.workflow = w; this.renderChips(); this.renderBar(); this.focus(); }, this.workflow?.id === w.id)),
        sep(),
        entry('plus', 'New workflow', null, () => editWorkflow(null, () => toast('Workflow saved.'))),
        entry('settings', 'Manage workflows', null, () => navigate({ kind: 'page', page: 'workflows' })),
      ];
    };

    menu.append(
      item('upload', 'Upload files', () => this.pickFiles()),
      sub('folder', 'Folders', folders),
      sub('skill', 'Skills', skills),
      sub('book', 'Knowledge', knowledge),
      sub('plug', 'Connectors', connectors),
      sub('folder', 'Projects', projects),
      sub('library', 'Libraries', libraries),
      sub('workflow', 'Workflows', workflows),
      item('file', 'Canvas', () => { this.useCanvas = true; this.renderChips(); this.focus(); }),
      sep(),
      item('reset', 'Reset input', () => { this.setText(''); this.attachments = []; this.folder = null; this.library = null; this.workflow = null; this.quote = null; this.useCanvas = false; this.renderChips(); this.renderBar(); })
    );
    close = popover(anchor, h('div', { class: 'plus-wrap' }, menu, flyout), 'below');
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
    h(
      'div',
      { class: 'menu model-menu' },
      search,
      list,
      h('div', { class: 'menu-sep' }),
      state.view.kind === 'home' && state.models.length > 1
        ? h('button', { class: 'menu-item', on: { click: () => { close(); openCompareDialog(() => { onPick(); composer.renderChips(); }); } } }, icon('apps', 16), 'Compare models…')
        : null,
      h('button', { class: 'menu-item', on: { click: () => { close(); void openSettings('providers'); } } }, icon('settings', 16), 'Manage providers')
    ),
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
    request.tool_name === 'start' && request.connector_id === 'terminal'
      ? h('pre', { class: 'step-pre' }, h('code', {}, `$ ${String((request.input as { command?: unknown }).command ?? '')}`))
      : request.tool_name === 'run_code' && typeof (request.input as { code?: unknown }).code === 'string'
      ? h('pre', { class: 'step-pre' }, h('code', {}, (request.input as { code: string }).code))
      : h('pre', { class: 'step-pre' }, JSON.stringify(request.input, null, 2)),
    h(
      'div',
      { class: 'dialog-actions' },
      h('button', { class: 'btn btn-secondary', on: { click: () => respond(false) } }, 'Deny'),
      h('button', { class: 'btn btn-secondary', on: { click: () => respond(true, true) } }, 'Always allow'),
      h('button', { class: 'btn btn-primary', on: { click: () => respond(true) } }, 'Allow once')
    )
  );
}

// ---------------------------------------------------------------------------------------------
// Views

const SUGGESTIONS: { icon: string; text: string; page?: boolean; more?: boolean }[] = [
  { icon: 'skill', text: 'Set up and discover what this assistant can do' },
  { icon: 'globe', text: 'What is the current state of the AI industry?' },
  { icon: 'plug', text: 'Connect your apps for better answers', page: true },
  { icon: 'sparkle', text: 'Explain a concept like I am new to it', more: true },
  { icon: 'file', text: 'Look through my documents folder and tell me what is there', more: true },
  { icon: 'terminal', text: 'Analyze a CSV file and chart what matters', more: true },
  { icon: 'apps', text: 'Build me a small app: a pomodoro timer', more: true },
  { icon: 'clock', text: 'Every weekday at 9:00, brief me on AI news', more: true },
];

/** Connectors the banner above the composer suggests, shown in turn. */
const BANNER_APPS = ['M', 'S', 'G', 'N'];
let bannerDismissed = (() => {
  try {
    return localStorage.getItem('connectBannerDismissed') === '1';
  } catch {
    return false;
  }
})();
let showMoreSuggestions = false;

export class HomeView {
  readonly el = h('div', { class: 'home' });
  readonly key: string;
  constructor(private projectId: string | null, private agentId: string | null = null) {
    this.key = `home:${projectId ?? ''}:${agentId ?? ''}:${state.mode}`;
    this.build();
  }

  build() {
    this.noModels = state.models.length === 0;
    const project = state.projects.find((p) => p.id === this.projectId);
    const agent = state.agents.find((a) => a.id === this.agentId);
    const code = state.mode === 'code' && !agent;
    const title = code
      ? h(
          'h1',
          { class: 'greeting' },
          'Ready for a session in ',
          h('button', { class: 'folder-pick', on: { click: () => void pickWorkdir() } }, icon('folder', 26), state.workdir ? state.workdir.split(/[\\/]/).pop() ?? state.workdir : 'Choose a folder'),
          '?'
        )
      : agent
        ? h('h1', { class: 'greeting' }, `What should ${agent.name} do?`)
        : h('h1', { class: 'greeting' }, project ? `${project.icon ? `${project.icon} ` : ''}New chat in ${project.name}` : greeting(state.instructions.name));
    composer.attach(null, code ? 'What do you want to build?' : 'Type / for quick access');
    const dismiss = h('button', { class: 'icon-btn small', title: 'Dismiss' }, icon('close', 16));
    dismiss.addEventListener('click', () => {
      bannerDismissed = true;
      try {
        localStorage.setItem('connectBannerDismissed', '1');
      } catch {
        /* per-viewer convenience only */
      }
      this.build();
    });
    const banner =
      state.models.length === 0
        ? h('div', { class: 'composer-banner' }, icon('key', 16), h('span', {}, 'Add an API key or a local server to start chatting'), button({ variant: 'link', label: 'Add provider', onClick: () => void openSettings('providers') }))
        : !code && !agent && !bannerDismissed
          ? h(
              'div',
              { class: 'composer-banner connect' },
              h('span', { class: 'banner-app' }, BANNER_APPS[new Date().getMinutes() % BANNER_APPS.length]),
              h('span', { class: 'grow' }, 'Work mode is better with your apps connected'),
              button({ variant: 'link', label: 'Connect', class: 'strong', onClick: () => navigate({ kind: 'page', page: 'connectors' }) }),
              dismiss
            )
          : null;
    const suggestions = code || agent
      ? null
      : h(
          'div',
          { class: 'suggestions' },
          SUGGESTIONS.filter((s) => showMoreSuggestions || !s.more).map((s) =>
            h(
              'button',
              {
                class: 'suggestion',
                on: { click: () => (s.page ? navigate({ kind: 'page', page: 'connectors' }) : (composer.setText(s.text), composer.focus())) },
              },
              icon(s.icon, 17),
              s.text.replace(/: $/, '')
            )
          ),
          showMoreSuggestions
            ? null
            : h('button', { class: 'suggestion', on: { click: () => { showMoreSuggestions = true; this.build(); } } }, icon('more', 17), 'Discover more')
        );
    const ghost = h('button', { class: `temp-toggle icon-only${temporaryNext ? ' on' : ''}`, title: temporaryNext ? 'Temporary chat: on (not saved to history)' : 'Temporary chat' }, icon('chatDashed', 20), temporaryNext ? h('span', {}, 'Temporary') : null);
    ghost.addEventListener('click', () => {
      temporaryNext = !temporaryNext;
      this.build();
    });
    clear(
      this.el,
      code ? null : ghost,
      h('div', { class: 'home-inner' }, agent ? h('span', { class: 'agent-avatar large' }, agent.avatar || agent.name.charAt(0)) : h('div', { class: 'brand-mark' }), title, h('div', { class: 'composer-wrap' }, banner, composer.el), suggestions)
    );
    setTimeout(() => composer.focus());
  }

  update() {
    // The models load after the first render; redraw once whether a provider exists is known.
    if ((state.models.length === 0) !== this.noModels) this.build();
    composer.renderBar();
  }

  private noModels = state.models.length === 0;
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

/** One prompt answered by several models side by side; the user keeps the answer they prefer. */
export class CompareView {
  readonly el = h('div', { class: 'chat compare' });
  readonly key: string;
  private grid = h('div', { class: 'compare-grid' });
  private top = h('div', {});
  private saved = new Map<string, Turn | null>();

  constructor(
    private prompt: string,
    private columns: { id: string; model: string }[]
  ) {
    this.key = `compare:${columns.map((c) => c.id).join(',')}`;
    const discard = h('button', { class: 'btn btn-secondary' }, icon('trash', 15), 'Discard all');
    discard.addEventListener('click', async () => {
      await Promise.allSettled(this.columns.map((c) => api.deleteConversation(c.id)));
      navigate({ kind: 'home' });
    });
    this.el.append(
      h('header', { class: 'chat-header' }, h('span', { class: 'chat-title-static' }, `Comparing ${columns.length} models`), h('span', { class: 'spacer' }), discard),
      h('div', { class: 'chat-scroll' }, h('div', { class: 'compare-column-wrap' }, h('div', { class: 'turn turn-user' }, h('div', { class: 'bubble' }, prompt)), this.top, this.grid))
    );
    this.update();
  }

  private async load(id: string) {
    try {
      const conversation = await api.conversation(id);
      const last = [...conversation.turns].reverse().find((t) => t.role === 'assistant') ?? null;
      this.saved.set(id, last);
    } catch {
      this.saved.set(id, null);
    }
    this.render();
  }

  private async keep(id: string) {
    await api.keepConversation(id);
    await Promise.allSettled(this.columns.filter((c) => c.id !== id).map((c) => api.deleteConversation(c.id)));
    compareModels = [];
    await refreshConversations();
    navigate({ kind: 'chat', id });
  }

  private render() {
    clear(this.top, renderApprovals());
    clear(
      this.grid,
      this.columns.map((c) => {
        const live = state.live.get(c.id);
        const turn = this.saved.get(c.id);
        let body: HTMLElement;
        if (live) {
          body = renderLiveTurn(c.id, live);
          if (live.question) body.append(renderQuestion(c.id, live.question));
        } else if (turn) {
          body = h('div', { class: 'turn turn-assistant' }, ...renderParts(turn.parts, false, `${c.id}:${turn.index}`));
        } else {
          if (!this.saved.has(c.id)) void this.load(c.id);
          body = h('div', { class: 'status-line' }, h('span', { class: 'dots' }), 'Waiting');
        }
        const done = !live && !!turn;
        const keep = h('button', { class: 'btn btn-primary', disabled: !done }, icon('check', 15), 'Keep this answer');
        keep.addEventListener('click', () => void this.keep(c.id));
        return h(
          'section',
          { class: 'compare-col' },
          h('div', { class: 'compare-head' }, h('strong', {}, modelName(c.model)), h('span', { class: 'spacer' }), keep),
          body
        );
      })
    );
  }

  update() {
    // A finished column reloads what was saved, so its final answer replaces the stream.
    for (const c of this.columns) {
      if (state.live.has(c.id)) this.saved.delete(c.id);
    }
    this.render();
  }
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
  private planPanel = h('aside', { class: 'plan-panel' });
  private sidePanel = h('aside', { class: 'side-panel' });
  /** Files changed in the session's folder: undefined until asked, null while asking, -1 without git. */
  private changeCount: number | null | undefined = undefined;
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
    enableAskAboutSelection(this.scroller, (quote) => composer.setQuote(quote));
    composer.attach(id, 'Type / for quick access');
    this.bottom.append(this.jump, this.overlay, composer.el, h('div', { class: 'disclaimer' }, 'AI can make mistakes. Check answers.'));
    this.el.append(this.header, this.scroller, this.bottom, this.planPanel, this.sidePanel);
    // Apps open beside the conversation; Escape closes the panel.
    setPanelHost((content) => {
      clear(this.sidePanel, content);
      this.el.classList.toggle('with-panel', content !== null);
    });
    this.el.addEventListener('keydown', (e) => {
      if (e.key === 'Escape' && this.el.classList.contains('with-panel') && !document.querySelector('.modal-backdrop')) closePanel();
    });
    void this.refresh(true);
  }

  async refresh(scroll: boolean) {
    try {
      this.conversation = await api.conversation(this.id);
      pendingUser.delete(this.id);
      // Opening a conversation that finished in the background marks it read.
      const summary = state.conversations.find((c) => c.id === this.id);
      if (summary?.unread && !state.live.has(this.id)) {
        summary.unread = false;
        void api.updateConversation({ id: this.id, unread: false }).then(() => changed());
      }
    } catch {
      this.conversation = {
        id: this.id,
        title: '',
        projectId: null,
        workdir: null,
        model: null,
        pinned: false,
        turns: [],
        running: true,
        usage: { inputTokens: 0, outputTokens: 0, cost: 0, contextTokens: 0 },
        compactedBefore: null,
        agentId: null,
        archived: false,
        versions: [],
      };
    }
    const optimistic = pendingUser.get(this.id);
    if (optimistic && this.conversation.turns.length === 0) {
      this.conversation.turns.push({ role: 'user', parts: userParts(optimistic.text, optimistic.images), index: -1, time: new Date().toISOString(), rating: 0 });
    }
    this.renderTranscript();
    this.update();
    if (scroll) this.scrollToBottom(false);
  }

  addOptimistic(text: string, images: { mediaType: string; data: string }[] = []) {
    this.conversation?.turns.push({ role: 'user', parts: userParts(text, images), index: -1, time: new Date().toISOString(), rating: 0 });
    this.renderTranscript();
    this.scrollToBottom(true);
  }

  renderTranscript() {
    this.changeCount = undefined;
    const c = this.conversation;
    if (!c) return;
    const running = state.live.has(this.id);
    const last = c.turns.length - 1;
    const divider = c.compactedBefore;
    let marked = false;
    clear(
      this.transcript,
      c.turns.flatMap((turn, i) => {
        // The branch switcher sits on the turn that varies: an edited message, or a regenerated answer.
        const previous = c.turns[i - 1];
        const v = c.versions.find(
          (b) =>
            (turn.role === 'user' && b.kind === 'prompt' && turn.index === b.from) ||
            (turn.role === 'assistant' && b.kind === 'answer' && previous?.role === 'user' && (previous.index === b.from || previous.index + 1 === b.from))
        );
        const branch = v ? versionSwitcher(c.id, v) : null;
        const el = turn.role === 'user' ? renderUserTurn(turn, c.id, branch) : renderAssistantTurn(c, turn, i === last, running, branch);
        // Turns before the summary point reach the model only through the summary.
        if (divider !== null && divider !== undefined && !marked && turn.role === 'user' && turn.index >= divider) {
          marked = true;
          return [h('div', { class: 'compaction-divider' }, h('span', {}, icon('book', 14), 'Earlier messages are summarized for the model')), el];
        }
        return [el];
      })
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
    const meter = usageMeter(this.id, state.usage.get(this.id), c?.usage);
    const exportBtn = h('button', { class: 'icon-btn', title: 'Export as Markdown' }, icon('share', 18));
    let changesBtn: HTMLElement | null = null;
    let terminalBtn: HTMLElement | null = null;
    if (workdir) {
      if (this.changeCount === undefined) {
        this.changeCount = null;
        void loadChanges(this.id)
          .then((c) => (this.changeCount = c.git ? c.files.length : -1))
          .catch(() => (this.changeCount = -1))
          .then(() => this.renderHeader());
      }
      if (this.changeCount !== -1) {
        terminalBtn = trigger({ icon: 'terminal', label: 'Terminal', title: 'Background commands in this folder', chevron: false, onClick: () => openTerminal(this.id, workdir.split(/[\\/]/).pop() ?? workdir) });
        changesBtn = trigger({ icon: 'file', label: 'Changes', title: 'Uncommitted changes in this folder', chevron: false, class: 'changes-btn', trailing: this.changeCount ? h('span', { class: 'count-badge' }, String(this.changeCount)) : null });
        changesBtn.addEventListener('click', () => openChanges(this.id, workdir.split(/[\\/]/).pop() ?? workdir));
      }
    }
    exportBtn.addEventListener('click', () => void api.exportConversation(this.id).then((saved) => saved && toast('Conversation exported.')));
    clear(
      this.header,
      h(
        'div',
        { class: 'crumbs' },
        temporaryChats.has(this.id) ? h('span', { class: 'temp-badge' }, icon('wait', 14), 'Temporary') : null,
        project ? h('button', { class: 'crumb', on: { click: () => navigate({ kind: 'home', projectId: project.id }) } }, icon('folder', 15), project.name) : null,
        workdir ? h('span', { class: 'crumb' }, icon('terminal', 15), workdir.split(/[\\/]/).pop() ?? workdir) : null,
        project || workdir ? h('span', { class: 'crumb-sep' }, '/') : null,
        titleBtn
      ),
      h('div', { class: 'header-actions' }, terminalBtn, changesBtn, meter, pin, exportBtn)
    );
  }

  update() {
    this.renderHeader();
    const live = state.live.get(this.id);
    clear(this.liveSlot, live ? renderLiveTurn(this.id, live) : null);
    // While the model works through a plan, the plan stays in view beside the conversation.
    const plan = live ? [...live.parts].reverse().find((p): p is Extract<Part, { kind: 'tool' }> => p.kind === 'tool' && p.name === 'update_plan') : undefined;
    const showPlan = Boolean(plan && !live?.error);
    clear(this.planPanel, showPlan && plan ? [h('div', { class: 'plan-head' }, icon('check', 15), 'Plan'), renderChecklist(planOf(plan))] : null);
    this.planPanel.classList.toggle('visible', showPlan);
    this.el.classList.toggle('with-plan', showPlan);
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

  /** Reads the newest answer aloud, when auto-play is on. */
  readLatestAloud() {
    const last = this.conversation?.turns[this.conversation.turns.length - 1];
    if (last?.role === 'assistant') autoPlay(textOf(last.parts));
  }

  private scrollToBottom(smooth: boolean) {
    requestAnimationFrame(() => this.scroller.scrollTo({ top: this.scroller.scrollHeight, behavior: smooth ? 'smooth' : 'auto' }));
  }
}

/** Tokens, cost and how full the model's window is, for the conversation header. */
function usageMeter(id: string, live: { usage: UsageTotals; window: number } | undefined, stored: UsageTotals | undefined): HTMLElement | null {
  const usage = live?.usage ?? stored;
  if (!usage || !(usage.inputTokens || usage.outputTokens)) return null;
  const pct = live && live.window ? Math.min(100, Math.round((usage.contextTokens / live.window) * 100)) : null;
  const money = (n: number) => '$' + n.toFixed(n < 0.01 ? 4 : 2);
  const lines = [
    pct !== null && live ? `Context: ${usage.contextTokens.toLocaleString()} of ${live.window.toLocaleString()} tokens (${pct}%)` : '',
    `Used: ${usage.inputTokens.toLocaleString()} in, ${usage.outputTokens.toLocaleString()} out`,
    usage.cost > 0 ? `Cost: ${money(usage.cost)}` : '',
  ].filter(Boolean);
  const total = Math.round((usage.inputTokens + usage.outputTokens) / 1000);
  return h(
    'button',
    { class: 'usage-meter', title: `${lines.join('. ')}. Click to see what fills the context.`, on: { click: () => void showContext(id) } },
    pct !== null ? h('span', { class: 'ring', style: { '--pct': String(pct) } }) : null,
    h('span', {}, `${total}k tokens${usage.cost > 0 ? ' · ' + money(usage.cost) : ''}`)
  );
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
      item('branch', 'Branch into a new chat', async () => {
        const branch = await api.forkConversation(id);
        await refreshConversations();
        navigate({ kind: 'chat', id: branch });
        toast('Branched. This chat continues from the same point.');
      }),
      item('download', 'Export as Markdown', () => void api.exportConversation(id).then((saved) => saved && toast('Conversation exported.'))),
      item('tag', 'Tags…', async () => {
        const current = state.conversations.find((c) => c.id === id)?.tags ?? [];
        const next = await promptText('Tags (comma separated)', current.join(', '));
        if (next === null || next === undefined) return;
        await api.updateConversation({ id, tags: next.split(',') });
        await refreshConversations();
      }),
      item('archive', state.conversations.find((c) => c.id === id)?.archived ? 'Unarchive' : 'Archive', async () => {
        const archived = !state.conversations.find((c) => c.id === id)?.archived;
        await api.updateConversation({ id, archived });
        if (archived && state.view.kind === 'chat' && state.view.id === id) navigate({ kind: 'home' });
        await refreshConversations();
        toast(archived ? 'Archived. Find it under Archived at the end of the list.' : 'Back in your conversations.');
      }),
      h('div', { class: 'menu-sep' }),
      item('trash', 'Delete', async () => {
        if (await confirmDialog('Delete conversation?', `“${title}” moves to Recently deleted, where it stays for 30 days (Settings → Profile).`)) {
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

const SLICE_COLORS = ['var(--accent)', '#4fa3d1', '#5fb98a', '#d9a441', '#c96f9b', '#8c8fd6', '#9aa0a6', '#d97a5b'];

/** The context view: what the next request carries, and how full the model's window is. */
async function showContext(id: string) {
  const view = await api.contextBreakdown(id).catch((e) => (toast(String(e), 'error'), null));
  if (!view) return;
  const used = view.measured || view.slices.reduce((n, s) => n + s.tokens, 0);
  const pct = Math.min(100, Math.round((used / view.window) * 100));
  const k = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(n >= 10_000 ? 0 : 1)}k` : String(n));
  void modal((close) =>
    h(
      'div',
      { class: 'dialog-small wide context-view' },
      modalHeader('Context', close),
      h('p', { class: 'muted' }, `${used.toLocaleString()} of ${view.window.toLocaleString()} tokens in use (${pct}%). Older messages are summarized automatically at 60%.`),
      h(
        'div',
        { class: 'context-bar' },
        view.slices.map((s, i) => h('span', { title: s.label, style: { width: `${(s.tokens / view.window) * 100}%`, background: SLICE_COLORS[i % SLICE_COLORS.length]! } }))
      ),
      h(
        'div',
        { class: 'context-legend' },
        view.slices.map((s, i) =>
          h('div', { class: 'context-row' }, h('span', { class: 'context-dot', style: { background: SLICE_COLORS[i % SLICE_COLORS.length]! } }), h('span', { class: 'grow' }, s.label), h('span', { class: 'muted' }, `${k(s.tokens)} · ${((s.tokens / view.window) * 100).toFixed(1)}%`))
        )
      ),
      view.summarizedMessages ? h('p', { class: 'muted small' }, `${view.summarizedMessages} earlier messages are carried as a summary.`) : null,
      view.clearedResults ? h('p', { class: 'muted small' }, `${view.clearedResults} older tool results are shortened; the assistant runs a tool again if it needs one.`) : null,
      h('p', { class: 'muted small' }, 'The total is what the provider measured; the split is an estimate.')
    )
  );
}
