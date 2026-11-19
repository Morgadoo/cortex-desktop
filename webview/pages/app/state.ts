/** The page's state, and the live turns Rust streams into it. */
import type {
  ApprovalMode,
  ApprovalRequest,
  ChatEvent,
  Agent,
  ConversationSummary,
  Instructions,
  UsageTotals,
  Skill,
  Model,
  Part,
  Project,
  Speed,
} from './api';

export type Page = 'connectors' | 'skills' | 'scheduled' | 'apps' | 'agents' | 'knowledge' | 'libraries' | 'workflows' | 'chats' | 'resources';

export type View =
  | { kind: 'home'; projectId?: string | null; agentId?: string | null }
  | { kind: 'agent'; id: string }
  | { kind: 'chat'; id: string }
  | { kind: 'compare'; prompt: string; columns: { id: string; model: string }[] }
  | { kind: 'page'; page: Page };

export type Mode = 'work' | 'code' | 'agents';

export interface SubagentStatus {
  title: string;
  status: string;
  activity: string;
}

/** A reply as it streams in, before it is read back from disk. */
export interface LiveTurn {
  parts: Part[];
  /** The tool the model is writing a call to. */
  pending: string | null;
  question: { callId: string; question: string; options: string[] } | null;
  error: string | null;
  startedAt: number;
  /** The model is summarizing earlier turns to make room. */
  compacting: boolean;
  /** When each part began and ended, by index, for step durations. */
  starts: number[];
  ends: number[];
  /** Guidance the user sent while the turn ran, shown where it joined. */
  steers: string[];
  /** This page saw the turn start, so it holds all of it (not only what came after it opened). */
  seenStart?: boolean;
}

function fresh(): LiveTurn {
  return { parts: [], pending: null, question: null, error: null, startedAt: Date.now(), compacting: false, starts: [], ends: [], steers: [] };
}

/** Starts a part, ending the one before it unless it already ended (a tool ends on its result). */
function begin(turn: LiveTurn, part: Part) {
  const now = Date.now();
  const previous = turn.parts.length - 1;
  if (previous >= 0 && turn.ends[previous] === undefined) turn.ends[previous] = now;
  turn.parts.push(part);
  turn.starts.push(now);
}

/** Closes the last part when the turn ends. */
export function endSteps(turn: LiveTurn) {
  const last = turn.parts.length - 1;
  if (last >= 0 && turn.ends[last] === undefined) turn.ends[last] = Date.now();
}

export const state = {
  view: { kind: 'home' } as View,
  mode: 'work' as Mode,
  conversations: [] as ConversationSummary[],
  projects: [] as Project[],
  models: [] as Model[],
  model: null as string | null,
  speed: 'fast' as Speed,
  // Auto by default: the assistant decides when to ask (remembered per device).
  approval: storedMode(),
  instructions: { tone: '', custom: '', name: '' } as Instructions,
  live: new Map<string, LiveTurn>(),
  approvals: [] as ApprovalRequest[],
  skills: [] as Skill[],
  agents: [] as Agent[],
  /** Sub-agent progress by the `delegate` call that started them. */
  subagents: new Map<string, SubagentStatus[]>(),
  /** The latest usage of each conversation, and its model's window. */
  usage: new Map<string, { usage: UsageTotals; window: number }>(),
  sidebarCollapsed: false,
  contextOpen: false,
  /** Code mode: the folder new sessions work in. */
  workdir: null as string | null,
  history: [] as View[],
  future: [] as View[],
};

type Listener = () => void;
const listeners = new Set<Listener>();

export function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Everything listening re-renders on the next frame, once, however many changes came in. */
let scheduled = false;
export function changed() {
  if (scheduled) return;
  scheduled = true;
  requestAnimationFrame(() => {
    scheduled = false;
    for (const listener of listeners) listener();
  });
}

export function navigate(view: View, record = true) {
  if (record) {
    state.history.push(state.view);
    state.future = [];
  }
  state.view = view;
  changed();
}

export function back() {
  const previous = state.history.pop();
  if (!previous) return;
  state.future.push(state.view);
  state.view = previous;
  changed();
}

export function forward() {
  const next = state.future.pop();
  if (!next) return;
  state.history.push(state.view);
  state.view = next;
  changed();
}

export function currentModel(): Model | undefined {
  return state.models.find((m) => m.reference === state.model);
}

/**
 * Folds one streamed event into the live turn of its conversation. Returns true when the turn
 * ended, so the caller reloads the stored conversation.
 */
