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

export type Page = 'connectors' | 'skills' | 'scheduled' | 'apps' | 'agents' | 'knowledge' | 'libraries';

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
}

function fresh(): LiveTurn {
  return { parts: [], pending: null, question: null, error: null, startedAt: Date.now(), compacting: false, starts: [], ends: [] };
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
  approval: 'ask' as ApprovalMode,
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
      state.live.set(id, fresh());
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
