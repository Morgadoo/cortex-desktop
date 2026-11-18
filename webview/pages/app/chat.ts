/** The home screen, a conversation, and the composer they share. */
import { emojiFor, findEmoji } from './emoji';
import { autocompleteOn } from './state';
import { tauriInvoke } from '../../shared/tauri';
import { api, newId, type ApprovalMode, type ChatEvent, type ChatParams, type Conversation, type Part, type Turn, type UsageTotals } from './api';
import { openOutputsPanel } from './outputs';
import { conversationPng } from './chatimage';
import { searchInstead } from './uploads';
import { renderPageView } from './coderun';
import { agentPanel } from './agentpanel';
import { fillVariables } from './variables';
import { startVoiceConversation, voiceActive, voiceTurnEnded } from './voice';
import { attachVim } from './vim';
import { actionFor } from './shortcuts';
import { clear, confirmDialog, greeting, h, icon, markdown, modal, popover, promptText, timeLabel, toast, clockHour12 } from './dom';
import { applyEvent, changed, currentModel, followUpsOn, navigate, setMode, state, temporaryByDefault, type LiveTurn, followStreamOn } from './state';
import { openSettings } from './settings';
import { openInstructions } from './pages';
import { closePanel, enableAskAboutSelection, hasPanel, openSidePanel, renderCanvasCard, setPanelHost, lineDiff } from './canvas';
import { hostOf, previewFor, siteIcon } from './favicon';
import './diagrams';
import { renderCodeRun, renderImage, renderVideo } from './coderun';
import { startRecording, toBase64, type Recording } from './dictation';
import { audioSettings, shortError, speakText, type Playback } from './audio';
import { editWorkflow, liveWorkflow, workflowPrompt, workflowsApi } from './workflows';
import type { Speed, Workflow } from './api';
import { diffLines, loadChanges, openChanges, openFiles, openDebug } from './changes';
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
  find_tools: 'Found tools',
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
  find_tools: 'tool',
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
  generate_video: 'video',
  make_animation: 'video',
  browser_view: 'globe',
  browser_act: 'globe',
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

/** The modes, in menu order; the number keys pick them while the menu is open. */
const MODE_ORDER: ApprovalMode[] = ['auto', 'accept_edits', 'plan', 'ask'];
const MODES: Record<ApprovalMode, { name: string; desc: string }> = {
  auto: { name: 'Auto', desc: 'The assistant handles permission decisions' },
  accept_edits: { name: 'Accept edits', desc: 'Automatically accept all file edits' },
  plan: { name: 'Plan', desc: 'Create a plan before making changes' },
  ask: { name: 'Ask', desc: 'Ask before every change' },
};

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
        h('span', { class: 'worker-activity muted' }, report ? `${report.steps} tool steps` : now?.activity ?? (running ? 'Waiting' : ''))
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
      h('span', { class: 'step-title' }, running ? (tasks.length === 1 ? 'Running a sub-agent' : mode === 'sequential' ? `Running ${tasks.length} sub-agents one after another` : mode === 'debate' ? `${tasks.length} agents debating` : `Running ${tasks.length} sub-agents in parallel`) : `Ran ${tasks.length === 1 ? 'a sub-agent' : `${tasks.length} sub-agents`}${mode === 'debate' ? ' in a two-round debate' : mode === 'sequential' ? ' in turn' : ''} · ${doneCount} reported`),
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
    const ms = parts.reduce((n, p) => n + (p.kind === 'reasoning' ? p.ms ?? 0 : 0), 0);
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
    ? h('div', { class: 'bubble-images' }, images.map((i) => h('img', { src: `data:${i.mediaType};base64,${i.data}`, alt: 'Attached image' })))
    : null;
  // Attached text files read as file chips that open in place, not as raw markup.
  const files: HTMLElement[] = [];
  rest = rest.replace(/\n*<attachment name="([^"]*)">\n?([\s\S]*?)\n?<\/attachment>/g, (_, name: string, body: string) => {
    files.push(h('details', { class: 'bubble-file' }, h('summary', {}, icon('file', 14), h('span', {}, name), h('small', { class: 'muted' }, `${body.split('\n').length} lines`)), h('pre', {}, body)));
    return '';
  }).trim();
  // Files kept in the Uploads library to be searched: chips too, without the note to the model.
  rest = rest.replace(/\n*\(Attached and kept in the "Uploads" library: ([^)]*)\. Search it with search_library[^)]*\)/g, (_, list: string) => {
    for (const m of list.matchAll(/"([^"]+)"/g)) files.push(h('div', { class: 'bubble-file searched', title: 'Kept in the Uploads library; the assistant searches it' }, icon('library', 14), h('span', {}, m[1]!), h('small', { class: 'muted' }, 'searched')));
    return '';
  }).trim();
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
    h('div', { class: `turn-actions user-actions${branch ? ' pinned' : ''}` }, branch, turn.index >= 0 ? editButton(turn, conversationId, text) : null, copy, turn.index >= 0 ? bookmarkButton(turn, conversationId, 15) : null, turn.index >= 0 ? deleteExchangeButton(turn, conversationId) : null, h('span', { class: 'turn-time' }, timeLabel(turn.time)))
  );
}

/** Bookmarked turn indexes of the open conversations, kept in step with the saved ones. */
const bookmarked = new Map<string, Set<number>>();

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
function bookmarkButton(turn: Turn, conversationId: string, size = 16): HTMLElement {
  const on = () => bookmarked.get(conversationId)?.has(turn.index) ?? false;
  const button = h('button', { class: `icon-btn small${on() ? ' active' : ''}`, title: on() ? 'Remove bookmark' : 'Bookmark' }, icon('bookmark', size));
  button.addEventListener('click', async () => {
    const next = !on();
    try {
      await tauriInvoke('chat_bookmark', { id: conversationId, index: turn.index, on: next });
      const set = bookmarked.get(conversationId) ?? new Set<number>();
      if (next) set.add(turn.index);
      else set.delete(turn.index);
      bookmarked.set(conversationId, set);
      button.classList.toggle('active', next);
      button.title = next ? 'Remove bookmark' : 'Bookmark';
      toast(next ? 'Bookmarked. Find it in the chat header or with Ctrl+K.' : 'Bookmark removed.');
      await refreshConversations();
    } catch (e) {
      toast(String(e), 'error');
    }
  });
  return button;
}

/**
 * Prints the open conversation as it reads on screen (Markdown, code, math, diagrams), without the
 * app around it; the system print dialog can save it as a PDF.
 */
/** The conversation as one PNG (LobeHub's screenshot export). */
async function exportImage(id: string) {
  try {
    const c = await api.conversation(id);
    const messages = c.turns.map((t) => ({ role: t.role, text: textOf(t.parts) })).filter((m) => m.text.trim());
    const png = await conversationPng(c.title, messages);
    let binary = '';
    for (let i = 0; i < png.length; i += 0x8000) binary += String.fromCharCode(...png.subarray(i, i + 0x8000));
    const name = `${(c.title || 'conversation').replace(/[\\/:*?"<>|]+/g, ' ').trim().slice(0, 60) || 'conversation'}.png`;
    if (await tauriInvoke<boolean>('chat_save_bytes', { name, data: btoa(binary) })) toast('Image saved.');
  } catch (e) {
    toast(String(e), 'error');
  }
}

async function printConversation(id: string, title: string) {
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
  const head = h('div', {}, h('h1', { class: 'print-title' }, title || 'Conversation'), h('p', { class: 'print-meta' }, `Lumo · ${new Date().toLocaleString(undefined, { hour12: clockHour12() })}`));
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
        const branch = await api.forkConversation(conversationId, turn.index, withoutFiles);
        await refreshConversations();
        navigate({ kind: 'chat', id: branch });
        toast(withoutFiles ? 'Branched from that answer, without the attached files.' : 'Branched from that answer.');
      } catch (e) {
        toast(String(e), 'error');
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
    await tauriInvoke('chat_rewrite_message', { id: conversationId, index, text });
    await viewCache.chat?.refresh(false);
    toast('Saved.');
  } catch (e) {
    toast(String(e), 'error');
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
function answerActions(): AnswerAction[] {
  try {
    const saved = JSON.parse(localStorage.getItem('lumo.answerActions') ?? 'null') as AnswerAction[] | null;
    if (Array.isArray(saved)) return saved.filter((a) => a && typeof a.name === 'string' && typeof a.prompt === 'string');
  } catch {
    /* the defaults */
  }
  return DEFAULT_ANSWER_ACTIONS;
}

function saveAnswerActions(actions: AnswerAction[]) {
  try {
    localStorage.setItem('lumo.answerActions', JSON.stringify(actions));
  } catch {
    toast('The actions could not be saved.', 'error');
  }
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
      const list = [...rows.querySelectorAll('.action-row')].map((r) => ({ name: (r.querySelector('input') as HTMLInputElement).value.trim(), prompt: (r.querySelector('textarea') as HTMLTextAreaElement).value.trim() })).filter((a) => a.name && a.prompt);
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
        button({ variant: 'secondary', icon: 'plus', label: 'Add an action', onClick: () => { addRow({ name: '', prompt: '' }); const last = rows.lastElementChild; last?.scrollIntoView({ block: 'nearest' }); last?.querySelector('input')?.focus(); } }),
        button({ variant: 'subtle', label: 'Restore the built-in ones', onClick: () => { clear(rows); DEFAULT_ANSWER_ACTIONS.forEach(addRow); } }),
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
        h('button', { class: 'menu-item', on: { click: () => { close(); editAnswerActions(); } } }, icon('settings', 15), h('span', {}, 'Edit actions…'))
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
        const text = await tauriInvoke<string>('chat_translate', { model: state.model, text: textOf(turn.parts), language });
        clear(
          box,
          h('div', { class: 'translation-head' }, icon('globe', 14), language, h('span', { class: 'spacer' }), h('button', { class: 'link-btn', on: { click: () => box.remove() } }, 'Hide')),
          h('div', { class: 'markdown' }, markdown(text))
        );
      } catch (e) {
        box.remove();
        toast(String(e), 'error');
      }
    };
    const close = popover(
      button,
      h(
        'div',
        { class: 'menu' },
        h('div', { class: 'menu-heading' }, 'Translate into'),
        TRANSLATE_TO.map((l) => h('button', { class: 'menu-item', on: { click: () => { close(); void show(l); } } }, l)),
        h('button', { class: 'menu-item', on: { click: async () => { close(); const other = await promptText('Translate into', '', 'Any language'); if (other) void show(other); } } }, 'Other…')
      )
    );
  });
  return button;
}

/** Removes a sent message and the reply to it, to tidy a conversation. */
function deleteExchangeButton(turn: Turn, conversationId: string): HTMLElement {
  return h(
    'button',
    {
      class: 'icon-btn small',
      title: 'Delete this message and its reply',
      on: {
        click: async () => {
          if (!(await confirmDialog('Delete this message?', 'The message and the reply to it are removed from the conversation.'))) return;
          try {
            await tauriInvoke('chat_delete_exchange', { id: conversationId, index: turn.index });
            await viewCache.chat?.refresh(false);
          } catch (e) {
            toast(String(e), 'error');
          }
        },
      },
    },
    icon('trash', 15)
  );
}

/** Edit a sent message: the conversation is answered again from that point. */
function editButton(turn: Turn, conversationId: string, text: string): HTMLElement {
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
        await api.send({ conversationId, text: full(), model: state.model, speed: state.speed, approval: state.approval, editIndex: turn.index });
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

function renderAssistantTurn(conversation: Conversation, turn: Turn, isLast: boolean, running: boolean, branch: HTMLElement | null = null): HTMLElement {
  const durations = isLast ? stepDurations.get(conversation.id) : undefined;
  const el = h('div', { class: 'turn turn-assistant', 'data-index': String(turn.index) }, ...renderParts(turn.parts, false, `${conversation.id}:${turn.index}`, durations));
  numberLibraryCitations(el, turn.parts);
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
  snippet: string;
  /** Linked in the answer or read; false for search results it did not use (Onyx's "found"). */
  cited: boolean;
}

/** What an answer drew on: the pages it links to, then any it opened; search hits when it cites none. */
function sourcesOf(parts: Part[]): Source[] {
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
  const chip = h('button', { class: 'sources-chip' }, 'Sources', h('span', { class: 'source-dots' }, distinct.slice(0, 3).map((s) => siteIcon(s.url, 'source-dot'))));
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
      if (reason) await tauriInvoke('chat_rate_reason', { id: conversation.id, index: turn.index, reason }).catch(() => undefined);
    }
  };
  const copy = h('button', { class: 'icon-btn small', title: 'Copy' }, icon('copy', 16));
  copy.addEventListener('click', () => {
    // Formatted for Office and mail apps, plain Markdown for editors (LibreChat's rich copy).
    const text = textOf(turn.parts);
    const html = h('div', {}, markdown(text)).innerHTML;
    const rich = typeof ClipboardItem !== 'undefined'
      ? navigator.clipboard.write([new ClipboardItem({ 'text/html': new Blob([html], { type: 'text/html' }), 'text/plain': new Blob([text], { type: 'text/plain' }) })])
      : Promise.reject(new Error('no rich clipboard'));
    void rich.catch(() => navigator.clipboard.writeText(text));
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
  const compare = iconButton({ icon: 'columns', title: 'Compare two versions', size: 'xs', onClick: () => void compareVersions(conversationId, versions.from, versions.current) });
  return h('span', { class: 'versions', title: versions.kind === 'prompt' ? 'Versions of this message and what followed' : 'Answers to this message' }, prev, h('span', { class: 'versions-label' }, `${versions.current + 1} / ${versions.total}`), next, compare);
}

/** Two versions of an answer side by side as one text, words removed and added marked (Open WebUI's Compare Text). */
async function compareVersions(conversationId: string, from: number, current: number) {
  let texts: string[];
  try {
    texts = await tauriInvoke<string[]>('chat_version_texts', { id: conversationId, from });
  } catch (e) {
    return void toast(String(e), 'error');
  }
  if (texts.length < 2) return;
  void modal((close) => {
    const pick = (value: number) => h('select', { class: 'input select-sm' }, texts.map((_, i) => h('option', { value: String(i), selected: i === value }, `Version ${i + 1}`)));
    const left = pick(current > 0 ? current - 1 : 1);
    const right = pick(current);
    const out = h('div', { class: 'word-diff' });
    const render = () => {
      const a = texts[Number(left.value)] ?? '';
      const b = texts[Number(right.value)] ?? '';
      // Words, with line breaks kept as their own token; runs of one kind join into one mark.
      const NL = '\u2028';
      const tokens = (t: string) => t.replace(/\n+/g, ` ${NL} `).split(/[ \t]+/).filter((x) => x !== '');
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
        words.join(' ').split(NL).flatMap((part, i) => (i ? [h('br'), part.trim()] : [part.trim()]));
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
    return h(
      'div',
      { class: 'dialog-small wide' },
      modalHeader('Compare versions', close, { flush: true }),
      h('div', { class: 'freq-row' }, left, h('span', { class: 'muted' }, '\u2192'), right, h('span', { class: 'spacer' }), stats),
      out
    );
  });
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
/** Automatic read-aloud stops for the session after it fails, rather than failing on every answer. */
let autoPlayFailed = false;

export function autoPlay(text: string) {
  if (autoPlayFailed || !audioSettings?.tts.autoPlay || audioSettings.tts.engine === 'off' || !text.trim() || voiceActive()) return;
  stopSpeaking();
  void speakText(text)
    .then((playback) => {
      speaking = { playback, button: null };
    })
    .catch((e) => {
      autoPlayFailed = true;
      toast(`${String(e).replace(/^Error:\s*/, '')} Reading answers aloud is paused until Lumo restarts.`, 'error');
    });
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
      toast(String(e).replace(/^Error:\s*/, ''), 'error');
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
    const last = live.parts[live.parts.length - 1];
    // While it reasons, the first words of the thought say what it is working out.
    const hint = last?.kind === 'reasoning' ? last.text.trim().split(/(?<=[.!?])\s/)[0]?.slice(0, 90) ?? '' : '';
    el.append(
      h(
        'div',
        { class: 'status-line' },
        h('span', { class: 'dots' }),
        last?.kind === 'reasoning' ? 'Thinking' : 'Working',
        hint ? h('span', { class: 'status-hint' }, ` · ${hint}`) : null,
        h('span', { class: 'live-timer', 'data-start': String(live.startedAt) }, ` · ${Math.max(0, Math.round((now - live.startedAt) / 1000))}s`)
      )
    );
  }
  if (live.error?.startsWith('Not sent. ')) {
    // A filter of the user's refused the message: nothing failed, and nothing to continue.
    el.append(h('div', { class: 'error-card notice' }, icon('shield', 16), h('div', { class: 'grow' }, h('strong', {}, 'Not sent'), h('div', {}, live.error.slice('Not sent. '.length)), h('p', { class: 'hint' }, 'A filter under Settings → Permissions → Functions stopped this message. Change it and send again.'))));
  } else if (live.error) {
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
        h('div', { class: 'error-actions' }, copyErrorButton(live.error, conversationId), resume)
      )
    );
  }
  return el;
}

