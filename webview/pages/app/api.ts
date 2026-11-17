/**
 * Typed wrappers over the chat commands in `src-tauri/src/chat/commands.rs`, and the events Rust
 * pushes back. The shapes mirror the Rust structs' camelCase serialization.
 */
import { tauriInvoke } from '../../shared/tauri';

export interface ProviderSettings {
  enabled: boolean;
  hasKey: boolean;
  baseUrl: string | null;
  customModels: string[];
}

export interface Provider {
  vendor: string;
  label: string;
  keyUrl: string;
  local: boolean;
  requiresKey: boolean;
  defaultBaseUrl: string;
  envKey: string | null;
  settings: ProviderSettings;
  ready: boolean;
}

export interface Model {
  reference: string;
  vendor: string;
  providerLabel: string;
  id: string;
  name: string;
  reasoning: boolean;
  tools: boolean;
  contextWindow: number | null;
}

export interface ConversationSummary {
  id: string;
  title: string;
  projectId: string | null;
  workdir: string | null;
  agentId: string | null;
  taskId: string | null;
  unread: boolean;
  pinned: boolean;
  updatedAt: string;
}

export type Part =
  | { kind: 'text'; text: string }
  | { kind: 'reasoning'; text: string }
  | { kind: 'image'; mediaType: string; data: string }
  | { kind: 'tool'; callId: string; name: string; arguments: Record<string, unknown>; output: string | null; isError: boolean };

export interface Turn {
  role: 'user' | 'assistant';
  parts: Part[];
  index: number;
  time: string | null;
  rating: number;
}

export interface UsageTotals {
  inputTokens: number;
  outputTokens: number;
  cost: number;
  contextTokens: number;
}

export interface Conversation {
  id: string;
  usage: UsageTotals;
  compactedBefore: number | null;
  title: string;
  projectId: string | null;
  workdir: string | null;
  agentId: string | null;
  model: string | null;
  pinned: boolean;
  turns: Turn[];
  running: boolean;
}

export interface Project {
  id: string;
  name: string;
  instructions: string;
  tone: string;
  icon: string;
  libraryId: string | null;
  shareChats: boolean;
}

export interface Instructions {
  tone: string;
  custom: string;
  name: string;
}

export interface Memory {
  id: string;
  text: string;
  createdAt: string;
  source: string;
}

export interface Memories {
  enabled: boolean;
  items: Memory[];
}

export interface Skill {
  name: string;
  description: string;
  body: string;
  origin: 'user' | 'builtin';
  enabled: boolean;
}

export interface Agent {
  id: string;
  name: string;
  avatar: string;
  description: string;
  instructions: string;
  model: string | null;
  tools: string[];
  skills: string[];
  think: boolean;
  createdAt: string;
}

export type Frequency =
  | { kind: 'once'; date: string; time: string }
  | { kind: 'daily'; time: string }
  | { kind: 'weekly'; days: number[]; time: string }
  | { kind: 'monthly'; day: number; time: string };

export interface Task {
  id: string;
  title: string;
  prompt: string;
  frequency: Frequency;
  agentId: string | null;
  model: string | null;
  enabled: boolean;
  allowChanges: boolean;
  createdAt: string;
  lastRun: string | null;
  lastConversation: string | null;
  lastStatus: string | null;
  nextRun: string | null;
  /** "Every Monday at 09:00". */
  schedule: string;
}

export type Speed = 'fast' | 'think';
export type ApprovalMode = 'ask' | 'accept_edits';

export type ChatEvent =
  | { type: 'started'; conversationId: string }
  | { type: 'text'; conversationId: string; text: string }
  | { type: 'reasoning'; conversationId: string; text: string }
  | { type: 'toolPending'; conversationId: string; name: string }
  | { type: 'toolStart'; conversationId: string; callId: string; name: string; arguments: Record<string, unknown> }
  | { type: 'toolEnd'; conversationId: string; callId: string; output: string; isError: boolean }
  | { type: 'question'; conversationId: string; callId: string; question: string; options: string[] }
  | { type: 'title'; conversationId: string; title: string }
  | { type: 'subagent'; conversationId: string; callId: string; index: number; title: string; status: string; activity: string }
  | { type: 'usage'; conversationId: string; usage: UsageTotals; contextWindow: number }
  | { type: 'compacting'; conversationId: string }
  | { type: 'compacted'; conversationId: string; upto: number }
  | { type: 'done'; conversationId: string; cancelled: boolean }
  | { type: 'error'; conversationId: string; message: string };

export interface ApprovalRequest {
  request_id: string;
  connector_id: string | null;
  tool_name: string;
  input: unknown;
  reason: { kind: string; sources: string[]; destination?: string } | null;
}

/** A connector as `get_tool_registry` reports it; only the fields the page reads. */
export interface ConnectorManifest {
  id: string;
  display_name: string;
  description?: string;
  icon: string | null;
  enabled: boolean;
  readiness: { state: string; reason?: string };
  tools: { name: string; title?: string; description: string; policy: 'auto_approve' | 'ask' | 'disable'; read_only?: boolean }[];
}

const call = tauriInvoke;

