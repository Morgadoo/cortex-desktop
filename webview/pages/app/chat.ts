/** The home screen, a conversation, and the composer they share. */
import { api, newId, type Conversation, type Part, type Turn, type UsageTotals } from './api';
import { clear, confirmDialog, greeting, h, icon, markdown, modal, popover, promptText, timeLabel, toast } from './dom';
import { changed, currentModel, navigate, state, type LiveTurn } from './state';
import { openSettings } from './settings';
import { openInstructions } from './pages';
import { enableAskAboutSelection, renderCanvasCard } from './canvas';

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
  web_search: 'Searched the web',
  remember: 'Saved to knowledge',
  forget: 'Removed from knowledge',
  use_skill: 'Used a skill',
  update_plan: 'Updated the plan',
  canvas: 'Wrote a canvas',
  delegate: 'Ran sub-agents',
};

const BUILTIN_ICONS: Record<string, string> = {
  web_search: 'search',
  remember: 'book',
  forget: 'book',
  use_skill: 'skill',
  update_plan: 'check',
  canvas: 'file',
  delegate: 'workflow',
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
  // A skill invoked with /name reads as a chip ahead of the message.
  let rest = text;
  const chips: HTMLElement[] = [];
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
  const bubble = h('div', { class: 'bubble' }, quoteEl, chips, rest);
  return h(
    'div',
    { class: 'turn turn-user' },
    bubble,
    h('div', { class: 'turn-actions user-actions' }, copy, h('span', { class: 'turn-time' }, timeLabel(turn.time)))
  );
}

function renderAssistantTurn(conversation: Conversation, turn: Turn, isLast: boolean, running: boolean): HTMLElement {
  const durations = isLast ? stepDurations.get(conversation.id) : undefined;
  const el = h('div', { class: 'turn turn-assistant' }, ...renderParts(turn.parts, false, `${conversation.id}:${turn.index}`, durations));
  // A reply that stopped after its tools ran, before answering, can pick up where it left off.
  const unfinished = isLast && !running && turn.parts[turn.parts.length - 1]?.kind === 'tool' && !state.live.has(conversation.id);
  if (unfinished) {
    const resume = h('button', { class: 'btn btn-ghost' }, icon('retry', 15), 'Continue');
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
    el.append(renderActions(conversation, turn, isLast));
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
  const el = h('div', { class: 'turn turn-assistant live' }, ...renderParts(live.parts, !live.error, `${conversationId}:live:${live.startedAt}`, durations));
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
    const resume = h('button', { class: 'btn btn-ghost' }, icon('retry', 15), 'Continue');
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
  /** Text the user selected and asked about. */
  private quote: string | null = null;
  /** Ask for the answer in a canvas. */
  private useCanvas = false;
  private chips: HTMLElement;
  private bar: HTMLElement;
  private conversationId: string | null = null;
  private placeholder = 'Ask anything. Type / for quick actions';

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
    if (!text && this.attachments.length === 0 && !this.quote) return;
    if (this.quote) {
      const quoted = this.quote.split('\n').map((l) => `> ${l}`).join('\n');
      text = `${quoted}\n\n${text || 'Tell me more about this.'}`;
    }
    if (this.useCanvas) text = `Use a canvas for this.\n\n${text}`;
    // `/skill-name rest` asks for that skill explicitly.
    const slash = text.match(/^\/([a-z0-9-]+)\s*([\s\S]*)$/);
    if (slash && state.skills.some((s) => s.name === slash[1])) {
      text = `Use the "${slash[1]}" skill.${slash[2] ? `\n\n${slash[2]}` : ''}`;
    }
    for (const file of this.attachments) {
      text += `\n\n<attachment name="${file.name}">\n${file.text}\n</attachment>`;
    }
    const ok = await send(text.trim(), this.conversationId);
    if (ok) {
      this.textarea.value = '';
      this.attachments = [];
      this.quote = null;
      this.useCanvas = false;
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

  private renderChips() {
    const extra: HTMLElement[] = [];
    if (this.quote) {
      extra.push(
        h(
          'div',
          { class: 'quote-chip' },
          icon('forward', 15),
          h('span', {}, this.quote),
          h('button', { class: 'chip-x', title: 'Remove', on: { click: () => { this.quote = null; this.renderChips(); } } }, icon('close', 13))
        )
      );
    }
    if (this.useCanvas) {
      extra.push(h('span', { class: 'chip tool-chip' }, icon('file', 14), 'Canvas', h('button', { class: 'chip-x', title: 'Remove', on: { click: () => { this.useCanvas = false; this.renderChips(); } } }, icon('close', 12))));
    }
    clear(
      this.chips,
      extra,
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
        item('file', 'Canvas', () => { this.useCanvas = true; this.renderChips(); this.focus(); }),
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
  private planPanel = h('aside', { class: 'plan-panel' });
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
    composer.attach(id, 'Reply…');
    this.bottom.append(this.jump, this.overlay, composer.el, h('div', { class: 'disclaimer' }, 'AI can make mistakes. Check important information.'));
    this.el.append(this.header, this.scroller, this.bottom, this.planPanel);
    void this.refresh(true);
  }

  async refresh(scroll: boolean) {
    try {
      this.conversation = await api.conversation(this.id);
      pendingUser.delete(this.id);
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
      };
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
    const divider = c.compactedBefore;
    let marked = false;
    clear(
      this.transcript,
      c.turns.flatMap((turn, i) => {
        const el = turn.role === 'user' ? renderUserTurn(turn) : renderAssistantTurn(c, turn, i === last, running);
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
    const meter = usageMeter(state.usage.get(this.id), c?.usage);
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
      h('div', { class: 'header-actions' }, meter, pin, exportBtn)
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

  private scrollToBottom(smooth: boolean) {
    requestAnimationFrame(() => this.scroller.scrollTo({ top: this.scroller.scrollHeight, behavior: smooth ? 'smooth' : 'auto' }));
  }
}

/** Tokens, cost and how full the model's window is, for the conversation header. */
function usageMeter(live: { usage: UsageTotals; window: number } | undefined, stored: UsageTotals | undefined): HTMLElement | null {
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
    'span',
    { class: 'usage-meter', title: lines.join('. ') },
    pct !== null ? h('span', { class: 'ring', style: `--pct:${pct}` }) : null,
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