/** Copies what a bug report needs: the error, the model, the app version and the platform. */
function copyErrorButton(error: string, conversationId: string): HTMLElement {
  const button = h('button', { class: 'btn btn-ghost btn-sm', title: 'Copy the error with the model, app version and platform, for a bug report' }, icon('copy', 14), 'Copy details');
  button.addEventListener('click', async () => {
    let version = '';
    try {
      version = await tauriInvoke<string>('plugin:app|version');
    } catch {
      version = 'unknown';
    }
    const details = [
      `Error: ${error}`,
      `Model: ${state.model ?? 'none'}`,
      `Mode: ${state.approval}, speed: ${state.speed}`,
      `App: Lumo ${version}`,
      `Platform: ${navigator.platform} (${navigator.userAgent.match(/AppleWebKit\/[\d.]+/)?.[0] ?? 'webview'})`,
      `Conversation: ${conversationId}`,
      `Time: ${new Date().toISOString()}`,
    ].join('\n');
    await navigator.clipboard.writeText(details);
    toast('Error details copied.');
  });
  return button;
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
  let workdir = !conversationId && state.mode === 'code' ? state.workdir : null;
  // A new Code session can work in its own git worktree, on a branch of its own.
  if (workdir && worktreeNext()) {
    try {
      const [folder, branch] = await tauriInvoke<[string, string]>('chat_create_worktree', { dir: workdir, name: text.split('\n')[0]!.slice(0, 60) });
      const settings = await api.appSettings();
      if (!settings.filesystem_allowed_roots.includes(folder)) settings.filesystem_allowed_roots.push(folder);
      await api.saveAppSettings(settings);
      workdir = folder;
      toast(`Working in a new worktree on ${branch}.`);
    } catch (e) {
      toast(String(e), 'error');
      return false;
    }
  }
  const temporary = !conversationId && temporaryNext;
  if (conversationId) followUps.delete(conversationId);
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
      toolsOff: [...toolsOff],
    });
    if (temporary) temporaryChats.add(id);
    temporaryNext = temporaryByDefault();
    void refreshConversations();
  } catch (e) {
    toast(String(e), 'error');
    return false;
  }
  return true;
}

/**
 * The assistant's built-in tools, each switchable for the chats you send from the + menu's
 * Tools (as Grok and Vibe show them): image generation is one tool among the others.
 */
const TOOL_GROUPS: { label: string; icon: string; names: string[]; hint: string }[] = [
  { label: 'Web search', icon: 'globe', names: ['web_search'], hint: 'Search the web and cite sources' },
  { label: 'Image generation', icon: 'image', names: ['generate_image'], hint: 'Create and edit images' },
  { label: 'Video generation', icon: 'video', names: ['generate_video'], hint: 'Short clips, or an image brought to life' },
  { label: 'Animations', icon: 'video', names: ['make_animation'], hint: 'Explainers, infographics and titles as code' },
  { label: 'Browser', icon: 'globe', names: ['browser_view', 'browser_act', 'browser'], hint: 'Open, read and use web pages' },
  { label: 'Canvas', icon: 'file', names: ['canvas'], hint: 'Documents, apps and sheets beside the chat' },
  { label: 'Run code', icon: 'terminal', names: ['run_code'], hint: 'Python and JavaScript in a sandbox' },
  { label: 'Sub-agents', icon: 'workflow', names: ['delegate'], hint: 'Research in parallel' },
  { label: 'Message agents', icon: 'chat', names: ['message_agent'], hint: 'Ask your other agents' },
  { label: 'Schedule tasks', icon: 'clock', names: ['schedule_task'], hint: 'Runs at set times' },
  { label: 'Knowledge', icon: 'book', names: ['remember', 'forget', 'knowledge_write'], hint: 'Save what it learns about you' },
];
const toolsOff = new Set<string>();

/** Image templates (Grok's "Modelo" cards): what to ask first and how to draw it. */
const IMAGE_TEMPLATES: { id: string; label: string; icon: string; brief: string }[] = [
  { id: 'mascot', label: 'Mascot maker', icon: 'sparkle', brief: 'This is a mascot or avatar. Unless I said, first ask me with ask_user for the style (options: Polished 2D game mascot, Chibi cute, Retro cartoon, Flat vector) and then the background (options: Plain white, Transparent look, In a scene). Then design one friendly, memorable character that fits the subject, centred, full body.' },
  { id: 'logo', label: 'Logo', icon: 'star', brief: 'This is a logo. Unless I said, first ask me with ask_user for the style (options: Minimal wordmark, Emblem or badge, Geometric icon, Hand-drawn) and the colours. Keep it simple, readable at small sizes, on a plain background.' },
  { id: 'sticker', label: 'Sticker', icon: 'image', brief: 'This is a die-cut sticker: bold outlines, a white border, plain background. Unless I said, ask me with ask_user for the style (options: Kawaii, Cartoon, Pixel art, Watercolour).' },
  { id: 'poster', label: 'Poster', icon: 'file', brief: 'This is a poster. Unless I said, ask me with ask_user for the mood (options: Bold and modern, Vintage, Minimal, Playful) and any text to show; keep text short and spelled exactly.' },
  { id: 'product', label: 'Product photo', icon: 'image', brief: 'This is a product photo. Unless I said, ask me with ask_user for the setting (options: White studio, Lifestyle scene, Flat lay, Dramatic dark). Realistic lighting, sharp focus on the product.' },
];