export const api = {
  providers: () => call<Provider[]>('chat_list_providers'),
  saveProvider: (provider: {
    vendor: string;
    enabled: boolean;
    baseUrl?: string | null;
    customModels?: string[];
    apiKey?: string;
    clearKey?: boolean;
  }) => call<Provider>('chat_save_provider', { provider }),
  models: () => call<{ models: Model[]; defaultModel: string | null }>('chat_list_models'),
  setDefaultModel: (reference: string) => call<void>('chat_set_default_model', { reference }),
  testModel: (reference: string) => call<string>('chat_test_model', { reference }),

  conversations: () => call<ConversationSummary[]>('chat_list_conversations'),
  conversation: (id: string) => call<Conversation>('chat_get_conversation', { id }),
  send: (request: {
    conversationId: string;
    text: string;
    model: string;
    speed: Speed;
    approval: ApprovalMode;
    projectId?: string | null;
    workdir?: string | null;
    resume?: boolean;
    agentId?: string | null;
    editIndex?: number;
    temporary?: boolean;
    images?: { mediaType: string; data: string }[];
  }) => call<void>('chat_send', { request }),
  cancel: (conversationId: string) => call<boolean>('chat_cancel', { conversationId }),
  answer: (callId: string, answer: string) => call<boolean>('chat_answer', { callId, answer }),
  updateConversation: (update: {
    id: string;
    title?: string;
    pinned?: boolean;
    unread?: boolean;
    projectId?: string | null;
    rating?: [number, number];
  }) => call<void>('chat_update_conversation', { update }),
  deleteConversation: (id: string) => call<void>('chat_delete_conversation', { id }),
  keepConversation: (id: string) => call<void>('chat_keep_conversation', { id }),
  exportConversation: (id: string) => call<boolean>('chat_export_conversation', { id }),

  projects: () => call<Project[]>('chat_list_projects'),
  saveProject: (project: Project) => call<Project[]>('chat_save_project', { project }),
  deleteProject: (id: string) => call<Project[]>('chat_delete_project', { id }),

  instructions: () => call<Instructions>('chat_get_instructions'),
  saveInstructions: (instructions: Instructions) => call<void>('chat_save_instructions', { instructions }),
  previewInstructions: (instructions: Instructions, model: string, prompt: string) =>
    call<string>('chat_preview_instructions', { instructions, model, prompt }),

  memories: () => call<Memories>('chat_get_memories'),
  addMemory: (text: string) => call<Memories>('chat_add_memory', { text }),
  updateMemory: (id: string, text: string) => call<Memories>('chat_update_memory', { id, text }),
  deleteMemory: (id: string | null) => call<Memories>('chat_delete_memory', { id }),
  setMemoryEnabled: (enabled: boolean) => call<Memories>('chat_set_memory_enabled', { enabled }),

  skills: () => call<Skill[]>('chat_list_skills'),
  saveSkill: (name: string, description: string, body: string) => call<Skill[]>('chat_save_skill', { name, description, body }),
  deleteSkill: (name: string) => call<Skill[]>('chat_delete_skill', { name }),
  setSkillEnabled: (name: string, enabled: boolean) => call<Skill[]>('chat_set_skill_enabled', { name, enabled }),
  importSkill: () => call<string | null>('chat_import_skill'),

  agents: () => call<Agent[]>('chat_list_agents'),
  agentTemplates: () => call<Agent[]>('chat_agent_templates'),
  saveAgent: (agent: Agent) => call<Agent[]>('chat_save_agent', { agent }),
  deleteAgent: (id: string) => call<Agent[]>('chat_delete_agent', { id }),

  tasks: () => call<Task[]>('chat_list_tasks'),
  saveTask: (task: Omit<Task, 'schedule'>) => call<Task[]>('chat_save_task', { task }),
  deleteTask: (id: string) => call<Task[]>('chat_delete_task', { id }),
  runTask: (id: string) => call<void>('chat_run_task', { id }),

  respondApproval: (requestId: string, approved: boolean, always = false) =>
    call<void>('respond_tool_approval', { requestId, approved, always }),

  connectors: () => call<ConnectorManifest[]>('get_tool_registry', {}),
  setConnectorEnabled: (connectorId: string, enabled: boolean) =>
    call<void>('set_connector_enabled', { connectorId, enabled }),
  setToolPolicy: (connectorId: string, toolName: string, policy: string) =>
    call<void>('set_tool_policy', { connectorId, toolName, policy }),

  pickDirectory: () => call<string | null>('pick_directory'),
  appSettings: () => call<AppSettings>('get_app_settings'),
  saveAppSettings: (settings: AppSettings) => call<void>('save_app_settings', { settings }),
};

export interface AppSettings {
  search_shortcut: string;
  search_shortcut_enabled: boolean;
  launch_at_startup: boolean;
  filesystem_allowed_roots: string[];
  filesystem_write_enabled: boolean;
  bash_enabled: boolean;
  settings_hotkey_enabled: boolean;
}

/** Subscribes to a Rust event; resolves to the unsubscribe function. */
export function listen<T>(event: string, handler: (payload: T) => void): Promise<() => void> {
  const listener = window.__TAURI__?.event?.listen ?? window.__TAURI_INTERNALS__?.event?.listen;
  if (!listener) return Promise.resolve(() => {});
  return listener(event, (e) => handler(e.payload as T));
}

export function newId(): string {
  return crypto.randomUUID();
}
