/** A conversation's transcript: user turns, tool steps, results, reasoning and the parts of an answer. */
import type { Part, Turn } from '../core/api';
import { renderPageView } from './coderun';
import { h } from '../ui/h';
import { icon } from '../ui/icons';
import { markdown } from '../ui/markdown';
import { timeLabel } from '../core/clock';
import { navigate, state } from '../core/state';
import { renderCanvasCard } from '../canvas/canvas';
import '../canvas/diagrams';
import { renderCodeRun, renderImage, renderVideo } from './coderun';
import { diffLines } from './changes';
import { button } from '../ui/ui';
import { showWorkerSteps } from './side-panels';
import { IMAGE_TEMPLATES } from './send';
import { bookmarkButton, deleteExchangeButton, editButton } from './turn-actions';
import { argumentHint, toolLabel } from './tool-labels';

/**
 * How long each step of a conversation's latest reply took, by its position in the reply's parts.
 * Measured while the reply streams and kept for the session, so "Thought for 3s" survives the
 * reload from disk.
 */
export const stepDurations = new Map<string, number[]>();

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

export function host(url: string): string {
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
export function resultRows(part: Extract<Part, { kind: 'tool' }>): { heading: string; rows: ResultRow[] } | null {
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
          const source = href ? host(href) : typeof x.library === 'string' ? x.library : path ? (path.split(/[\\/]/).slice(-2, -1)[0] ?? '') : '';
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
      ...results.rows
        .slice(0, 3)
        .map((row) =>
          h(
            row.href ? 'a' : 'div',
            { class: 'result-row', ...(row.href ? { href: row.href, target: '_blank', rel: 'noopener noreferrer' } : {}) },
            icon(row.href ? 'globe' : 'file', 15),
            h('span', { class: 'result-title' }, row.title),
            h('span', { class: 'result-source' }, row.source)
          )
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
    h(
      'div',
      { class: 'step-content' },
      h(
        'div',
        { class: 'results-card' },
        run.flatMap((p) => runRows(p, live && p.output === null))
      )
    ),
    h('div', { class: 'step-content' }, raw)
  );
}

interface PlanStep {
  text: string;
  status: string;
}

export function planOf(part: Extract<Part, { kind: 'tool' }>): PlanStep[] {
  const steps = part.arguments.steps;
  return Array.isArray(steps) ? steps.filter((s): s is PlanStep => Boolean(s) && typeof (s as PlanStep).text === 'string') : [];
}

export function renderChecklist(steps: PlanStep[]): HTMLElement {
  return h(
    'div',
    { class: 'checklist' },
    steps.map((s) => h('div', { class: `check-item ${s.status}` }, h('span', { class: 'check-box' }, s.status === 'done' ? icon('check', 12) : s.status === 'in_progress' ? h('span', { class: 'check-dot' }) : null), h('span', {}, s.text)))
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
    reports = part.output ? ((JSON.parse(part.output) as { reports: WorkerReport[] }).reports ?? []) : [];
  } catch {
    reports = [];
  }
  const progress = state.subagents.get(part.callId) ?? [];
  const running = live && part.output === null;
  const doneCount = reports.filter((r) => r.status === 'done').length || progress.filter((p) => p?.status === 'done').length;
  const mode = typeof part.arguments.mode === 'string' ? part.arguments.mode : 'parallel';
  // A debate's second round comes after the tasks, one row per member.
  const count = Math.max(tasks.length, reports.length, progress.length);
  const rows = Array.from({ length: count }, (_, i) => {
    const task: { title?: string; instructions?: string } = tasks[i] ?? { title: reports[i]?.title ?? progress[i]?.title, instructions: 'Answers the other members’ first answers.' };
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
        h('span', { class: 'worker-activity muted' }, report ? `${report.steps} tool steps` : (now?.activity ?? (running ? 'Waiting' : '')))
      )
    );
    const body = h('div', { class: 'worker-body' }, h('div', { class: 'step-label' }, 'Task'), h('p', { class: 'muted' }, task.instructions ?? ''));
    if (report) {
      const md = h('div', { class: 'markdown small' });
      md.append(markdown(report.report));
      const steps = button({ variant: 'link', size: 'sm', icon: 'chevronRight', label: 'View every step', onClick: () => void showWorkerSteps(part.callId, i, task.title ?? `Task ${i + 1}`) });
      body.append(h('div', { class: 'step-label' }, 'Report'), md, steps);
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
      h(
        'span',
        { class: 'step-title' },
        running
          ? tasks.length === 1
            ? 'Running a sub-agent'
            : mode === 'sequential'
              ? `Running ${tasks.length} sub-agents one after another`
              : mode === 'debate'
                ? `${tasks.length} agents debating`
                : `Running ${tasks.length} sub-agents in parallel`
          : `Ran ${tasks.length === 1 ? 'a sub-agent' : `${tasks.length} sub-agents`}${mode === 'debate' ? ' in a two-round debate' : mode === 'sequential' ? ' in turn' : ''} · ${doneCount} reported`
      ),
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
    const lines = written.diff && written.diff !== 'unchanged' ? written.diff.split('\n').filter((l) => !l.startsWith('---') && !l.startsWith('+++')) : written.created ? content.split('\n').map((l) => `+${l}`) : [];
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
    h('div', { class: 'step-row' }, h('span', { class: 'step-icon' }, icon(live ? 'brain' : 'checkCircle', 16)), h('span', { class: 'step-title' }, live ? 'Thinking' : took ? `Thought for ${took}` : 'Thought')),
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
/** "Created index.html" or "Edited cart.py", as Vibe names file steps; null for other tools. */
function fileAction(part: Extract<Part, { kind: 'tool' }>): string | null {
  if (part.name !== 'filesystem__fs_write' && part.name !== 'filesystem__fs_edit') return null;
  const path = typeof part.arguments.path === 'string' ? part.arguments.path : '';
  const file = path.split(/[\\/]/).filter(Boolean).pop();
  if (!file) return null;
  if (part.output === null) return part.name === 'filesystem__fs_write' ? `Writing ${file}` : `Editing ${file}`;
  if (part.isError) return null;
  let created = false;
  try {
    created = Boolean((JSON.parse(part.output) as { created?: boolean }).created);
  } catch {
    created = false;
  }
  return part.name === 'filesystem__fs_edit' ? `Edited ${file}` : created ? `Created ${file}` : `Wrote ${file}`;
}

function groupSummary(parts: Part[], live: boolean): string {
  const last = parts[parts.length - 1];
  if (!last) return '';
  if (live) {
    const running = parts.filter((p) => p.kind === 'tool' && p.output === null).length;
    if (running > 1) return `Running ${running} actions`;
    return last.kind === 'tool' && last.output === null ? toolLabel(last.name).action : last.kind === 'reasoning' ? 'Thinking' : 'Working';
  }
  const tools = parts.filter((p): p is Extract<Part, { kind: 'tool' }> => p.kind === 'tool');
  if (!tools.length) {
    // "Thought for 4s", from the time measured while it streamed or the one stored with it.
    const ms = parts.reduce((n, p) => n + (p.kind === 'reasoning' ? (p.ms ?? 0) : 0), 0);
    const took = seconds(ms || undefined);
    return took ? `Thought for ${took}` : 'Thought';
  }
  if (tools.length === 1) return fileAction(tools[0]!) ?? toolLabel(tools[0]!.name).action;
  const counts = new Map<string, number>();
  let other = 0;
  const others: string[] = [];
  for (const t of tools) {
    const kind = KINDS.find(([re]) => re.test(t.name));
    if (kind) counts.set(kind[1], (counts.get(kind[1]) ?? 0) + 1);
    else {
      other += 1;
      const label = fileAction(t) ?? toolLabel(t.name).action;
      if (!others.includes(label)) others.push(label);
    }
  }
  const labels = KINDS.filter(([, one]) => counts.has(one)).map(([, one, many]) => {
    const n = counts.get(one)!;
    return `${n} ${n === 1 ? one : many}`;
  });
  // A few different steps read better by name ("Found tools · Create calendar event") than as a count.
  if (other && others.length <= 2) labels.push(...others);
  else if (other) labels.push(`${other} ${other === 1 ? 'tool call' : 'tool calls'}`);
  return labels.slice(0, 3).join(' · ');
}

/**
 * Text, and the activity between it, in the order the model produced them. Each run of activity
 * is one group: open while it is happening, folded under its last step once the answer follows.
 */
export function renderParts(parts: Part[], live: boolean, key: string, durations: number[] = []): HTMLElement[] {
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
        if (p.kind === 'reasoning') timeline.append(renderReasoning(p.text, stepLive, durations[index] ?? p.ms));
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
    if (part.kind === 'tool' && (part.name === 'browser_view' || part.name === 'browser_act')) {
      flush();
      out.push(renderPageView(part, live, key.split(':')[0] ?? ''));
      return;
    }
    if (part.kind === 'tool' && part.name === 'generate_image') {
      flush();
      out.push(renderImage(part, live, key.split(':')[0] ?? ''));
      return;
    }
    if (part.kind === 'tool' && (part.name === 'generate_video' || part.name === 'make_animation')) {
      flush();
      out.push(renderVideo(part, live, key.split(':')[0] ?? ''));
      return;
    }
    // Knowledge read and written shows as pills in the answer (Vibe's knowledge pills).
    if (part.kind === 'tool' && KNOWLEDGE_TOOLS.includes(part.name)) {
      flush();
      out.push(knowledgePill(part));
      return;
    }
    if (part.kind === 'tool' && part.name === 'run_code') {
      flush();
      out.push(renderCodeRun(part, live, key.split(':')[0] ?? ''));
      return;
    }
    if (part.kind === 'notice') {
      // Inline, as in Vibe 3.1.0: what happened to the reply stays with it.
      flush();
      out.push(h('div', { class: 'notice-line' }, icon('wait', 14), h('span', {}, part.text)));
      return;
    }
    if (part.kind === 'text') {
      flush();
      const body = h('div', { class: 'markdown' });
      body.append(markdown(part.text));
      if (live && i === parts.length - 1) {
        body.classList.add('streaming');
        fadeInFresh(body, part.text);
      }
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

/** The streamed text at the last render, to know which characters are new. */
let lastStreamed = '';

/**
 * Smooth streaming (LibreChat's fade-in): the characters that arrived since the last render are
 * wrapped in a span that fades in, once; what was already shown is left as it is, so nothing
 * flickers when the reply is rendered again.
 */
function fadeInFresh(body: HTMLElement, text: string) {
  let fresh = text.startsWith(lastStreamed) ? text.length - lastStreamed.length : 0;
  lastStreamed = text;
  if (fresh <= 0 || fresh > 400 || matchMedia('(prefers-reduced-motion: reduce)').matches) return;
  const nodes: Text[] = [];
  const walker = document.createTreeWalker(body, NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) nodes.push(n as Text);
  for (let k = nodes.length - 1; k >= 0 && fresh > 0; k--) {
    const node = nodes[k]!;
    if (!node.data.trim() || node.parentElement?.closest('pre, code, .katex')) {
      fresh -= node.data.length;
      continue;
    }
    const take = Math.min(fresh, node.data.length);
    const tail = node.splitText(node.data.length - take);
    const span = document.createElement('span');
    span.className = 'fresh';
    tail.replaceWith(span);
    span.append(tail);
    fresh -= take;
  }
}

const KNOWLEDGE_TOOLS = ['remember', 'forget', 'knowledge_write', 'knowledge_read'];

/** A knowledge read or write as a pill in the answer; it opens the Knowledge page. */
function knowledgePill(part: Extract<Part, { kind: 'tool' }>): HTMLElement {
  const arg = (k: string) => String(part.arguments[k] ?? '').trim();
  const pending = part.output === null;
  const label = (() => {
    if (part.isError) return part.name === 'remember' || part.name === 'knowledge_write' ? `Not saved: ${(part.output ?? '').replace(/^Not saved: /, '').split('.')[0]}` : 'Could not read knowledge';
    switch (part.name) {
      case 'remember':
        return `${pending ? 'Saving' : 'Saved'} to knowledge: ${arg('fact')}`;
      case 'forget':
        return pending ? 'Removing a fact' : 'Removed a fact from knowledge';
      case 'knowledge_write':
        return `${pending ? 'Writing' : arg('id') ? 'Updated' : 'Saved'} the knowledge page ${arg('title') ? `“${arg('title')}”` : ''}`.trim();
      default: {
        let title = arg('title') || arg('id');
        try {
          title = (JSON.parse(part.output ?? '{}') as { title?: string }).title ?? title;
        } catch {
          /* plain text output */
        }
        return `Read the knowledge page${title ? ` “${title}”` : ''}`;
      }
    }
  })();
  return h(
    'button',
    { class: `knowledge-pill${part.isError ? ' error' : ''}${pending ? ' pending' : ''}`, title: 'Open Knowledge', on: { click: () => navigate({ kind: 'page', page: 'knowledge' }) } },
    icon('book', 13),
    h('span', {}, label.length > 140 ? `${label.slice(0, 140)}…` : label)
  );
}

export function textOf(parts: Part[]): string {
  return parts
    .filter((p): p is Extract<Part, { kind: 'text' }> => p.kind === 'text')
    .map((p) => p.text)
    .join('\n\n');
}

export function renderUserTurn(turn: Turn, conversationId: string, branch: HTMLElement | null = null): HTMLElement {
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
  // A follow-up the user scheduled from this chat, running here.
  const followUp = rest.match(/^\(Scheduled follow-up: (.+?)\) /);
  if (followUp) {
    chips.push(h('span', { class: 'skill-chip' }, icon('clock', 14), `Scheduled: ${followUp[1]}`));
    rest = rest.slice(followUp[0].length);
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
  // Image mode's instruction (imageDirective) reads as a chip, as Grok shows its image tool.
  const imageAsk = rest.match(/^Create (an image|(\d) images \(variations\)) with generate_image \(shape: (\w+)[^)]*\)\.([^\n]*)\n\n/);
  if (imageAsk) {
    const template = IMAGE_TEMPLATES.find((t) => imageAsk[4]!.trim() && imageAsk[4]!.includes(t.brief.slice(0, 40)));
    const label = ['Image', template?.label, imageAsk[3] !== 'square' ? imageAsk[3]![0]!.toUpperCase() + imageAsk[3]!.slice(1) : null, imageAsk[2] ? `×${imageAsk[2]}` : null].filter(Boolean).join(' · ');
    chips.push(h('span', { class: 'skill-chip' }, icon('image', 14), label));
    rest = rest.slice(imageAsk[0].length);
  }
  const quote = rest.match(/^((?:> .*(?:\n|$))+)\n*/);
  let quoteEl: HTMLElement | null = null;
  if (quote) {
    quoteEl = h('div', { class: 'bubble-quote' }, icon('forward', 13), quote[1]!.replace(/^> /gm, '').trim());
    rest = rest.slice(quote[0].length);
  }
  const images = turn.parts.filter((p): p is Extract<Part, { kind: 'image' }> => p.kind === 'image');
  const gallery = images.length
    ? h(
        'div',
        { class: 'bubble-images' },
        images.map((i) => h('img', { src: `data:${i.mediaType};base64,${i.data}`, alt: 'Attached image' }))
      )
    : null;
  // Attached text files read as file chips that open in place, not as raw markup.
  const files: HTMLElement[] = [];
  rest = rest
    .replace(/\n*<attachment name="([^"]*)">\n?([\s\S]*?)\n?<\/attachment>/g, (_, name: string, body: string) => {
      files.push(h('details', { class: 'bubble-file' }, h('summary', {}, icon('file', 14), h('span', {}, name), h('small', { class: 'muted' }, `${body.split('\n').length} lines`)), h('pre', {}, body)));
      return '';
    })
    .trim();
  // Files kept in the Uploads library to be searched: chips too, without the note to the model.
  rest = rest
    .replace(/\n*\(Attached and kept in the "Uploads" library: ([^)]*)\. Search it with search_library[^)]*\)/g, (_, list: string) => {
      for (const m of list.matchAll(/"([^"]+)"/g))
        files.push(h('div', { class: 'bubble-file searched', title: 'Kept in the Uploads library; the assistant searches it' }, icon('library', 14), h('span', {}, m[1]!), h('small', { class: 'muted' }, 'searched')));
      return '';
    })
    .trim();
  const body = h('div', { class: 'bubble-text' }, rest);
  const bubble = h('div', { class: 'bubble' }, gallery, quoteEl, chips, body, files.length ? h('div', { class: 'bubble-files' }, files) : null);
  // Long messages (pasted documents, logs) fold to a few lines, with Show more.
  const long = rest.length > 1200 || rest.split('\n').length > 16;
  let toggle: HTMLElement | null = null;
  if (long) {
    body.classList.add('folded');
    toggle = h('button', { class: 'fold-toggle' }, 'Show more');
    toggle.addEventListener('click', () => {
      const folded = body.classList.toggle('folded');
      toggle!.textContent = folded ? 'Show more' : 'Show less';
    });
    bubble.append(toggle);
  }
  return h(
    'div',
    { class: 'turn turn-user', 'data-index': String(turn.index) },
    bubble,
    h(
      'div',
      { class: `turn-actions user-actions${branch ? ' pinned' : ''}` },
      branch,
      turn.index >= 0 ? editButton(turn, conversationId, text) : null,
      copy,
      turn.index >= 0 ? bookmarkButton(turn, conversationId, 15) : null,
      turn.index >= 0 ? deleteExchangeButton(turn, conversationId) : null,
      h('span', { class: 'turn-time' }, timeLabel(turn.time))
    )
  );
}

// The live timers tick between events, so "Working · 12s" keeps counting while a step runs.
setInterval(() => {
  document.querySelectorAll<HTMLElement>('.live-timer[data-start]').forEach((el) => {
    const start = Number(el.dataset.start);
    if (start) el.textContent = ` · ${Math.max(0, Math.round((Date.now() - start) / 1000))}s`;
  });
}, 1000);