export function applyEvent(event: ChatEvent): boolean {
  const id = event.conversationId;
  let live = state.live.get(id);
  const ensure = (): LiveTurn => {
    if (!live) {
      live = fresh();
      state.live.set(id, live);
    }
    return live;
  };
  switch (event.type) {
    case 'started':
      state.live.set(id, { ...fresh(), seenStart: true });
      return false;
    case 'steered':
      ensure().steers.push(event.text);
      return false;
    case 'notice':
      // A note after the reply finished (a skill learned from it): the saved chat has it, so
      // reload instead of reopening a live turn.
      if (!live) return true;
      live.parts.push({ kind: 'notice', text: event.text });
      return false;
    case 'text': {
      const turn = ensure();
      const last = turn.parts[turn.parts.length - 1];
      if (last?.kind === 'text') last.text += event.text;
      else begin(turn, { kind: 'text', text: event.text });
      turn.pending = null;
      return false;
    }
    case 'reasoning': {
      const turn = ensure();
      const last = turn.parts[turn.parts.length - 1];
      if (last?.kind === 'reasoning') last.text += event.text;
      else begin(turn, { kind: 'reasoning', text: event.text });
      return false;
    }
    case 'toolPending':
      ensure().pending = event.name;
      return false;
    case 'toolStart': {
      const turn = ensure();
      turn.pending = null;
      begin(turn, {
        kind: 'tool',
        callId: event.callId,
        name: event.name,
        arguments: event.arguments,
        output: null,
        isError: false,
      });
      return false;
    }
    case 'toolEnd': {
      const turn = ensure();
      const index = turn.parts.findIndex((p) => p.kind === 'tool' && p.callId === event.callId);
      const part = turn.parts[index];
      if (part?.kind === 'tool') {
        turn.ends[index] = Date.now();
        part.output = event.output;
        part.isError = event.isError;
      }
      if (turn.question?.callId === event.callId) turn.question = null;
      return false;
    }
    case 'question':
      ensure().question = { callId: event.callId, question: event.question, options: event.options };
      return false;
    case 'title': {
      const summary = state.conversations.find((c) => c.id === id);
      if (summary) summary.title = event.title;
      return false;
    }
    case 'subagent': {
      const list = state.subagents.get(event.callId) ?? [];
      list[event.index] = { title: event.title, status: event.status, activity: event.activity };
      state.subagents.set(event.callId, list);
      return false;
    }
    case 'usage':
      state.usage.set(id, { usage: event.usage, window: event.contextWindow });
      return false;
    case 'compacting':
      ensure().compacting = true;
      return false;
    case 'compacted':
      ensure().compacting = false;
      return false;
    case 'error':
      ensure().error = event.message;
      endSteps(ensure());
      return true;
    case 'done':
      if (live) endSteps(live);
      return true;
  }
}

function storedMode(): ApprovalMode {
  try {
    const saved = localStorage.getItem('cortex.mode');
    if (saved === 'ask' || saved === 'accept_edits' || saved === 'plan' || saved === 'auto') return saved;
  } catch {
    // Storage unavailable: the default.
  }
  return 'auto';
}

/** Whether new chats start temporary (not saved to history), as Open WebUI allows. */
export function temporaryByDefault(): boolean {
  try {
    return localStorage.getItem('cortex.temporaryDefault') === 'on';
  } catch {
    return false;
  }
}

export function setTemporaryByDefault(on: boolean) {
  try {
    localStorage.setItem('cortex.temporaryDefault', on ? 'on' : 'off');
  } catch {
    // Remembering is a convenience.
  }
}

/** Whether three follow-up questions are suggested after each answer (on unless turned off). */
export function followUpsOn(): boolean {
  try {
    return localStorage.getItem('cortex.followUps') !== 'off';
  } catch {
    return true;
  }
}

/** Suggest how the message being typed continues (off unless turned on: each pause is a request). */
export function autocompleteOn(): boolean {
  try {
    return localStorage.getItem('cortex.autocomplete') === 'on';
  } catch {
    return false;
  }
}

export function setAutocomplete(on: boolean) {
  try {
    localStorage.setItem('cortex.autocomplete', on ? 'on' : 'off');
  } catch {
    /* per-device convenience only */
  }
}

/** Follow an answer down the page as it streams (on unless turned off). */
export function followStreamOn(): boolean {
  try {
    return localStorage.getItem('cortex.followStream') !== 'off';
  } catch {
    return true;
  }
}

export function setFollowStream(on: boolean) {
  try {
    localStorage.setItem('cortex.followStream', on ? 'on' : 'off');
  } catch {
    // Remembering is a convenience.
  }
}

/** A short chime with the "reply ready" notification (off unless turned on). */
export function finishSoundOn(): boolean {
  try {
    return localStorage.getItem('cortex.finishSound') === 'on';
  } catch {
    return false;
  }
}

export function setFinishSound(on: boolean) {
  try {
    localStorage.setItem('cortex.finishSound', on ? 'on' : 'off');
  } catch {
    // Remembering is a convenience.
  }
}

/** Two soft notes, made on the spot (no sound file). */
export function playChime() {
  try {
    const ctx = new AudioContext();
    [660, 880].forEach((freq, i) => {
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.frequency.value = freq;
      const t = ctx.currentTime + i * 0.14;
      gain.gain.setValueAtTime(0.0001, t);
      gain.gain.exponentialRampToValueAtTime(0.18, t + 0.02);
      gain.gain.exponentialRampToValueAtTime(0.0001, t + 0.35);
      osc.connect(gain).connect(ctx.destination);
      osc.start(t);
      osc.stop(t + 0.4);
    });
    setTimeout(() => void ctx.close(), 1000);
  } catch {
    // No audio output: the notification is enough.
  }
}

/** Notify when a reply finishes while the window is in the background (on unless turned off). */
export function finishNotificationsOn(): boolean {
  try {
    return localStorage.getItem('cortex.notifyFinished') !== 'off';
  } catch {
    return true;
  }
}

export function setFinishNotifications(on: boolean) {
  try {
    localStorage.setItem('cortex.notifyFinished', on ? 'on' : 'off');
  } catch {
    // Remembering is a convenience.
  }
}

export function setFollowUps(on: boolean) {
  try {
    localStorage.setItem('cortex.followUps', on ? 'on' : 'off');
  } catch {
    // Remembering is a convenience.
  }
}

export function setMode(mode: ApprovalMode) {
  state.approval = mode;
  try {
    localStorage.setItem('cortex.mode', mode);
  } catch {
    // Remembering is a convenience.
  }
}
