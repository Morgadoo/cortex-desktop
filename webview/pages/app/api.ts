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
  /** How the conversation left off, for the sidebar's hover. */
  preview?: string;
  /** Bookmarked messages: turn index and snippet. */
  bookmarks?: [number, string][];
  unread: boolean;
  pinned: boolean;
  archived: boolean;
  tags: string[];
  createdAt: string;
  updatedAt: string;
}

export interface SearchHit {
  id: string;
  title: string;
  snippet: string;
  inTitle: boolean;
  archived: boolean;
  tags: string[];
  updatedAt: string;
}

export interface TrashItem {
  id: string;
  title: string;
  deletedAt: string;
  daysLeft: number;
}

export interface StorageHealth {
  conversations: number;
  damaged: { id: string; backup: string | null }[];
  trash: number;
  backupDays: number;
  newestBackup: string | null;
  bytes: number;
  keepDays: number;
}

export interface ContextBreakdown {
  window: number;
  measured: number;
  slices: { label: string; tokens: number }[];
  summarizedMessages: number;
  clearedResults: number;
}

export type Part =
  | { kind: 'text'; text: string }
  | { kind: 'reasoning'; text: string; ms?: number }
  | { kind: 'notice'; text: string }
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

export interface ChatParams {
  temperature: number | null;
  maxTokens: number | null;
  /** Nucleus sampling, 0–1. */
  topP?: number | null;
  instructions: string;
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
  archived: boolean;
  /** This conversation's own model settings. */
  params: ChatParams;
  /** Turn indexes the user bookmarked. */
  bookmarks: number[];
  turns: Turn[];
  running: boolean;
  /** Which answer to the last prompt shows, when it was answered more than once. */
  versions: { from: number; kind: 'answer' | 'prompt'; current: number; total: number }[];
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
  /** preference, work, person, project, habit, fact, or empty. */
  category: string;
  /** 1 (a detail) to 3 (always keep in mind). */
  importance: number;
  /** `stated` by the user or `inferred` by the assistant. */
  confidence: string;
  /** The chat it was learned in. */
  conversation?: string;
}

export interface Memories {
  enabled: boolean;
  /** Whether the assistant may save and change what it knows. */
  update?: boolean;
  items: Memory[];
  /** Vibe's "Sensitive data" switch: card numbers, bank accounts, passwords and keys are not saved. */
  keepSensitiveOut: boolean;
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
  askTools: string[];
  think: boolean;
  fallbackModel: string | null;
  monthlyBudget: number | null;
  /** Prompts offered on the agent's new-chat page. */
  starters?: string[];
  /** Libraries the agent searches, by id. */
  libraries?: string[];
  /** Vault secrets it may use, by name. */
  secrets?: string[];
  /** Added to the newest message on every request. */
  reminder?: string;
  createdAt: string;
}

export interface AgentSpend {
  runs: number;
  tokens: number;
  cost: number;
}

export type Frequency =
  | { kind: 'once'; date: string; time: string }
  | { kind: 'daily'; time: string }
  | { kind: 'weekly'; days: number[]; time: string }
  | { kind: 'monthly'; day: number; time: string }
  | { kind: 'yearly'; month: number; day: number; time: string }
  | { kind: 'newFiles'; folder: string };

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
  /** Runs continue this conversation (a follow-up asked for in a chat). */
  replyIn?: string | null;
  /** Told with a JSON POST when a run finishes. */
  webhook?: string | null;
  lastStatus: string | null;
  nextRun: string | null;
  /** "Every Monday at 09:00". */
  schedule: string;
}

export type Speed = 'fast' | 'think';
export type ApprovalMode = 'ask' | 'accept_edits' | 'plan' | 'auto';

export type ChatEvent =
  | { type: 'started'; conversationId: string }
  | { type: 'steered'; conversationId: string; text: string }
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
  | { type: 'notice'; conversationId: string; text: string }
  | { type: 'error'; conversationId: string; message: string };

export interface ApprovalRequest {
  request_id: string;
  connector_id: string | null;
  tool_name: string;
  input: unknown;
  reason: { kind: string; sources: string[]; destination?: string; detail?: string } | null;
}

/** A connector as `get_tool_registry` reports it; only the fields the page reads. */
export interface Workflow {
  id: string;
  name: string;
  description: string;
  steps: string[];
  builtin: boolean;
}

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
  setParams: (id: string, params: ChatParams) => call<ChatParams>('chat_set_params', { id, params }),
  followUps: (id: string, model: string) => call<string[]>('chat_follow_ups', { id, model }),
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
    connectors?: string[] | null;
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
    archived?: boolean;
    tags?: string[];
  }) => call<void>('chat_update_conversation', { update }),
  selectVersion: (id: string, from: number, index: number) => call<void>('chat_select_version', { id, from, index }),
  steer: (conversationId: string, text: string) => call<boolean>('chat_steer', { conversationId, text }),
  commitChanges: (conversationId: string, message: string) => call<string>('chat_code_commit', { conversationId, message }),
  importConversations: () => call<{ imported: number; skipped: number; source: string } | null>('chat_import_conversations'),
  deleteConversation: (id: string) => call<void>('chat_delete_conversation', { id }),
  search: (query: string) => call<SearchHit[]>('chat_search', { query }),
  trash: () => call<TrashItem[]>('chat_trash'),
  restoreConversation: (id: string) => call<void>('chat_restore_conversation', { id }),
  deleteForever: (id: string | null) => call<void>('chat_delete_forever', { id }),
  storageHealth: () => call<StorageHealth>('chat_storage_health'),
  recoverConversation: (id: string) => call<string>('chat_recover_conversation', { id }),
  contextBreakdown: (id: string) => call<ContextBreakdown>('chat_context_breakdown', { id }),
  forkConversation: (id: string, upto: number | null = null, withoutFiles = false, agentId: string | null = null) => call<string>('chat_fork_conversation', { id, upto, withoutFiles, agentId }),
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
  agentNotebook: (id: string) => call<string>('chat_agent_notebook', { id }),
  saveAgentNotebook: (id: string, text: string) => call<void>('chat_save_agent_notebook', { id, text }),
  agentSpend: () => call<Record<string, AgentSpend>>('chat_agent_spend'),
  draftAgent: (description: string) => call<Agent>('chat_draft_agent', { description }),
  usageMonth: () => call<AgentSpend>('chat_usage_month'),
  subagentTranscript: (callId: string, index: number) => call<{ kind: string; tool: string; input: string; output: string; isError: boolean }[]>('chat_subagent_transcript', { callId, index }),
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