/** The message for image mode: use the image tool, with the chosen shape, count and template. */
function imageDirective(img: { shape: string; count: number; template: string | null }, text: string): string {
  const t = IMAGE_TEMPLATES.find((x) => x.id === img.template);
  const what = img.count > 1 ? `${img.count} images (variations)` : 'an image';
  return `Create ${what} with generate_image (shape: ${img.shape}${img.count > 1 ? `, count: ${img.count}` : ''}).${t ? ` ${t.brief}` : ''}\n\n${text || 'Surprise me.'}`;
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
let temporaryNext = temporaryByDefault();
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
  /** A file from disk (not a chat, page or canvas), kept in the recent files once sent. */
  file?: boolean;
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
  /** Image mode (the image generation tool, explicitly): its shape, how many, and a template. */
  private image: { shape: 'square' | 'landscape' | 'portrait'; count: number; template: string | null } | null = null;
  private chips: HTMLElement;
  private bar: HTMLElement;
  private conversationId: string | null = null;
  private placeholder = 'Type / for quick access';

  constructor() {
    this.textarea = h('textarea', { class: 'composer-input', rows: 1, placeholder: this.placeholder });
    // Vim keys (Settings → General), ahead of the box's own keys; menus keep theirs.
    attachVim(this.textarea, () => !!this.slashMenu || !!document.querySelector('.popover'), (mode) => {
      this.el.dataset.vim = mode;
    });
    this.textarea.addEventListener('input', () => {
      this.scheduleSuggestion();
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
      // Autocomplete: Tab takes the suggestion, Esc dismisses it.
      if (this.suggestion && e.key === 'Tab' && !e.shiftKey) {
        e.preventDefault();
        this.acceptSuggestion();
        return;
      }
      if (this.suggestion && e.key === 'Escape') {
        e.preventDefault();
        this.clearSuggestion();
        return;
      }
      // Shift+Tab cycles the modes, as in Claude. GTK names it ISO_Left_Tab, so match the code too.
      if (actionFor(e) === 'cycleMode') {
        e.preventDefault();
        setMode(MODE_ORDER[(MODE_ORDER.indexOf(state.approval) + 1) % MODE_ORDER.length]!);
        this.renderBar();
        return;
      }
      // Prompt history, as in Vibe: Up and Down walk earlier prompts; Ctrl+R searches them.
      if ((e.key === 'ArrowUp' || e.key === 'ArrowDown') && !e.shiftKey && this.walkHistory(e.key === 'ArrowUp' ? 1 : -1)) {
        e.preventDefault();
        return;
      }
      if (actionFor(e) === 'promptHistory') {
        e.preventDefault();
        this.searchHistory();
        return;
      }
      if (e.key === 'e' && (e.ctrlKey || e.metaKey) && !e.shiftKey) {
        e.preventDefault();
        void this.extendPrompt();
        return;
      }
      // Ctrl+Z right after Improve prompt puts the prompt as it was typed back.
      if (e.key === 'z' && (e.ctrlKey || e.metaKey) && this.beforeExtend && this.textarea.value === this.beforeExtend.after) {
        e.preventDefault();
        this.setText(this.beforeExtend.before);
        this.beforeExtend = null;
        return;
      }
      if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
        e.preventDefault();
        void this.submit(e.altKey);
      }
    });
    this.textarea.addEventListener('paste', async (e) => {
      // A long paste goes in as an attachment, keeping the message readable (LibreChat).
      const pasted = e.clipboardData?.getData('text/plain') ?? '';
      if (pasted.length > 2500) {
        e.preventDefault();
        const lines = pasted.split('\n').length;
        this.attachments.push({ name: `Pasted text (${lines} lines)`, text: pasted });
        this.renderChips();
        this.renderBar();
        toast('Long paste added as an attachment.');
        return;
      }
      // WebKitGTK exposes pasted images through `items` only; other engines through `files` too.
      const fromItems = Array.from(e.clipboardData?.items ?? [])
        .filter((i) => i.kind === 'file' && IMAGE_TYPES.includes(i.type))
        .map((i) => i.getAsFile())
        .filter((f): f is File => f !== null);
      const files = fromItems.length ? fromItems : Array.from(e.clipboardData?.files ?? []).filter((f) => IMAGE_TYPES.includes(f.type));
      const others = Array.from(e.clipboardData?.files ?? []).filter((f) => !IMAGE_TYPES.includes(f.type));
      if (!files.length && others.length) {
        e.preventDefault();
        void this.addFiles(others);
        return;
      }
      if (!files.length) return;
      e.preventDefault();
      for (const file of files) this.attachments.push({ name: file.name || 'pasted image', text: '', image: await readImage(file) });
      this.renderChips();
      this.renderBar();
    });
    this.chips = h('div', { class: 'chips' });
    this.bar = h('div', { class: 'composer-bar' });
    this.hint.addEventListener('mousedown', (e) => { e.preventDefault(); this.acceptSuggestion(); });
    this.el = h('div', { class: 'composer' }, h('div', { class: 'composer-box' }, this.chips, this.hint, this.textarea, this.bar));
    // Files dropped on the composer are attached like picked ones.
    let depth = 0;
    const hasFiles = (e: DragEvent) => Array.from(e.dataTransfer?.types ?? []).includes('Files');
    this.el.addEventListener('dragenter', (e) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      depth++;
      this.el.classList.add('drop-target');
    });
    this.el.addEventListener('dragover', (e) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
    });
    this.el.addEventListener('dragleave', () => {
      depth = Math.max(0, depth - 1);
      if (!depth) this.el.classList.remove('drop-target');
    });
    this.el.addEventListener('drop', (e) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      depth = 0;
      this.el.classList.remove('drop-target');
      void this.addFiles(Array.from(e.dataTransfer?.files ?? []));
    });
    this.renderBar();
  }

  private slashMenu: { close: () => void; items: string[]; index: number; list: HTMLElement } | null = null;
  /** Where Up/Down is in the prompt history: -1 is the draft being typed. */
  private historyIndex = -1;
  private draft = '';

  /** Steps through earlier prompts while the caret is on the first (Up) or last (Down) line. */
  private walkHistory(step: 1 | -1): boolean {
    const value = this.textarea.value;
    const caret = this.textarea.selectionStart;
    const atEdge = step === 1 ? !value.slice(0, caret).includes('\n') : !value.slice(caret).includes('\n');
    // Only from an empty composer, or while already walking, so arrows still move the caret.
    if (!atEdge || (this.historyIndex === -1 && value.trim())) return false;
    const history = promptHistory();
    const next = this.historyIndex + step;
    if (next < -1 || next >= history.length) return false;
    if (this.historyIndex === -1) this.draft = value;
    this.historyIndex = next;
    this.setText(next === -1 ? this.draft : history[next]!);
    return true;
  }

  /** The prompt before Improve prompt rewrote it, for Ctrl+Z. */
  private beforeExtend: { before: string; after: string } | null = null;
  private extending = false;

  /**
   * Improve prompt (LobeHub's Prompt Extend): the model rewrites what is typed into a clearer,
   * more complete prompt before it is sent. Ctrl+Z puts the original back.
   */
  async extendPrompt() {
    const before = this.textarea.value;
    if (!before.trim()) return toast('Type a prompt first.');
    if (this.extending) return;
    if (!state.model) return toast('Choose a model first.', 'error');
    this.extending = true;
    this.el.classList.add('extending');
    toast('Improving the prompt…');
    try {
      const after = await tauriInvoke<string>('chat_improve_text', {
        model: state.model,
        text: before,
        instruction:
          'This is a prompt the user is about to send to an AI assistant. Rewrite it as a clearer, more complete prompt: ' +
          'state the goal, the useful context and constraints the user implied, and the form of the answer wanted. ' +
          'Keep the user\'s language, voice and every detail; do not answer it, and do not invent facts or requirements. ' +
          'Keep {{placeholders}}, /commands and @mentions as they are. Keep it short when the request is simple.',
        context: null,
      });
      if (this.textarea.value !== before) return; // The user kept typing: leave their text.
      this.setText(after);
      this.beforeExtend = { before, after };
      this.focus();
      toast('Prompt improved. Ctrl+Z puts yours back.');
    } catch (e) {
      toast(String(e), 'error');
    } finally {
      this.extending = false;
      this.el.classList.remove('extending');
    }
  }

  /** The suggested continuation of what is typed, shown above the composer (Open WebUI's autocomplete). */
  private suggestion = '';
  private suggestionTimer = 0;
  private suggestionSeq = 0;
  private readonly hint = h('button', { class: 'autocomplete-hint', type: 'button', hidden: true, title: 'Tab to add it, Esc to dismiss' });

  private clearSuggestion() {
    this.suggestion = '';
    this.suggestionSeq++;
    this.hint.hidden = true;
  }

  private acceptSuggestion() {
    const text = this.textarea.value + this.suggestion;
    this.clearSuggestion();
    this.setText(text);
    this.textarea.setSelectionRange(text.length, text.length);
    this.focus();
  }

  private scheduleSuggestion() {
    window.clearTimeout(this.suggestionTimer);
    this.clearSuggestion();
    const typed = this.textarea.value;
    if (!autocompleteOn() || !state.model || this.slashMenu || typed.trim().length < 8 || typed.startsWith('/') || this.textarea.selectionStart !== typed.length) return;
    this.suggestionTimer = window.setTimeout(() => {
      const seq = ++this.suggestionSeq;
      void tauriInvoke<string>('chat_complete_prompt', { model: state.model, text: typed })
        .then((continuation) => {
          // Only while the text is as it was when asked.
          if (seq !== this.suggestionSeq || this.textarea.value !== typed || !continuation.trim()) return;
          this.suggestion = continuation;
          clear(this.hint, h('span', { class: 'muted' }, '…' + typed.slice(-24).trimStart()), h('strong', {}, continuation), h('kbd', {}, 'Tab'));
          this.hint.hidden = false;
        })
        .catch(() => undefined);
    }, 700);
  }

  /** Ctrl+R: search earlier prompts and put one in the composer. */
  private searchHistory() {
    const history = promptHistory();
    if (!history.length) return toast('No earlier prompts yet.');
    void modal((close) => {
      const input = h('input', { class: 'input', placeholder: 'Search your earlier prompts' });
      const results = h('div', { class: 'search-results' });
      const render = () => {
        const q = input.value.toLowerCase().trim();
        const found = history.filter((p) => !q || p.toLowerCase().includes(q)).slice(0, 30);
        clear(results, found.length ? found.map((p) => h('button', { class: 'menu-item', on: { click: () => { close(); this.setText(p); this.focus(); } } }, icon('clock', 15), h('span', { class: 'history-text' }, p))) : h('p', { class: 'muted' }, 'No match.'));
      };
      input.addEventListener('input', render);
      input.addEventListener('keydown', (e) => { if (e.key === 'Enter') (results.querySelector('button') as HTMLButtonElement | null)?.click(); });
      render();
      setTimeout(() => input.focus());
      return h('div', { class: 'dialog-small wide' }, input, results);
    });
  }

  /** What the `#` menu offers besides chats, loaded each time it opens. */
  private hashSources: { libraries: string[]; pages: { id: string; title: string }[]; canvases: { id: string; title: string; kind: string }[] } | null = null;

  /**
   * `#` adds context where you type it (Open WebUI's `#`): a library to search, a knowledge page,
   * a canvas or another chat, attached in full. The `#word` is taken out of the message.
   */
  private updateHashMenu(): boolean {
    const before = this.textarea.value.slice(0, this.textarea.selectionStart);
    const match = before.match(/(?:^|\s)#([^\s#]*)$/);
    if (!match) {
      this.hashSources = null;
      return false;
    }
    if (this.hashSources === null) {
      this.hashSources = { libraries: [], pages: [], canvases: [] };
      const sources = this.hashSources;
      void Promise.all([
        tauriInvoke<{ name: string }[]>('chat_list_libraries').catch(() => []),
        tauriInvoke<{ id: string; title: string }[]>('chat_knowledge_pages').catch(() => []),
        tauriInvoke<{ id: string; title: string; kind: string }[]>('chat_list_canvases').catch(() => []),
      ]).then(([libs, pages, canvases]) => {
        sources.libraries = libs.map((l) => l.name);
        sources.pages = pages;
        sources.canvases = canvases;
        if (this.hashSources === sources) this.updateSlashMenu();
      });
    }
    const q = match[1]!.toLowerCase();
    const has = (t: string) => (t || '').toLowerCase().includes(q);
    const groups: { heading: string; icon: Parameters<typeof icon>[0]; entries: { key: string; label: string }[] }[] = [
      { heading: 'Search a library', icon: 'library', entries: this.hashSources.libraries.filter(has).slice(0, 4).map((n) => ({ key: `lib:${n}`, label: n })) },
      { heading: 'Add a knowledge page', icon: 'book', entries: this.hashSources.pages.filter((p) => has(p.title)).slice(0, 4).map((p) => ({ key: `page:${p.id}`, label: p.title || 'Untitled' })) },
      { heading: 'Add a canvas', icon: 'file', entries: this.hashSources.canvases.filter((c) => has(c.title)).slice(0, 4).map((c) => ({ key: `canvas:${c.id}`, label: c.title || 'Untitled' })) },
      {
        heading: 'Add a chat as context',
        icon: 'chat',
        entries: state.conversations.filter((c) => c.id !== this.conversationId && !c.archived && has(c.title)).slice(0, 5).map((c) => ({ key: `chat:${c.id}`, label: c.title || 'Untitled' })),
      },
    ].filter((g) => g.entries.length);
    if (!groups.length) {
      this.slashMenu?.close();
      this.slashMenu = null;
      return true;
    }
    const items = groups.flatMap((g) => g.entries.map((e) => e.key));
    const index = Math.min(this.slashMenu?.index ?? 0, items.length - 1);
    const list = h('div', { class: 'menu slash-menu' });
    for (const g of groups) {
      list.append(h('div', { class: 'menu-heading' }, g.heading));
      for (const e of g.entries) {
        const i = items.indexOf(e.key);
        list.append(h('button', { class: `menu-item${i === index ? ' selected' : ''}`, on: { mousedown: (ev: MouseEvent) => { ev.preventDefault(); this.pickSkill(e.key); } } }, icon(g.icon, 16), h('span', {}, e.label)));
      }
    }
    this.slashMenu?.close();
    const close = popover(this.textarea, list, 'above');
    this.slashMenu = { close, items, index, list };
    return true;
  }

  /**
   * `:name` lists emoji by shortcode (Open WebUI's and Slack's `:`); a closing `:` on an exact
   * shortcode turns it into the emoji at once.
   */
  private updateEmojiMenu(): boolean {
    const caret = this.textarea.selectionStart;
    const before = this.textarea.value.slice(0, caret);
    const closed = before.match(/(^|\s):([a-z0-9_+-]+):$/i);
    const exact = closed ? emojiFor(closed[2]!) : undefined;
    if (closed && exact) {
      this.replaceBeforeCaret(closed[0].length - closed[1]!.length, exact);
      this.slashMenu?.close();
      this.slashMenu = null;
      return true;
    }
    const match = before.match(/(?:^|\s):([a-z0-9_+-]{2,})$/i);
    if (!match) return false;
    const found = findEmoji(match[1]!);
    this.slashMenu?.close();
    this.slashMenu = null;
    if (!found.length) return true;
    const items = found.map(([name]) => `emoji:${name}`);
    const index = 0;
    const list = h('div', { class: 'menu slash-menu' }, h('div', { class: 'menu-heading' }, 'Emoji'));
    found.forEach(([name, emoji], i) =>
      list.append(
        h('button', { class: `menu-item${i === index ? ' selected' : ''}`, on: { mousedown: (e: MouseEvent) => { e.preventDefault(); this.pickSkill(`emoji:${name}`); } } }, h('span', { class: 'emoji-glyph' }, emoji), h('span', {}, `:${name}:`))
      )
    );
    const close = popover(this.textarea, list, 'above');
    this.slashMenu = { close, items, index, list };
    return true;
  }

  /** Replaces the `count` characters before the caret with `text`, keeping the rest. */
  private replaceBeforeCaret(count: number, text: string) {
    const caret = this.textarea.selectionStart;
    const value = this.textarea.value;
    this.setText(value.slice(0, caret - count) + text + value.slice(caret));
    const at = caret - count + text.length;
    this.textarea.setSelectionRange(at, at);
  }

  /** Takes the `#word` being typed out of the message. */
  private dropHashWord() {
    const caret = this.textarea.selectionStart;
    const before = this.textarea.value.slice(0, caret).replace(/(^|\s)#[^\s#]*$/, '$1');
    this.setText(before + this.textarea.value.slice(caret));
  }

  /** Typing `/` (or `@`, as Vibe Work does) at the start lists the enabled skills; picking one inserts `/name `. */
  /** The folder of the coding session being written in (an open one, or a new Code chat's). */
  private workdir(): string | null {
    const open = this.conversationId ? state.conversations.find((c) => c.id === this.conversationId) : null;
    if (open) return open.workdir ?? null;
    return state.mode === 'code' ? state.workdir : null;
  }

  private folderFiles: { dir: string; files: string[] } | null = null;

  /**
   * `@path` in a coding session (Vibe's @ file mentions): the folder's files (git-aware: tracked
   * and new, not ignored), matched anywhere in the path; picking one attaches its text.
   */
  private updateFileMenu(): boolean {
    const dir = this.workdir();
    if (!dir) return false;
    const before = this.textarea.value.slice(0, this.textarea.selectionStart);
    const match = before.match(/(?:^|\s)@([^\s@]*)$/);
    // `@` alone at the start still offers models, agents and skills; a path character makes it a file.
    if (!match || (before.startsWith('@') && match.index === 0 && !/[./_-]/.test(match[1]!) && match[1]!.length < 3)) return false;
    if (this.folderFiles?.dir !== dir) {
      this.folderFiles = { dir, files: [] };
      const asked = this.folderFiles;
      void tauriInvoke<[string[], boolean]>('chat_folder_files', { dir })
        .then(([files]) => {
          asked.files = files;
          if (this.folderFiles === asked) this.updateSlashMenu();
        })
        .catch(() => undefined);
    }
    const q = match[1]!.toLowerCase();
    // Matches in the file name first, then anywhere in the path; shorter paths first.
    const hits = this.folderFiles.files
      .filter((f) => f.toLowerCase().includes(q))
      .sort((a, b) => Number(!a.toLowerCase().split('/').pop()!.includes(q)) - Number(!b.toLowerCase().split('/').pop()!.includes(q)) || a.length - b.length)
      .slice(0, 8);
    if (!hits.length) {
      this.slashMenu?.close();
      this.slashMenu = null;
      return true;
    }
    const items = hits.map((f) => `file:${f}`);
    const index = Math.min(this.slashMenu?.index ?? 0, items.length - 1);
    const list = h('div', { class: 'menu slash-menu' }, h('div', { class: 'menu-heading' }, 'Add a file'));
    hits.forEach((f, i) => {
      const slash = f.lastIndexOf('/');
      list.append(
        h(
          'button',
          { class: `menu-item rich${i === index ? ' selected' : ''}`, on: { mousedown: (e: MouseEvent) => { e.preventDefault(); this.pickSkill(`file:${f}`); } } },
          icon('file', 16),
          h('span', {}, h('strong', { class: 'no-translate' }, f.slice(slash + 1)), h('small', { class: 'mono no-translate' }, slash > 0 ? f.slice(0, slash) : './'))
        )
      );
    });
    this.slashMenu?.close();
    const close = popover(this.textarea, list, 'above');
    this.slashMenu = { close, items, index, list };
    return true;
  }

  private updateSlashMenu() {
    if (this.updateFileMenu()) return;
    if (this.updateHashMenu()) return;
    if (this.updateEmojiMenu()) return;
    const match = this.textarea.value.match(/^([/@])([a-z0-9-]*)$/);
    // The skills used most come first: the menu learns from use.
    const uses = skillUses();
    const skills = match ? state.skills.filter((s) => s.enabled && s.name.startsWith(match[2]!)).sort((a, b) => (uses[b.name] ?? 0) - (uses[a.name] ?? 0)) : [];
    // App commands come first after `/` (Open WebUI's built-in slash commands).
    const commands = match && match[1] === '/' ? SLASH_COMMANDS.filter((c) => c.name.startsWith(match[2]!) && (c.inChat ? Boolean(this.conversationId) : true)) : [];
    // `@` also switches the model (Open WebUI's @model), matched on its name or id.
    const key = (t: string) => t.toLowerCase().replace(/[^a-z0-9]+/g, '');
    const models = match && match[1] === '@' ? state.models.filter((m) => key(m.name).startsWith(key(match[2]!)) || key(m.id).startsWith(key(match[2]!))).slice(0, 6) : [];
    // And `@` starts a chat with one of your agents (LibreChat's @agent).
    const agents = match && match[1] === '@' ? state.agents.filter((a) => key(a.name).startsWith(key(match[2]!))).slice(0, 5) : [];
    if (!skills.length && !commands.length && !models.length && !agents.length) {
      this.slashMenu?.close();
      this.slashMenu = null;
      return;
    }
    const list = h('div', { class: 'menu slash-menu' });
    const items = [...commands.map((c) => `cmd:${c.name}`), ...models.map((m) => `model:${m.reference}`), ...agents.map((a) => `agent:${a.id}`), ...skills.map((s) => s.name)];
    const index = Math.min(this.slashMenu?.index ?? 0, items.length - 1);
    if (commands.length) list.append(h('div', { class: 'menu-heading' }, 'Commands'));
    commands.forEach((c, i) =>
      list.append(
        h(
          'button',
          { class: `menu-item rich${i === index ? ' selected' : ''}`, on: { mousedown: (e: MouseEvent) => { e.preventDefault(); this.pickSkill(`cmd:${c.name}`); } } },
          icon(c.icon, 17),
          h('span', {}, h('strong', {}, `/${c.name}`), h('small', {}, c.desc))
        )
      )
    );
    if (models.length) list.append(h('div', { class: 'menu-heading' }, 'Switch model'));
    models.forEach((m, j) => {
      const i = commands.length + j;
      list.append(
        h(
          'button',
          { class: `menu-item rich${i === index ? ' selected' : ''}`, on: { mousedown: (e: MouseEvent) => { e.preventDefault(); this.pickSkill(`model:${m.reference}`); } } },
          icon('sparkle', 17),
          h('span', {}, h('strong', {}, m.name), h('small', {}, m.providerLabel))
        )
      );
    });
    if (agents.length) list.append(h('div', { class: 'menu-heading' }, 'Chat with an agent'));
    agents.forEach((a, j) => {
      const i = commands.length + models.length + j;
      list.append(
        h(
          'button',
          { class: `menu-item rich${i === index ? ' selected' : ''}`, on: { mousedown: (e: MouseEvent) => { e.preventDefault(); this.pickSkill(`agent:${a.id}`); } } },
          h('span', { class: 'agent-avatar small' }, a.avatar || a.name.charAt(0)),
          h('span', {}, h('strong', {}, a.name), h('small', {}, a.description))
        )
      );
    });
    if (skills.length) list.append(h('div', { class: 'menu-heading' }, 'Skills'));
    skills.forEach((skill, j) => {
      const i = commands.length + models.length + agents.length + j;
      list.append(
        h(
          'button',
          { class: `menu-item rich${i === index ? ' selected' : ''}`, on: { mousedown: (e: MouseEvent) => { e.preventDefault(); this.pickSkill(skill.name); } } },
          icon('skill', 17),
          h('span', {}, h('strong', {}, `/${skill.name}`), h('small', {}, skill.description))
        )
      );
    });
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
    if (name.startsWith('cmd:')) {
      this.setText('');
      return void this.runCommand(name.slice(4));
    }
    if (name.startsWith('file:')) {
      const path = name.slice(5);
      const dir = this.workdir();
      const word = this.textarea.value.slice(0, this.textarea.selectionStart).match(/@[^\s@]*$/);
      if (word) this.replaceBeforeCaret(word[0].length, `@${path} `);
      if (dir) {
        void tauriInvoke<string>('chat_folder_read', { dir, path })
          .then((text) => {
            if (!this.attachments.some((a) => a.name === path)) this.attachments.push({ name: path, text });
            this.renderChips();
            this.renderBar();
          })
          .catch((e) => toast(String(e), 'error'));
      }
      return this.focus();
    }
    if (name.startsWith('lib:')) {
      this.dropHashWord();
      this.library = name.slice(4);
      this.renderChips();
      this.renderBar();
      return this.focus();
    }
    if (name.startsWith('emoji:')) {
      const emoji = emojiFor(name.slice(6));
      const word = this.textarea.value.slice(0, this.textarea.selectionStart).match(/:[a-z0-9_+-]*$/i);
      if (emoji && word) this.replaceBeforeCaret(word[0].length, `${emoji} `);
      return this.focus();
    }
    if (name.startsWith('page:') || name.startsWith('canvas:')) {
      const page = name.startsWith('page:');
      const id = name.slice(name.indexOf(':') + 1);
      this.dropHashWord();
      const load = page
        ? tauriInvoke<{ id: string; title: string; body: string }[]>('chat_knowledge_pages').then((pages) => {
            const p = pages.find((x) => x.id === id);
            if (!p) throw new Error('That page is gone.');
            return { name: `Page: ${p.title || 'Untitled'}`, text: `# ${p.title}\n\n${p.body}` };
          })
        : tauriInvoke<{ title: string; kind: string; versions: { content: string }[] }>('chat_get_canvas', { id }).then((c) => ({
            name: `Canvas: ${c.title || 'Untitled'}`,
            text: `${c.title} (${c.kind})\n\n${c.versions[c.versions.length - 1]?.content ?? ''}`,
          }));
      void load
        .then((a) => {
          this.attachments.push(a);
          this.renderChips();
          this.renderBar();
        })
        .catch((e) => toast(String(e instanceof Error ? e.message : e), 'error'));
      return this.focus();
    }
    if (name.startsWith('chat:')) {
      const chat = state.conversations.find((c) => c.id === name.slice(5));
      this.dropHashWord();
      if (chat) {
        void tauriInvoke<string>('chat_conversation_text', { id: chat.id })
          .then((text) => {
            this.attachments.push({ name: `Chat: ${chat.title || 'Untitled'}`, text });
            this.renderChips();
            this.renderBar();
          })
          .catch((e) => toast(String(e), 'error'));
      }
      return this.focus();
    }
    if (name.startsWith('agent:')) {
      const agent = state.agents.find((a) => a.id === name.slice(6));
      this.setText('');
      if (!agent) return;
      state.mode = 'agents';
      navigate({ kind: 'home', agentId: agent.id });
      toast(`New chat with ${agent.name}.`);
      return void setTimeout(() => this.focus());
    }
    if (name.startsWith('model:')) {
      const reference = name.slice(6);
      state.model = reference;
      void api.setDefaultModel(reference);
      this.setText('');
      this.renderBar();
      toast(`Using ${state.models.find((m) => m.reference === reference)?.name ?? reference}.`);
      return this.focus();
    }
    this.setText(`/${name} `);
    this.textarea.focus();
  }

  /** The app commands of the `/` menu. */
  private runCommand(name: string) {
    const id = this.conversationId;
    switch (name) {
      case 'new':
        return navigate({ kind: 'home' });
      case 'temporary':
        temporaryNext = true;
        toast('The next new chat is temporary: it is not saved to your history.');
        return navigate({ kind: 'home' });
      case 'fork':
        if (id) void api.forkConversation(id).then(async (branch) => { await refreshConversations(); navigate({ kind: 'chat', id: branch }); toast('Branched into a new chat.'); });
        return;
      case 'export':
        if (id) void api.exportConversation(id).then((saved) => saved && toast('Conversation exported.'));
        return;
      case 'context':
        if (id) void showContext(id);
        return;
      case 'settings':
        if (id) void openChatSettings(id);
        return;
      case 'model': {
        const btn = this.bar.querySelector<HTMLElement>('.model-btn');
        if (btn) openModelPicker(btn, () => this.renderBar());
        return;
      }
      case 'voice':
        return this.startVoice();
      case 'leaderboard':
        return void openLeaderboard();
      case 'clear':
        this.attachments = [];
        this.renderChips();
        this.renderBar();
        return this.focus();
    }
  }

  attach(conversationId: string | null, placeholder: string) {
    // Unsent text stays with its conversation (and the home composer), across restarts.
    if (conversationId !== this.conversationId) {
      saveDraft(this.conversationId, this.textarea.value);
      this.textarea.value = drafts()[conversationId ?? 'home'] ?? '';
      // Attachments stay with their conversation too (in memory: images can be large).
      draftAttachments.set(this.conversationId ?? 'home', this.attachments);
      this.attachments = draftAttachments.get(conversationId ?? 'home') ?? [];
      draftAttachments.delete(conversationId ?? 'home');
    }
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

  private expanded = false;

  private autosize() {
    this.textarea.style.height = 'auto';
    this.textarea.style.height = this.expanded ? '60vh' : `${Math.min(this.textarea.scrollHeight, 260)}px`;
  }

  private running(): boolean {
    return this.conversationId !== null && state.live.has(this.conversationId) && !state.live.get(this.conversationId)?.error;
  }

  async submit(queue = false) {
    if (this.running()) {
      // Steer: what the user types while a task runs joins it at its next step. Queued
      // messages wait instead, and go one by one once the task ends.
      const text = this.textarea.value.trim();
      if (!text || !this.conversationId) return;
      if (queue) {
        setQueue(this.conversationId, [...(queued.get(this.conversationId) ?? []), text]);
        this.textarea.value = '';
        this.autosize();
        this.renderChips();
        this.renderBar();
        return;
      }
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
    if (this.image) text = imageDirective(this.image, text);
    if (this.folder) text = `Work with the files in the folder \`${this.folder}\`.\n\n${text}`;
    if (this.library) text = `Search the "${this.library}" library (search_library) for this, and cite what you use.\n\n${text}`;
    if (this.workflow) text = workflowPrompt(this.workflow, text);
    // `/skill-name rest` asks for that skill explicitly.
    const typed = this.textarea.value.trim();
    const slash = text.match(/^[/@]([a-z0-9-]+)\s*([\s\S]*)$/);
    if (slash && state.skills.some((s) => s.name === slash[1])) {
      countSkillUse(slash[1]!);
      text = `Use the "${slash[1]}" skill.${slash[2] ? `\n\n${slash[2]}` : ''}`;
    }
    // `{{current_date}}` fills in; other `{{blanks}}` ask for a value first.
    const filled = await fillVariables(text);
    if (filled === null) return;
    text = filled;
    // Large files go to the Uploads library to be searched (Settings → Profile → Uploaded files).
    const searched: string[] = [];
    for (const file of this.attachments.filter((a) => !a.image)) {
      if (searchInstead(file.text.length)) {
        try {
          await tauriInvoke<string>('chat_upload_to_library', { name: file.name, text: file.text });
          searched.push(file.name);
          continue;
        } catch (e) {
          toast(`${file.name} goes in full: ${String(e)}`, 'error');
        }
      }
      text += `\n\n<attachment name="${file.name}">\n${file.text}\n</attachment>`;
    }
    if (searched.length) text += `\n\n(Attached and kept in the "Uploads" library: ${searched.map((n) => `"${n}"`).join(', ')}. Search it with search_library for what the request needs, or read a file whole with read_document.)`;
    const images = this.attachments.flatMap((a) => (a.image ? [a.image] : []));
    // Sent text is no longer a draft (sending from home moves to the new chat right away).
    this.clearSuggestion();
    window.clearTimeout(this.suggestionTimer);
    const unsent = this.textarea.value;
    const unsentFiles = this.attachments;
    this.textarea.value = '';
    this.attachments = [];
    saveDraft(this.conversationId, '');
    const ok = await send(text.trim(), this.conversationId, images);
    if (!ok && !this.textarea.value) {
      this.textarea.value = unsent;
      this.attachments = unsentFiles;
      this.renderChips();
    }
    if (ok) {
      if (typed) rememberPrompt(typed);
      const files = unsentFiles.filter((a) => a.file).map((a) => ({ name: a.name, text: a.text, image: a.image, usedAt: new Date().toISOString() }));
      if (files.length) void tauriInvoke('chat_remember_files', { files }).catch(() => undefined);
      this.historyIndex = -1;
      this.draft = '';
      this.textarea.value = '';
      this.attachments = [];
      this.quote = null;
      this.useCanvas = false;
      this.setImage(null);
      this.folder = null;
      this.library = null;
      this.workflow = null;
      this.autosize();
      this.renderChips();
      this.renderBar();
    }
  }

  /** Turns image mode on (with the image tool) or off. */
  setImage(next: { shape: 'square' | 'landscape' | 'portrait'; count: number; template: string | null } | null) {
    this.image = next;
    if (next) toolsOff.delete('generate_image');
    this.textarea.placeholder = next ? 'Describe the image you want…' : this.placeholder;
    this.renderChips();
    this.renderBar();
  }

  private imageOptions(anchor: HTMLElement) {
    if (!this.image) return;
    const img = this.image;
    let close = () => {};
    const pick = (change: Partial<typeof img>) => { this.setImage({ ...img, ...change }); close(); this.focus(); };
    const row = (label: string, on: boolean, run: () => void, ic?: string) => h('button', { class: `menu-item${on ? ' selected' : ''}`, on: { click: run } }, ic ? icon(ic, 15) : icon(on ? 'check' : 'dot', 14), h('span', { class: 'grow' }, label));
    close = popover(
      anchor,
      h(
        'div',
        { class: 'menu image-menu' },
        h('div', { class: 'menu-heading' }, 'Shape'),
        (['square', 'landscape', 'portrait'] as const).map((s) => row(s === 'square' ? 'Square 1:1' : s === 'landscape' ? 'Landscape 3:2' : 'Portrait 2:3', img.shape === s, () => pick({ shape: s }))),
        h('div', { class: 'menu-heading' }, 'How many'),
        h('div', { class: 'count-row' }, [1, 2, 3, 4].map((n) => h('button', { class: `count-btn${img.count === n ? ' active' : ''}`, on: { click: () => pick({ count: n }) } }, String(n)))),
        h('div', { class: 'menu-heading' }, 'Template'),
        row('None: just my description', !img.template, () => pick({ template: null })),
        IMAGE_TEMPLATES.map((t) => row(t.label, img.template === t.id, () => pick({ template: t.id }), t.icon))
      ),
      'above'
    );
  }

  setQuote(text: string) {
    this.quote = text;
    this.renderChips();
    this.focus();
  }

  renderChips() {
    this.chipsDrawnRunning = this.running();
    const extra: HTMLElement[] = [];
    // Messages waiting for the running task to end, with Send now (steer) and remove.
    const waiting = this.conversationId ? queued.get(this.conversationId) ?? [] : [];
    if (waiting.length && this.conversationId) {
      const id = this.conversationId;
      const paused = pausedQueues.has(id);
      const idle = !this.running();
      // The queue's own controls: pause or resume it, and send the next one when nothing runs
      // (after a restart, or while paused).
      extra.push(
        h(
          'div',
          { class: 'queued-head' },
          h('span', { class: 'queued-label' }, `${waiting.length} queued${paused ? ' · paused' : ''}`),
          h('span', { class: 'spacer' }),
          idle
            ? h('button', { class: 'link-btn', title: 'Send the first queued message now', on: { click: () => {
                // A reply started meanwhile: the queue resumes, so this goes as soon as it ends.
                if (this.running()) {
                  pausedQueues.delete(id);
                  saveQueues();
                  this.renderChips();
                  return void toast('A reply is running; the queue resumes and this goes next.');
                }
                const [next, ...rest] = waiting;
                setQueue(id, rest);
                this.renderChips();
                if (next) void send(next, id);
              } } }, 'Send next')
            : null,
          h('button', { class: 'link-btn', title: paused ? 'Send queued messages again as each turn ends' : 'Keep them waiting until you resume', on: { click: () => { if (paused) pausedQueues.delete(id); else pausedQueues.add(id); saveQueues(); this.renderChips(); } } }, paused ? 'Resume' : 'Pause'),
          h('button', { class: 'link-btn', on: { click: () => { setQueue(id, []); this.renderChips(); } } }, 'Clear')
        )
      );
    }
    waiting.forEach((text, i) => {
      const id = this.conversationId!;
      const drop = () => {
        const list = [...(queued.get(id) ?? [])];
        list.splice(i, 1);
        setQueue(id, list);
        this.renderChips();
      };
      const move = (by: -1 | 1) => {
        const list = [...(queued.get(id) ?? [])];
        const j = i + by;
        if (j < 0 || j >= list.length) return;
        [list[i], list[j]] = [list[j]!, list[i]!];
        setQueue(id, list);
        this.renderChips();
      };
      extra.push(
        h(
          'div',
          { class: 'queued-row' },
          icon('clock', 14),
          h('span', { class: 'queued-label' }, `${i + 1}`),
          h('span', { class: 'queued-text', title: text }, text),
          waiting.length > 1 ? iconButton({ icon: 'chevronUp', title: 'Move up', size: 'xs', class: 'chip-x', onClick: () => move(-1) }) : null,
          waiting.length > 1 ? iconButton({ icon: 'chevron', title: 'Move down', size: 'xs', class: 'chip-x', onClick: () => move(1) }) : null,
          this.running() ? h('button', { class: 'link-btn', title: 'Send it to the running task now', on: { click: () => { drop(); void api.steer(id, text); } } }, 'Send now') : null,
          h('button', { class: 'link-btn', title: 'Edit', on: { click: () => { drop(); this.setText(text); this.focus(); } } }, 'Edit'),
          iconButton({ icon: 'close', title: 'Remove from the queue', size: 'xs', class: 'chip-x', onClick: drop })
        )
      );
    });
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
    if (this.image) {
      const img = this.image;
      const label = ['Image', img.template ? IMAGE_TEMPLATES.find((t) => t.id === img.template)?.label : null, img.shape !== 'square' ? (img.shape === 'landscape' ? 'Landscape' : 'Portrait') : null, img.count > 1 ? `×${img.count}` : null].filter(Boolean).join(' · ');
      const c = chip({ icon: 'image', tone: 'accent', label, onRemove: () => { this.setImage(null); this.renderChips(); } });
      c.classList.add('chip-clickable');
      c.title = 'Shape, how many, and templates';
      c.addEventListener('click', (e) => { if (!(e.target as HTMLElement).closest('.chip-x')) this.imageOptions(c); });
      extra.push(c);
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
    input.addEventListener('change', () => {
      input.remove();
      void this.addFiles(Array.from(input.files ?? []));
    });
    input.click();
  }

  /** Attaches files: images as images, documents and workbooks as their text. */
  async addFiles(files: File[]) {
    {
      for (const file of files) {
        if (IMAGE_TYPES.includes(file.type)) {
          if (file.size > 8 * 1024 * 1024) {
            toast(`${file.name} is larger than 8 MB.`, 'error');
            continue;
          }
          this.attachments.push({ name: file.name, text: '', image: await readImage(file), file: true });
          continue;
        }
        // Videos: spread-out frames as images, and what is said as a transcript (ffmpeg).
        if (file.type.startsWith('video/') || /\.(mp4|mov|m4v|mkv|avi)$/i.test(file.name)) {
          try {
            if (file.size > 200 * 1024 * 1024) throw new Error(`${file.name} is larger than 200 MB.`);
            toast(`Reading ${file.name}…`);
            const bytes = new Uint8Array(await file.arrayBuffer());
            let binary = '';
            for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
            const v = await tauriInvoke<{ seconds: number; frames: { at: number; data: string }[]; transcript: string | null; note: string | null }>('chat_read_video', { name: file.name, data: btoa(binary) });
            const clock = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`;
            for (const f of v.frames) this.attachments.push({ name: `${file.name} · ${clock(f.at)}`, text: '', image: { mediaType: 'image/jpeg', data: f.data }, file: true });
            const about = `Video ${file.name}, ${clock(v.seconds)} long: ${v.frames.length} frames are attached as images, taken at ${v.frames.map((f) => clock(f.at)).join(', ')}.`;
            this.attachments.push({ name: `${file.name} (${v.transcript ? 'transcript' : 'about'})`, text: v.transcript ? `${about}\n\nWhat is said in it:\n${v.transcript}` : `${about} ${v.note ?? ''}`, file: true });
            if (v.note) toast(`${file.name}: ${v.note}`);
          } catch (e) {
            toast(`${file.name}: ${String(e)}`, 'error');
          }
          continue;
        }
        // Recordings are transcribed with the speech-to-text model, and the transcript attached.
        if (file.type.startsWith('audio/') || /\.(mp3|m4a|wav|ogg|oga|opus|flac|webm)$/i.test(file.name)) {
          try {
            if (file.size > 25 * 1024 * 1024) throw new Error(`${file.name} is larger than 25 MB.`);
            toast(`Transcribing ${file.name}…`);
            const bytes = new Uint8Array(await file.arrayBuffer());
            let binary = '';
            for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
            const text = await tauriInvoke<string>('chat_transcribe', { audio: btoa(binary), mime: file.type || 'audio/mpeg' });
            this.attachments.push({ name: `${file.name} (transcript)`, text, file: true });
          } catch (e) {
            toast(`${file.name}: ${String(e)}`, 'error');
          }
          continue;
        }
        // PDFs, Office documents and ZIP archives are read into text, as libraries read them.
        if (/\.(pdf|docx|pptx|odt|odp|zip)$/i.test(file.name)) {
          try {
            const bytes = new Uint8Array(await file.arrayBuffer());
            let binary = '';
            for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
            const text = await tauriInvoke<string>('chat_read_document', { name: file.name, data: btoa(binary) });
            this.attachments.push({ name: file.name, text: text.length > 400_000 ? `${text.slice(0, 400_000)}\n… (cut; add the file to a library to search all of it)` : text, file: true });
          } catch (e) {
            toast(`${file.name}: ${String(e)}`, 'error');
          }
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
            this.attachments.push({ name: file.name, text: text.length > 400_000 ? `${text.slice(0, 400_000)}\n… (cut; analyse the full file with run_code)` : text, file: true });
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
        this.attachments.push({ name: file.name, text, file: true });
      }
      this.renderChips();
      this.renderBar();
    }
  }

  /** Attaches a block of text (forwarded messages and the like) to the next message. */
  attachText(name: string, text: string) {
    this.attachments.push({ name, text });
    this.renderChips();
    this.renderBar();
  }

  /** Whether the queue's controls were drawn for a running reply. */
  private chipsDrawnRunning = false;

  renderBar() {
    const running = this.running();
    if (this.conversationId && queued.has(this.conversationId) && running !== this.chipsDrawnRunning) this.renderChips();
    this.textarea.placeholder = running ? 'Steer the task: Enter sends it now, Alt+Enter queues it for after' : this.image ? 'Describe the image you want…' : this.placeholder;
    const plus = h('button', { class: 'icon-btn', title: 'Add' }, icon('plus', 18));
    plus.addEventListener('click', () => this.openPlusMenu(plus));

    // The mode, as in Claude: who decides about changes, and whether to build or plan first.
    const approval = h('button', { class: `mode-pill mode-${state.approval}`, title: 'Mode (Shift+Tab to switch)' }, MODES[state.approval].name);
    approval.addEventListener('click', () => {
      const choose = (value: ApprovalMode) => {
        setMode(value);
        close();
        document.removeEventListener('keydown', onKey, true);
        this.renderBar();
        this.focus();
      };
      const onKey = (e: KeyboardEvent) => {
        const n = Number(e.key);
        if (n >= 1 && n <= MODE_ORDER.length) {
          e.preventDefault();
          e.stopPropagation();
          choose(MODE_ORDER[n - 1]!);
        }
      };
      document.addEventListener('keydown', onKey, true);
      const close = popover(
        approval,
        h(
          'div',
          { class: 'menu mode-menu' },
          h('div', { class: 'menu-heading' }, 'Mode'),
          MODE_ORDER.map((value, i) =>
            h(
              'button',
              { class: `menu-item rich mode-item${state.approval === value ? ' selected' : ''}`, on: { click: () => choose(value) } },
              h('span', { class: 'mode-text' }, h('strong', {}, MODES[value].name), h('small', {}, MODES[value].desc)),
              state.approval === value ? h('span', { class: 'mode-check' }, icon('check', 16)) : h('span', { class: 'mode-check' }),
              h('span', { class: 'mode-key' }, String(i + 1))
            )
          )
        ),
        'above'
      );
      // Closing the menu any other way also stops listening for its number keys.
      setTimeout(() => document.addEventListener('mousedown', () => document.removeEventListener('keydown', onKey, true), { once: true }));
    });

    const model = currentModel();
    const SPEED_NAMES: Record<Speed, string> = { fast: 'Fast', low: 'Think a little', think: 'Think', deep: 'Think harder' };
    const speed = trigger({ label: SPEED_NAMES[state.speed] ?? 'Fast', title: 'Thinking level' });
    speed.addEventListener('click', () => {
      const option = (value: Speed, name: string, desc: string, ic: string) =>
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
          option('fast', 'Fast', 'Quick answers, no extended reasoning', 'bolt'),
          ...(model && !model.reasoning
            ? [option('think', 'Think', 'This model does not reason; replies are unchanged', 'brain')]
            : [
                option('low', 'Think a little', 'A short reasoning pass', 'brain'),
                option('think', 'Think', 'Reasons longer for complex tasks', 'brain'),
                option('deep', 'Think harder', 'The most reasoning, for the hardest problems; slower and costlier', 'brain'),
              ])
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

    const discard = h('button', { class: 'icon-btn', title: 'Discard the recording' }, icon('close', 16));
    discard.addEventListener('click', () => {
      this.recorder?.cancel();
      this.recorder = null;
      this.renderBar();
    });
    const dictation = this.transcribing ? h('span', { class: 'dots' }) : this.recorder ? h('span', { class: 'dictation' }, discard, mic) : null;
    // Hands-free: talk, it answers aloud, and listens again.
    const voice = !running && !hasText && !this.recorder && !this.transcribing
      ? h('button', { class: 'icon-btn', title: 'Voice conversation: talk hands-free', on: { click: () => this.startVoice() } }, icon('wave', 17))
      : null;
    // A taller editor for long prompts (Open WebUI's expand).
    const expand = h('button', { class: `icon-btn${this.expanded ? ' active' : ''}`, title: this.expanded ? 'Shrink the editor' : 'Expand the editor' }, icon('expand', 16));
    expand.addEventListener('click', () => {
      this.expanded = !this.expanded;
      this.el.classList.toggle('expanded', this.expanded);
      this.autosize();
      this.renderBar();
      this.focus();
    });
    clear(this.bar, h('div', { class: 'bar-left' }, plus, approval), h('div', { class: 'bar-right' }, speed, modelBtn, expand, dictation, voice, action));
  }

  private recorder: Recording | null = null;
  private transcribing = false;
  private startingDictation = false;

  /** Starts a hands-free voice conversation in this chat. */
  startVoice() {
    if (audioSettings?.stt.engine === 'off') {
      toast('Voice input is off. Choose a speech-to-text engine in Settings → Audio.', 'error');
      return void openSettings('audio');
    }
    startVoiceConversation({
      send: (text) => send(text, this.conversationId),
      answer: async (id) => {
        try {
          const c = await api.conversation(id);
          const last = c.turns[c.turns.length - 1];
          return last?.role === 'assistant' ? textOf(last.parts) : null;
        } catch {
          return null;
        }
      },
    });
  }

  private async toggleDictation() {
    if (this.recorder) return void this.finishDictation();
    if (this.startingDictation) return;
    this.startingDictation = true;
    try {
      const engine = audioSettings?.stt.engine ?? 'provider';
      if (engine === 'off') {
        toast('Voice input is off. Choose a speech-to-text engine in Settings → Audio.', 'error');
        return void openSettings('audio');
      }
      if (engine === 'provider') {
        const media = await tauriInvoke<{ speechModel: string | null }>('chat_media_models');
        if (!media.speechModel) {
          toast('Voice input needs speech-to-text: add OpenAI or Groq, or choose another engine in Settings → Audio.', 'error');
          return void openSettings('audio');
        }
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
        if (row.classList.contains('active')) {
          clear(flyout, content);
          // Long lists near the bottom of the window open upward, so they stay on screen.
          const r = flyout.getBoundingClientRect();
          const over = r.bottom - (window.innerHeight - 8);
          if (over > 0) flyout.style.top = `${row.offsetTop - 6 - Math.min(over, r.top - 8)}px`;
        }
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
    const tools = async () => [
      heading('Tools the assistant can use'),
      ...TOOL_GROUPS.map((g) => {
        const on = h('input', { type: 'checkbox', class: 'switch', checked: !g.names.every((n) => toolsOff.has(n)) });
        on.addEventListener('change', () => {
          for (const n of g.names) (on.checked ? toolsOff.delete(n) : toolsOff.add(n));
          if (!on.checked && g.names.includes('generate_image')) this.setImage(null);
        });
        return h('label', { class: 'menu-item static', title: g.hint }, icon(g.icon, 16), h('span', { class: 'grow ellipsis' }, g.label), on);
      }),
      sep(),
      h('small', { class: 'menu-note muted' }, 'For the chats you send from here. Agents also have their own tools.'),
    ];
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
    // Another conversation as context, as Open WebUI's "Reference chats".
    const chats = async () => {
      const search = h('input', { class: 'input small', placeholder: 'Search chats' });
      const list = h('div', { class: 'flyout-list' });
      const render = () => {
        const q = search.value.toLowerCase();
        const shown = state.conversations.filter((c) => c.id !== this.conversationId && (!q || c.title.toLowerCase().includes(q))).slice(0, 12);
        clear(
          list,
          shown.length
            ? shown.map((c) =>
                entry('chat', c.title || 'Untitled', null, async () => {
                  try {
                    const text = await tauriInvoke<string>('chat_conversation_text', { id: c.id });
                    this.attachments.push({ name: `Chat: ${c.title || 'Untitled'}`, text });
                    this.renderChips();
                    this.renderBar();
                  } catch (e) {
                    toast(String(e), 'error');
                  }
                })
              )
            : empty('No chat matches.')
        );
      };
      search.addEventListener('input', render);
      render();
      setTimeout(() => search.focus());
      return [heading('Add a chat as context'), h('div', { class: 'flyout-head' }, search), list];
    };
    // Files attached lately (LibreChat's recent files): attached again from what was read then.
    const recent = async () => {
      type Recent = { name: string; text: string; image?: { mediaType: string; data: string }; usedAt: string };
      const files = await tauriInvoke<Recent[]>('chat_recent_files');
      const ago = (iso: string) => {
        const days = Math.floor((Date.now() - new Date(iso).getTime()) / 86_400_000);
        return days <= 0 ? 'today' : days === 1 ? 'yesterday' : `${days} days ago`;
      };
      return [
        heading('Attach again'),
        ...(files.length
          ? files.map((f) => entry(f.image ? 'eye' : 'file', f.name, ago(f.usedAt), () => {
              this.attachments.push({ name: f.name, text: f.text, image: f.image, file: true });
              this.renderChips();
              this.renderBar();
              this.focus();
            }))
          : [empty('Files you attach show here.')]),
        files.length ? sep() : null,
        files.length ? entry('trash', 'Clear the list', null, () => void tauriInvoke('chat_forget_recent_file', { index: null }).then(() => toast('Recent files cleared.'))) : null,
      ];
    };
    const workflows = async () => {
      const all = await workflowsApi.list();
      return [
        heading('Run a workflow'),
        ...all.map((w) => entry('workflow', w.name, `${liveWorkflow(w).steps.length} steps${w.production ? ' · production' : ''}`, () => { this.workflow = w; this.renderChips(); this.renderBar(); this.focus(); }, this.workflow?.id === w.id)),
        sep(),
        entry('plus', 'New workflow', null, () => editWorkflow(null, () => toast('Workflow saved.'))),
        entry('settings', 'Manage workflows', null, () => navigate({ kind: 'page', page: 'workflows' })),
      ];
    };

    menu.append(
      item('upload', 'Upload files', () => this.pickFiles()),
      sub('clock', 'Recent files', recent),
      item('sparkle', 'Improve prompt (Ctrl+E)', () => void this.extendPrompt()),
      sub('folder', 'Folders', folders),
      sub('skill', 'Skills', skills),
      sub('book', 'Knowledge', knowledge),
      sub('plug', 'Connectors', connectors),
      sub('folder', 'Projects', projects),
      sub('library', 'Libraries', libraries),
      sub('workflow', 'Workflows', workflows),
      sub('chat', 'Other chats', chats),
      item('file', 'Canvas', () => { this.useCanvas = true; this.renderChips(); this.focus(); }),
      item('image', 'Create an image', () => { this.setImage(this.image ?? { shape: 'square', count: 1, template: null }); this.focus(); }),
      sub('tool', 'Tools', tools),
      sep(),
      item('reset', 'Reset input', () => { this.setText(''); this.attachments = []; this.folder = null; this.library = null; this.workflow = null; this.quote = null; this.useCanvas = false; this.setImage(null); this.renderChips(); this.renderBar(); })
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
  const skip = () => answer('(The user skipped this question. Choose what fits best yourself, say what you chose, and go on without asking it again.)');
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
    h('div', { class: 'question-option free' }, h('span', { class: 'num' }, String(q.options.length + 1)), input),
    // Skip (Grok's "Ignorar"): the assistant decides by itself and goes on.
    h('div', { class: 'question-foot' }, h('small', { class: 'muted' }, 'Esc skips'), h('span', { class: 'spacer' }), h('button', { class: 'btn btn-ghost btn-sm', title: 'Let the assistant choose and go on', on: { click: skip } }, 'Skip'))
  );
  card.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') return void skip();
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
  const reason = request.reason?.kind === 'auto'
    ? h('p', { class: 'approval-risk auto' }, icon('sparkle', 14), `Auto checked this and wants your OK: ${request.reason.detail ?? 'it goes beyond what you asked'}.`)
    : request.reason?.kind === 'risky'
    ? h('p', { class: 'approval-risk' }, icon('shield', 14), `This asks even in the current mode: ${request.reason.detail ?? 'it is hard to undo'}.`)
    : request.reason
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
      h('button', { class: 'btn btn-secondary', on: { click: () => respond(false) } }, 'Decline'),
      h('button', { class: 'btn btn-secondary', on: { click: () => respond(true, true) } }, 'Always allow'),
      h('button', { class: 'btn btn-primary', on: { click: () => respond(true) } }, 'Continue')
    )
  );
}

// ---------------------------------------------------------------------------------------------
// Views

const SUGGESTIONS: { icon: string; text: string; page?: boolean; more?: boolean; image?: boolean }[] = [
  { icon: 'image', text: 'Create an image', image: true },
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
    // Code sessions can each get their own git worktree (Vibe's worktrees).
    const worktreeToggle = code && state.workdir
      ? h(
          'label',
          { class: 'worktree-toggle', title: 'The session works in a git worktree beside the repository, on a branch of its own, so the main checkout stays as it is' },
          (() => {
            const box = h('input', { type: 'checkbox', checked: worktreeNext() });
            box.addEventListener('change', () => setWorktreeNext(box.checked));
            return box;
          })(),
          h('span', {}, 'In a new worktree, on its own branch')
        )
      : null;
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
    // An agent's own conversation starters: one click asks it.
    const starters = agent?.starters?.length
      ? h(
          'div',
          { class: 'suggestions' },
          agent.starters.map((text) =>
            h('button', { class: 'suggestion', on: { click: () => { composer.setText(text); void composer.submit(); } } }, icon('sparkle', 17), text)
          )
        )
      : null;
    const suggestions = code || agent
      ? starters
      : h(
          'div',
          { class: 'suggestions' },
          SUGGESTIONS.filter((s) => showMoreSuggestions || !s.more).map((s) =>
            h(
              'button',
              {
                class: 'suggestion',
                on: { click: () => (s.page ? navigate({ kind: 'page', page: 'connectors' }) : s.image ? (composer.setImage({ shape: 'square', count: 1, template: null }), composer.focus()) : (composer.setText(s.text), composer.focus())) },
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
      h('div', { class: 'home-inner' }, agent ? h('span', { class: 'agent-avatar large' }, agent.avatar || agent.name.charAt(0)) : h('div', { class: 'brand-mark' }), title, h('div', { class: 'composer-wrap' }, banner, composer.el), worktreeToggle, suggestions, agent ? agentPanel(agent.id) : null)
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

/**
 * A hue for an avatar: emoji with a colour of their own get it (🍊 orange, 🌿 green…), anything
 * else a steady hue from its characters, so each agent keeps its colour.
 */
export function avatarHue(avatar: string): number {
  const known: [RegExp, number][] = [
    [/[🍎🍓🍒🌹❤️🔥🚒🐞🦀]/u, 4], [/[🍊🧡🦊🎃🏀🥕]/u, 28], [/[🍋🌻⭐💛🐝🌟☀️]/u, 48], [/[🌿🍀🌱💚🐸🥝🌲🌵]/u, 135],
    [/[🐳💧🌊💙🧊🦋🫐]/u, 205], [/[🍇🔮💜🦄☂️]/u, 275], [/[🌸🎀🩷🐷🌷]/u, 330],
  ];
  for (const [re, hue] of known) if (re.test(avatar)) return hue;
  let hash = 0;
  for (const ch of avatar) hash = (hash * 31 + ch.codePointAt(0)!) >>> 0;
  return hash % 360;
}

/** Whether the next Code session starts in a new git worktree. */
function worktreeNext(): boolean {
  try {
    return localStorage.getItem('lumo.worktree') === 'on';
  } catch {
    return false;
  }
}

function setWorktreeNext(on: boolean) {
  try {
    localStorage.setItem('lumo.worktree', on ? 'on' : 'off');
  } catch {
    /* per-device convenience only */
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

/** One prompt answered by several models side by side; the user keeps the answer they prefer. */
export class CompareView {
  readonly el = h('div', { class: 'chat compare' });
  readonly key: string;
  private grid = h('div', { class: 'compare-grid' });
  private top = h('div', {});
  private saved = new Map<string, Turn | null>();
  private blind = blindCompare();

  constructor(
    private prompt: string,
    private columns: { id: string; model: string }[]
  ) {
    this.key = `compare:${columns.map((c) => c.id).join(',')}`;
    // Blind: model names stay hidden until an answer is kept (an arena, as in Open WebUI).
    const blindToggle = h('button', { class: `btn btn-secondary${this.blind ? ' active' : ''}`, title: 'Hide which model wrote which answer until you choose' }, icon('eye', 15), this.blind ? 'Blind' : 'Names shown');
    blindToggle.addEventListener('click', () => {
      this.blind = !this.blind;
      setBlindCompare(this.blind);
      blindToggle.lastChild!.textContent = this.blind ? 'Blind' : 'Names shown';
      blindToggle.classList.toggle('active', this.blind);
      this.render();
    });
    const board = h('button', { class: 'btn btn-secondary' }, icon('list', 15), 'Leaderboard');
    board.addEventListener('click', () => void openLeaderboard());
    const discard = h('button', { class: 'btn btn-secondary' }, icon('trash', 15), 'Discard all');
    discard.addEventListener('click', async () => {
      await Promise.allSettled(this.columns.map((c) => api.deleteConversation(c.id)));
      navigate({ kind: 'home' });
    });
    this.el.append(
      h('header', { class: 'chat-header' }, h('span', { class: 'chat-title-static' }, `Comparing ${columns.length} models`), h('span', { class: 'spacer' }), blindToggle, board, discard),
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
    // The kept answer's model wins against the others, for the leaderboard.
    const winner = this.columns.find((c) => c.id === id)?.model;
    if (winner) {
      void tauriInvoke('chat_arena_record', { winner, losers: this.columns.filter((c) => c.id !== id).map((c) => c.model) }).catch(() => undefined);
      if (this.blind) toast(`You chose ${modelName(winner)}.`);
    }
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
          h('div', { class: 'compare-head' }, h('strong', {}, this.blind ? `Answer ${String.fromCharCode(65 + this.columns.indexOf(c))}` : modelName(c.model)), h('span', { class: 'spacer' }), keep),
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
  /** The prompt of the part being read, pinned at the top once it scrolls away (Vibe's sticky prompt). */
  private sticky = h('button', { class: 'sticky-prompt' });
  private stickyTarget: HTMLElement | null = null;
  private planPanel = h('aside', { class: 'plan-panel' });
  private sidePanel = h('aside', { class: 'side-panel' });
  /** Files changed in the session's folder: undefined until asked, null while asking, -1 without git. */
  private changeCount: number | null | undefined = undefined;
  private stickToBottom = true;

  constructor(private id: string) {
    this.key = `chat:${id}`;
    this.scroller.append(this.sticky, h('div', { class: 'chat-column' }, this.transcript, this.liveSlot));
    this.sticky.addEventListener('click', () => this.stickyTarget?.scrollIntoView({ behavior: 'smooth', block: 'start' }));
    this.scroller.addEventListener('scroll', () => this.updateSticky());
    // Ctrl+O folds or unfolds every activity group at once.
    this.el.addEventListener('keydown', (e) => {
      if (actionFor(e) === 'toggleSteps') {
        e.preventDefault();
        const groups = [...this.transcript.querySelectorAll<HTMLDetailsElement>('details.activity'), ...this.liveSlot.querySelectorAll<HTMLDetailsElement>('details.activity')];
        const open = groups.some((g) => !g.open);
        groups.forEach((g) => (g.open = open));
      }
    });
    this.scroller.addEventListener('scroll', () => {
      const distance = this.scroller.scrollHeight - this.scroller.scrollTop - this.scroller.clientHeight;
      this.stickToBottom = distance < 80;
      this.jump.classList.toggle('visible', distance > 300);
    });
    this.jump.addEventListener('click', () => this.scrollToBottom(true));
    enableAskAboutSelection(this.scroller, (quote) => composer.setQuote(quote));
    // Coding sessions read like Vibe Code: "Send a message".
    composer.attach(id, state.conversations.find((c) => c.id === id)?.workdir ? 'Send a message' : 'Type / for quick access');
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

  /** An agent's chat takes a colour from its avatar (OpenBot's bubble colours). */
  private tintForAgent() {
    const agent = this.conversation?.agentId ? state.agents.find((a) => a.id === this.conversation!.agentId) : undefined;
    this.el.classList.toggle('agent-tinted', Boolean(agent));
    if (agent) this.el.style.setProperty('--agent-hue', String(avatarHue(agent.avatar || agent.name)));
    else this.el.style.removeProperty('--agent-hue');
  }

  async refresh(scroll: boolean) {
    try {
      this.conversation = await api.conversation(this.id);
      // A turn running since before this window opened (or reloaded): what it streamed so far.
      if (this.conversation.running && !state.live.get(this.id)?.seenStart) {
        const events = await tauriInvoke<ChatEvent[]>('chat_live_events', { conversationId: this.id }).catch(() => [] as ChatEvent[]);
        // The replay holds everything so far, including what already arrived here: start over from it.
        if (events.length) {
          state.live.delete(this.id);
          for (const e of events) applyEvent(e);
        }
      }
      bookmarked.set(this.id, new Set(this.conversation.bookmarks ?? []));
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
        params: { temperature: null, maxTokens: null, instructions: '' },
        bookmarks: [],
        versions: [],
      };
    }
    const optimistic = pendingUser.get(this.id);
    if (optimistic && this.conversation.turns.length === 0) {
      this.conversation.turns.push({ role: 'user', parts: userParts(optimistic.text, optimistic.images), index: -1, time: new Date().toISOString(), rating: 0 });
    }
    this.tintForAgent();
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
      }),
      !running && c.turns[last]?.role === 'assistant' ? followUpRow(c.id) : null
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
    // The chat at a glance (Open WebUI's chat overview): every message you sent, where earlier
    // turns were summarized for the model, and a jump to each.
    const prompts = (c?.turns ?? []).filter((t) => t.role === 'user' && t.index >= 0);
    const outlineBtn = prompts.length > 2 ? h('button', { class: 'icon-btn', title: 'Outline of this chat' }, icon('list', 18)) : null;
    outlineBtn?.addEventListener('click', () => {
      const cut = c?.compactedBefore ?? null;
      const items: HTMLElement[] = [];
      let marked = false;
      prompts.forEach((t, i) => {
        if (cut !== null && !marked && t.index >= cut) {
          marked = true;
          items.push(h('div', { class: 'menu-note muted' }, '↑ summarized for the model'));
        }
        const text = textOf(t.parts).replace(/<attachment[\s\S]*?<\/attachment>/g, '').replace(/\s+/g, ' ').trim();
        items.push(h('button', { class: 'menu-item outline-item', on: { click: () => { close(); scrollToTurn(t.index); } } }, h('span', { class: 'outline-n' }, String(i + 1)), h('span', {}, text.length > 90 ? `${text.slice(0, 90)}…` : text || '(attachments)')));
      });
      const close = popover(outlineBtn, h('div', { class: 'menu outline-menu' }, h('div', { class: 'menu-heading' }, `${prompts.length} messages`), items));
    });
    // Bookmarked messages of this chat, to jump to.
    const marks = summary?.bookmarks ?? [];
    const marksBtn = marks.length ? h('button', { class: 'icon-btn', title: `Bookmarks (${marks.length})` }, icon('bookmark', 18), h('span', { class: 'badge-count' }, String(marks.length))) : null;
    marksBtn?.addEventListener('click', () => {
      const close = popover(
        marksBtn,
        h(
          'div',
          { class: 'menu bookmarks-menu' },
          h('div', { class: 'menu-heading' }, 'Bookmarks'),
          marks.map(([index, snippet]) =>
            h('button', { class: 'menu-item rich', on: { click: () => { close(); if (!scrollToTurn(index)) toast('That message is in another version of this chat.'); } } }, icon('bookmark', 15), h('span', {}, snippet))
          )
        )
      );
    });
    const exportBtn = h('button', { class: 'icon-btn', title: 'Share or export' }, icon('share', 18));
    let changesBtn: HTMLElement | null = null;
    let terminalBtn: HTMLElement | null = null;
    let filesBtn: HTMLElement | null = null;
    let debugBtn: HTMLElement | null = null;
    if (workdir) {
      if (this.changeCount === undefined) {
        this.changeCount = null;
        void loadChanges(this.id)
          .then((c) => (this.changeCount = c.git ? c.files.length : -1))
          .catch(() => (this.changeCount = -1))
          .then(() => this.renderHeader());
      }
      debugBtn = trigger({ icon: 'tool', label: 'Debug', title: 'The session\'s raw history and usage', chevron: false, onClick: () => openDebug(this.id) });
      filesBtn = trigger({ icon: 'folder', label: 'Files', title: 'Browse the files in this folder', chevron: false, onClick: () => openFiles(this.id, workdir.split(/[\\/]/).pop() ?? workdir) });
      if (this.changeCount !== -1) {
        terminalBtn = trigger({ icon: 'terminal', label: 'Terminal', title: 'Background commands in this folder', chevron: false, onClick: () => openTerminal(this.id, workdir.split(/[\\/]/).pop() ?? workdir) });
        changesBtn = trigger({ icon: 'file', label: 'Changes', title: 'Uncommitted changes in this folder', chevron: false, class: 'changes-btn', trailing: this.changeCount ? h('span', { class: 'count-badge' }, String(this.changeCount)) : null });
        changesBtn.addEventListener('click', () => openChanges(this.id, workdir.split(/[\\/]/).pop() ?? workdir));
      }
    }
    exportBtn.addEventListener('click', () => {
      const item = (ic: string, label: string, hint: string, run: () => void) =>
        h('button', { class: 'menu-item rich', on: { click: () => { close(); run(); } } }, icon(ic, 17), h('span', {}, h('strong', {}, label), h('small', {}, hint)));
      const close = popover(
        exportBtn,
        h(
          'div',
          { class: 'menu' },
          item('link', 'Share a link', 'Anyone on your network with the link can open it', () => {
            const shared = this.sharePage();
            if (shared) void import('./share').then((s) => s.shareLink(shared.title, shared.page));
          }),
          item('globe', 'Share as web page', 'One file anyone can open in a browser', () => void this.shareAsPage()),
          item('download', 'Export as Markdown', 'The text, for notes and documents', () => void api.exportConversation(this.id).then((saved) => saved && toast('Conversation exported.')))
        )
      );
    });
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
      h('div', { class: 'header-actions' }, filesBtn, terminalBtn, changesBtn, debugBtn, meter, outlineBtn, marksBtn, h('button', { class: 'icon-btn', title: 'Outputs: what this chat made', on: { click: () => void openOutputsPanel(this.id) } }, icon('outputs', 17)), pin, exportBtn)
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
    if (this.stickToBottom && followStreamOn()) this.scrollToBottom(false);
  }

  private updateSticky() {
    const top = this.scroller.getBoundingClientRect().top;
    const prompts = [...this.transcript.querySelectorAll<HTMLElement>('.turn-user')];
    // The last prompt that has scrolled fully out of view, while its answer is still on screen.
    const passed = prompts.filter((p) => p.getBoundingClientRect().bottom < top + 4);
    const current = passed[passed.length - 1] ?? null;
    const next = current ? prompts[prompts.indexOf(current) + 1] : undefined;
    const answerVisible = !next || next.getBoundingClientRect().top > top + 60;
    const show = Boolean(current && answerVisible);
    this.sticky.classList.toggle('visible', show);
    if (show && current !== this.stickyTarget) {
      this.stickyTarget = current;
      // The message itself: not its file chips or the Show more button.
      const text = (current!.querySelector('.bubble-text') ?? current!.querySelector('.bubble'))?.textContent?.trim() ?? '';
      clear(this.sticky, h('span', {}, text.length > 160 ? `${text.slice(0, 160)}…` : text));
    }
  }

  /**
   * The conversation as one self-contained web page: what was asked and answered, rendered,
   * with code, citations and sources, readable in light or dark, loading nothing from elsewhere.
   */
  /** The conversation as one self-contained page. */
  private sharePage(): { title: string; page: string } | null {
    const c = this.conversation;
    if (!c) return null;
    const title = this.header.querySelector('.crumb-title')?.textContent?.trim() || 'Conversation';
    const body = document.createElement('div');
    for (const turn of c.turns) {
      if (turn.role === 'user') {
        body.append(h('section', { class: 'you' }, h('div', { class: 'who' }, 'You'), h('div', { class: 'bubble' }, textOf(turn.parts))));
        continue;
      }
      const answer = h('div', { class: 'markdown' });
      for (const part of turn.parts) if (part.kind === 'text') answer.append(markdown(part.text));
      // Frames, buttons and hover-only bits do not travel; links and text do.
      answer.querySelectorAll('iframe, button, .code-head, .preview-head, script, .cite-icon, .site-icon').forEach((el) => el.remove());
      const sources = sourcesOf(turn.parts).filter((s) => s.cited);
      body.append(
        h(
          'section',
          { class: 'assistant' },
          h('div', { class: 'who' }, 'Assistant'),
          answer,
          sources.length ? h('div', { class: 'sources' }, h('strong', {}, 'Sources'), h('ol', {}, sources.map((s) => h('li', {}, h('a', { href: s.url }, s.title), ` · ${hostOf(s.url)}`)))) : null
        )
      );
    }
    const esc = (t: string) => t.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
    const page = `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'">
<title>${esc(title)}</title>
<style>
:root { --bg: #fff; --fg: #1d1d1f; --muted: #6b6b70; --card: #f3f3f5; --border: #e3e3e6; --accent: #6d4aff; color-scheme: light dark; }
@media (prefers-color-scheme: dark) { :root { --bg: #1e1e20; --fg: #ececee; --muted: #9a9aa0; --card: #2a2a2d; --border: #38383c; --accent: #9b84ff; } }
body { margin: 0; background: var(--bg); color: var(--fg); font: 16px/1.6 system-ui, -apple-system, "Segoe UI", sans-serif; }
main { max-width: 780px; margin: 0 auto; padding: 32px 16px 64px; }
h1.title { font-size: 1.5rem; margin: 0 0 4px; } .meta { color: var(--muted); font-size: .85rem; margin-bottom: 28px; }
section { margin: 0 0 28px; } .who { font-size: .75rem; font-weight: 700; text-transform: uppercase; letter-spacing: .04em; color: var(--muted); margin-bottom: 6px; }
.you .bubble { background: var(--card); border-radius: 14px; padding: 10px 14px; white-space: pre-wrap; }
a { color: var(--accent); } pre { background: var(--card); border: 1px solid var(--border); border-radius: 10px; padding: 12px; overflow-x: auto; font-size: .85rem; }
code { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; } :not(pre) > code { background: var(--card); border-radius: 4px; padding: 1px 4px; }
table { border-collapse: collapse; } th, td { border: 1px solid var(--border); padding: 4px 8px; } img { max-width: 100%; border-radius: 8px; }
.sources { font-size: .85rem; color: var(--muted); border-top: 1px solid var(--border); margin-top: 12px; padding-top: 8px; } .sources ol { margin: 4px 0 0; padding-left: 20px; }
details > summary { display: none; } .footer { color: var(--muted); font-size: .8rem; margin-top: 40px; }
</style></head><body><main>
<h1 class="title">${esc(title)}</h1><div class="meta">Shared from Lumo · ${new Date().toLocaleDateString(undefined, { year: 'numeric', month: 'long', day: 'numeric' })}</div>
${body.innerHTML}
<div class="footer">AI answers can contain mistakes. Check important information.</div>
</main></body></html>`;
    return { title, page };
  }

  private async shareAsPage() {
    const shared = this.sharePage();
    if (!shared) return;
    const { title, page } = shared;
    try {
      if (await tauriInvoke<boolean>('chat_save_shared_page', { title, html: page })) toast('Saved as a web page. Send the file to share the conversation.');
    } catch (e) {
      toast(String(e), 'error');
    }
  }

  /** The turn ended: the stored conversation replaces the live one. */
  async finished() {
    await this.refresh(false);
  }

  /** Redraws with something new under the last answer, keeping it in view if the bottom was. */
  showAddedBelow() {
    const distance = this.scroller.scrollHeight - this.scroller.scrollTop - this.scroller.clientHeight;
    this.renderTranscript();
    if (distance < 160) this.scrollToBottom(true);
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

/**
 * Select several messages (LobeHub's multi-select): tick them, then copy them, forward them to a
 * new chat or an agent as an attachment, or delete them with their replies. Esc leaves.
 */
function selectMessages(conversationId: string) {
  const view = viewCache.chat;
  const transcript = view?.el.querySelector<HTMLElement>('.transcript');
  if (!view || view.key !== `chat:${conversationId}` || !transcript || transcript.classList.contains('selecting')) return;
  transcript.classList.add('selecting');
  const turns = [...transcript.querySelectorAll<HTMLElement>('.turn[data-index]')].filter((t) => Number(t.dataset.index) >= 0);
  const selected = new Set<HTMLElement>();
  const count = h('span', { class: 'select-count' });
  const textOf = (turn: HTMLElement) => {
    const user = turn.classList.contains('turn-user');
    const body = user
      ? [turn.querySelector<HTMLElement>('.bubble-text')?.innerText ?? '', ...[...turn.querySelectorAll<HTMLElement>('.bubble-file summary span')].map((f) => `[Attached: ${f.innerText}]`)].filter(Boolean).join('\n')
      : [...turn.querySelectorAll<HTMLElement>('.markdown')].filter((m) => !m.closest('details, .activity') && !m.parentElement?.closest('.markdown')).map((m) => m.innerText).join('\n\n');
    return `${user ? 'You' : 'Assistant'}:\n${body.trim()}`;
  };
  const chosen = () => turns.filter((t) => selected.has(t));
  const transcriptText = () => chosen().map(textOf).join('\n\n---\n\n');
  const buttons: HTMLButtonElement[] = [];
  const update = () => {
    count.textContent = selected.size ? `${selected.size} selected` : 'Tick the messages';
    buttons.forEach((b) => (b.disabled = !selected.size));
  };
  const toggles: [HTMLElement, (e: MouseEvent) => void][] = [];
  const checks = turns.map((turn) => {
    const box = h('input', { type: 'checkbox', class: 'turn-check', title: 'Select' });
    box.addEventListener('change', () => {
      if (box.checked) selected.add(turn);
      else selected.delete(turn);
      turn.classList.toggle('selected', box.checked);
      update();
    });
    turn.prepend(box);
    // A click anywhere on the message ticks it, except on its links and buttons.
    const toggle = (e: MouseEvent) => {
      if (e.target === box || (e.target as HTMLElement).closest('a, button, input, summary, select, textarea')) return;
      box.checked = !box.checked;
      box.dispatchEvent(new Event('change'));
    };
    turn.addEventListener('click', toggle);
    toggles.push([turn, toggle]);
    return box;
  });
  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Escape') exit();
  };
  const exit = () => {
    checks.forEach((c) => c.remove());
    toggles.forEach(([t, f]) => t.removeEventListener('click', f));
    turns.forEach((t) => t.classList.remove('selected'));
    transcript.classList.remove('selecting');
    bar.remove();
    document.removeEventListener('keydown', onKey);
  };
  const action = (ic: string, label: string, run: (b: HTMLButtonElement) => void, danger = false) => {
    const b = h('button', { class: `btn btn-sm ${danger ? 'btn-danger' : 'btn-secondary'}` }, icon(ic, 14), label);
    b.addEventListener('click', () => run(b));
    buttons.push(b);
    return b;
  };
  const forwardTo = (agentId: string | null) => {
    const text = transcriptText();
    const n = selected.size;
    exit();
    if (agentId) state.mode = 'agents';
    navigate({ kind: 'home', agentId: agentId ?? undefined });
    setTimeout(() => {
      composer.attachText(`Forwarded messages (${n})`, text);
      composer.focus();
    }, 50);
    toast(agentId ? `Forwarded to ${state.agents.find((a) => a.id === agentId)?.name ?? 'the agent'}.` : 'Forwarded to a new chat.');
  };
  const all = h('button', { class: 'link-btn' }, 'All');
  all.addEventListener('click', () => {
    const on = selected.size < turns.length;
    checks.forEach((c) => {
      c.checked = on;
      c.dispatchEvent(new Event('change'));
    });
  });
  const bar = h(
    'div',
    { class: 'select-bar' },
    count,
    all,
    h('span', { class: 'spacer' }),
    action('copy', 'Copy', () => void navigator.clipboard.writeText(transcriptText()).then(() => toast('Copied.'))),
    action('share', 'Forward', (b) => {
      const close = popover(
        b,
        h(
          'div',
          { class: 'menu' },
          h('div', { class: 'menu-heading' }, 'Forward to'),
          h('button', { class: 'menu-item', on: { click: () => { close(); forwardTo(null); } } }, icon('plus', 16), h('span', {}, 'A new chat')),
          ...state.agents.map((a) => h('button', { class: 'menu-item', on: { click: () => { close(); forwardTo(a.id); } } }, icon('sparkle', 16), h('span', {}, a.name)))
        ),
        'above'
      );
    }),
    action('trash', 'Delete', async () => {
      const indices = chosen().map((t) => Number(t.dataset.index));
      if (!(await confirmDialog(`Delete ${indices.length === 1 ? 'this message' : `${indices.length} messages`}?`, 'Each message goes with the question or answer it belongs to, and any other versions of them.', 'Delete'))) return;
      try {
        const n = await tauriInvoke<number>('chat_delete_exchanges', { id: conversationId, indices });
        exit();
        await viewCache.chat?.refresh(false);
        toast(`Deleted ${n} ${n === 1 ? 'exchange' : 'exchanges'}.`);
      } catch (e) {
        toast(String(e), 'error');
      }
    }, true),
    h('button', { class: 'btn btn-sm btn-ghost', on: { click: () => exit() } }, 'Done')
  );
  document.addEventListener('keydown', onKey);
  view.el.append(bar);
  update();
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
      ...(state.agents.length
        ? [
            item('sparkle', 'Continue with an agent…', () => {
              // Hand the conversation to an agent (LobeHub's forward): a copy it carries on.
              const close = popover(
                anchor,
                h(
                  'div',
                  { class: 'menu' },
                  h('div', { class: 'menu-heading' }, 'Continue with'),
                  state.agents.map((a) =>
                    h('button', { class: 'menu-item', on: { click: async () => {
                      close();
                      try {
                        const next = await api.forkConversation(id, null, false, a.id);
                        await refreshConversations();
                        state.mode = 'agents';
                        navigate({ kind: 'chat', id: next });
                        toast(`${a.name} has the conversation now.`);
                      } catch (e) {
                        toast(String(e), 'error');
                      }
                    } } }, h('span', { class: 'agent-avatar small' }, a.avatar || a.name.charAt(0)), h('span', {}, a.name))
                  )
                )
              );
            }),
          ]
        : []),
      item('check', 'Select messages', () => {
        if (state.view.kind !== 'chat' || state.view.id !== id) navigate({ kind: 'chat', id });
        setTimeout(() => selectMessages(id), state.view.kind === 'chat' && state.view.id === id ? 0 : 400);
      }),
      item('download', 'Export as Markdown', () => void api.exportConversation(id).then((saved) => saved && toast('Conversation exported.'))),
      item('link', 'Copy link', () => void navigator.clipboard.writeText(`lumo://chat/${id}`).then(() => toast('Link copied. It opens this chat in Lumo on this computer.'))),
      ...(state.view.kind === 'chat' && state.view.id !== id ? [item('columns', 'Open beside this chat', () => openChatBeside(id))] : []),
      item('expand', 'Open in new window', () => void tauriInvoke('chat_open_chat_window', { id }).catch((e) => toast(String(e), 'error'))),
      item('download', 'Print or save as PDF', () => void printConversation(id, title)),
      item('image', 'Export as image', () => void exportImage(id)),
      item('code', 'Export as JSON', () => void tauriInvoke<boolean>('chat_export_conversation_json', { id }).then((saved) => saved && toast('Conversation exported.')).catch((e) => toast(String(e), 'error'))),
      item('settings', 'Chat settings…', () => void openChatSettings(id)),
      item('dot', 'Mark as unread', async () => {
        await api.updateConversation({ id, unread: true });
        await refreshConversations();
        if (state.view.kind === 'chat' && state.view.id === id) navigate({ kind: 'home' });
      }),
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

/** Messages queued behind a running task, by conversation; kept across restarts. */
const queued = new Map<string, string[]>();
/** Conversations whose queue waits instead of sending when a turn ends. */
const pausedQueues = new Set<string>();
const QUEUE_KEY = 'lumo.queue';

function loadQueues() {
  try {
    const saved = JSON.parse(localStorage.getItem(QUEUE_KEY) ?? '{}') as { queues?: Record<string, string[]>; paused?: string[] };
    for (const [id, list] of Object.entries(saved.queues ?? {})) if (Array.isArray(list) && list.length) queued.set(id, list.map(String));
    for (const id of saved.paused ?? []) pausedQueues.add(id);
  } catch {
    // Nothing saved, or unreadable: start empty.
  }
}
loadQueues();

function saveQueues() {
  try {
    localStorage.setItem(QUEUE_KEY, JSON.stringify({ queues: Object.fromEntries(queued), paused: [...pausedQueues].filter((id) => queued.has(id)) }));
  } catch {
    // Kept for this session only.
  }
}

function setQueue(id: string, list: string[]) {
  if (list.length) queued.set(id, list);
  else {
    queued.delete(id);
    pausedQueues.delete(id);
  }
  saveQueues();
}
/** Suggested next messages under the last answer, by conversation. */
const followUps = new Map<string, string[]>();

/** After a turn ends: the next queued message goes, or else follow-ups are suggested. */
export function turnEnded(id: string, ok: boolean) {
  voiceTurnEnded(id, ok);
  const list = queued.get(id);
  if (ok && list?.length && !pausedQueues.has(id)) {
    const [next, ...rest] = list;
    setQueue(id, rest);
    composer.renderChips();
    void send(next!, id);
    return;
  }
  if (list?.length) {
    composer.renderChips();
    return;
  }
  if (ok) void suggestFollowUps(id);
}

async function suggestFollowUps(id: string) {
  const model = state.model;
  if (!followUpsOn() || !model || temporaryChats.has(id)) return;
  try {
    const list = await api.followUps(id, model);
    // Something was sent meanwhile: these no longer fit.
    if (state.live.has(id)) return;
    followUps.set(id, list);
    if (viewCache.chat?.key === `chat:${id}`) viewCache.chat.showAddedBelow();
  } catch {
    // Suggestions are optional; a failed request shows none.
  }
}

function followUpRow(id: string): HTMLElement | null {
  const list = followUps.get(id);
  if (!list?.length) return null;
  return h(
    'div',
    { class: 'follow-ups' },
    h('span', { class: 'follow-ups-label' }, 'Follow-ups'),
    list.map((text) => h('button', { class: 'follow-up', on: { click: () => { followUps.delete(id); void send(text, id); } } }, icon('forward', 14), h('span', {}, text)))
  );
}

/** One conversation's model settings: temperature, reply length and its own instructions. */
async function openChatSettings(id: string) {
  let current: ChatParams;
  try {
    current = (await api.conversation(id)).params;
  } catch {
    current = { temperature: null, maxTokens: null, instructions: '' };
  }
  await modal((close) => {
    const temperature = h('input', { class: 'input', type: 'number', min: '0', max: '2', step: '0.1', placeholder: 'Model default', value: current.temperature ?? '' });
    const maxTokens = h('input', { class: 'input', type: 'number', min: '16', step: '1', placeholder: 'Model default', value: current.maxTokens ?? '' });
    const topP = h('input', { class: 'input', type: 'number', min: '0.05', max: '1', step: '0.05', placeholder: 'Model default', value: current.topP ?? '' });
    const instructions = h('textarea', { class: 'input', rows: 5, placeholder: 'For example: answer in Portuguese; keep code in TypeScript.' }, current.instructions);
    const number = (el: HTMLInputElement) => (el.value.trim() === '' || Number.isNaN(Number(el.value)) ? null : Number(el.value));
    const save = async () => {
      try {
        await api.setParams(id, { temperature: number(temperature), maxTokens: number(maxTokens), topP: number(topP), instructions: instructions.value });
        close();
        toast('Saved. The next reply uses these settings.');
      } catch (e) {
        toast(String(e), 'error');
      }
    };
    return h(
      'div',
      { class: 'dialog-small wide' },
      h('h2', {}, 'Chat settings'),
      h('p', { class: 'muted' }, 'For this conversation only. Empty fields follow your defaults (Settings → General), then the model.'),
      h('label', { class: 'field' }, h('span', { class: 'field-label' }, 'Temperature'), temperature, h('span', { class: 'field-hint' }, '0 is focused and repeatable, 1 is balanced, up to 2 is inventive.')),
      h('label', { class: 'field' }, h('span', { class: 'field-label' }, 'Longest reply (tokens)'), maxTokens),
      h('label', { class: 'field' }, h('span', { class: 'field-label' }, 'Top-p'), topP, h('span', { class: 'field-hint' }, 'Only the likeliest words adding up to this share are considered; lower is more predictable.')),
      h('label', { class: 'field' }, h('span', { class: 'field-label' }, 'Instructions for this chat'), instructions, h('span', { class: 'field-hint' }, 'Added to your standing instructions.')),
      h(
        'div',
        { class: 'dialog-actions' },
        h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'),
        h('button', { class: 'btn btn-primary', on: { click: () => void save() } }, 'Save')
      )
    );
  });
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

// ---------------------------------------------------------------------------------------------
// What the composer learns from use, kept on this device: earlier prompts and skill counts.

const HISTORY_KEY = 'lumo.promptHistory';
const SKILL_USE_KEY = 'lumo.skillUses';

function readStored<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : fallback;
  } catch {
    return fallback;
  }
}

function writeStored(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Storage can be unavailable; history is a convenience.
  }
}

/** Earlier prompts, newest first. */
function promptHistory(): string[] {
  return readStored<string[]>(HISTORY_KEY, []);
}

function rememberPrompt(text: string) {
  const history = promptHistory().filter((p) => p !== text);
  history.unshift(text);
  writeStored(HISTORY_KEY, history.slice(0, 300));
}

function skillUses(): Record<string, number> {
  return readStored<Record<string, number>>(SKILL_USE_KEY, {});
}

function countSkillUse(name: string) {
  const uses = skillUses();
  uses[name] = (uses[name] ?? 0) + 1;
  writeStored(SKILL_USE_KEY, uses);
}

/** A sub-agent's transcript: each tool it called, with what came back, then its report. */
async function showWorkerSteps(callId: string, index: number, title: string) {
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

// The live timers tick between events, so "Working · 12s" keeps counting while a step runs.
setInterval(() => {
  document.querySelectorAll<HTMLElement>('.live-timer[data-start]').forEach((el) => {
    const start = Number(el.dataset.start);
    if (start) el.textContent = ` · ${Math.max(0, Math.round((Date.now() - start) / 1000))}s`;
  });
}, 1000);

/** The Sources panel beside the conversation: each page with its site, title and what it says. */
/** Asks which library, then saves the page's text there. */
async function saveToLibrary(anchor: HTMLElement, url: string) {
  const libraries = await tauriInvoke<{ id: string; name: string }[]>('chat_list_libraries').catch(() => []);
  const save = async (id: string, name: string) => {
    close();
    toast(`Saving to ${name}…`);
    try {
      await tauriInvoke('chat_add_web_page', { libraryId: id, url });
      toast(`Saved to ${name}. Library searches can now find and cite it.`);
    } catch (e) {
      toast(String(e), 'error');
    }
  };
  const close = popover(
    anchor,
    h(
      'div',
      { class: 'menu' },
      h('div', { class: 'menu-heading' }, 'Save page to'),
      libraries.length ? libraries.map((l) => h('button', { class: 'menu-item', on: { click: () => void save(l.id, l.name) } }, icon('library', 15), h('span', {}, l.name))) : h('p', { class: 'muted menu-note' }, 'No libraries yet: make one under Libraries.')
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
  const swap = iconButton({ icon: 'columns', title: 'Swap: this one in front, the current one beside', size: 'sm', onClick: () => {
    closePanel();
    navigate({ kind: 'chat', id });
    setTimeout(() => openChatBeside(current), 500);
  } });
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
                  ? h('div', { class: 'beside-user' }, t.parts.filter((p) => p.kind === 'text').map((p) => (p as { text: string }).text.replace(/<attachment[\s\S]*$/, '').trim()).join('\n'))
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

function openSourcesPanel(sources: Source[]) {
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
function numberLibraryCitations(el: HTMLElement, parts: Part[]) {
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

/** Unsent attachments by conversation, for this session. */
const draftAttachments = new Map<string, Attachment[]>();

/** Unsent text by conversation (`home` for a new chat), kept on this device. */
function drafts(): Record<string, string> {
  try {
    return JSON.parse(localStorage.getItem('lumo.drafts') ?? '{}') as Record<string, string>;
  } catch {
    return {};
  }
}

function saveDraft(conversationId: string | null, text: string) {
  try {
    const all = drafts();
    const key = conversationId ?? 'home';
    if (text.trim()) all[key] = text;
    else delete all[key];
    // A few dozen drafts at most; the oldest go first.
    const keys = Object.keys(all);
    for (const k of keys.slice(0, Math.max(0, keys.length - 50))) delete all[k];
    localStorage.setItem('lumo.drafts', JSON.stringify(all));
  } catch {
    // Drafts are a convenience.
  }
}

/** The app's own `/` commands (Open WebUI's built-in slash commands). */
const SLASH_COMMANDS: { name: string; desc: string; icon: string; inChat?: boolean }[] = [
  { name: 'new', desc: 'Start a new chat', icon: 'plus' },
  { name: 'temporary', desc: 'Start a chat that is not saved', icon: 'chatDashed' },
  { name: 'model', desc: 'Choose the model', icon: 'sparkle' },
  { name: 'voice', desc: 'Talk hands-free', icon: 'wave' },
  { name: 'fork', desc: 'Branch this chat into a new one', icon: 'branch', inChat: true },
  { name: 'context', desc: 'See what fills the context window', icon: 'book', inChat: true },
  { name: 'settings', desc: 'Model settings for this chat', icon: 'settings', inChat: true },
  { name: 'export', desc: 'Export this chat as Markdown', icon: 'download', inChat: true },
  { name: 'leaderboard', desc: 'Your models ranked by your comparisons', icon: 'list' },
  { name: 'clear', desc: 'Remove the attachments from the composer', icon: 'close' },
];

function blindCompare(): boolean {
  try {
    return localStorage.getItem('lumo.blindCompare') === 'on';
  } catch {
    return false;
  }
}

function setBlindCompare(on: boolean) {
  try {
    localStorage.setItem('lumo.blindCompare', on ? 'on' : 'off');
  } catch {
    // A convenience only.
  }
}

/** Your models ranked by the comparisons you decided (Elo). */
export async function openLeaderboard() {
  const board = await tauriInvoke<{ model: string; rating: number; wins: number; losses: number }[]>('chat_arena_board').catch(() => []);
  await modal((close) => {
    const reset = h('button', { class: 'btn btn-secondary', on: { click: async () => { if (await confirmDialog('Reset the leaderboard?', 'Every recorded comparison is forgotten.', 'Reset')) { await tauriInvoke('chat_arena_reset'); close(); toast('Leaderboard reset.'); } } } }, 'Reset');
    return h(
      'div',
      { class: 'dialog-small wide' },
      h('h2', {}, 'Model leaderboard'),
      h('p', { class: 'muted' }, 'From your own comparisons: the answer you keep wins against the others. Ratings are Elo, starting at 1000.'),
      board.length
        ? h(
            'table',
            { class: 'leaderboard' },
            h('thead', {}, h('tr', {}, h('th', {}, '#'), h('th', {}, 'Model'), h('th', {}, 'Rating'), h('th', {}, 'Won'), h('th', {}, 'Lost'))),
            h('tbody', {}, board.map((s, i) => h('tr', {}, h('td', {}, String(i + 1)), h('td', {}, modelName(s.model)), h('td', {}, String(s.rating)), h('td', {}, String(s.wins)), h('td', {}, String(s.losses)))))
          )
        : h('p', { class: 'muted' }, 'No comparisons decided yet. Compare 2–3 models from a new chat and keep the best answer.'),
      h('div', { class: 'dialog-actions' }, board.length ? reset : null, h('span', { class: 'spacer' }), h('button', { class: 'btn btn-primary', on: { click: close } }, 'Close'))
    );
  });
}
