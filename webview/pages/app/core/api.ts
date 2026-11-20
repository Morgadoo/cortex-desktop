/**
 * Typed wrappers over the chat commands in `src-tauri/src/chat/commands.rs`, and the events Rust
 * pushes back. The shapes mirror the Rust structs' camelCase serialization; those generated from
 * the Rust types themselves are in `generated/ipc.ts`.
 */
import { tauriInvoke } from '../../../shared/tauri';

// Generated from the Rust types (scripts/generate-types.sh), so the two cannot drift.
import type {
  AgentSpend,
  ChatEvent,
  ChatParams,
  ContextBreakdown,
  Conversation,
  ConversationSummary,
  Frequency,
  Instructions,
  Memories,
  Memory,
  ModelView,
  Part,
  Project,
  ProviderSettings,
  ProviderView,
  SearchHit,
  Skill,
  Speed,
  StorageHealth,
  TrashItem,
  Turn,
  UsageTotals,
  Workflow,
} from './generated/ipc';

export type {
  AgentSpend,
  ChatEvent,
  ChatParams,
  ContextBreakdown,
  Conversation,
  ConversationSummary,
  Frequency,
  Instructions,
  Memories,
  Memory,
  Part,
  Project,
  ProviderSettings,
  SearchHit,
  Skill,
  Speed,
  StorageHealth,
  TrashItem,
  Turn,
  UsageTotals,
  Workflow,
};
export type Provider = ProviderView;
export type Model = ModelView;

export interface Agent {
  id: string;
  name: string;
  /** Its job title ("3D Artist"), shown next to its name: the role that sets its responsibilities. */
  title?: string;
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
  /** A group's members, by agent id (LobeHub's agent groups). */
  members?: string[];
  /** How a group's members work: parallel, sequential or debate. */
  groupMode?: string;
  /** Its code may reach the internet, to install packages into its own environment. */
  codeNetwork?: boolean;
  /** It runs an organization: creates, changes and removes the agents under it. */
  canManageAgents?: boolean;
  /** The agent it reports to, by id. */
  reportsTo?: string | null;
  createdAt: string;
}

export interface Task {
  id: string;
  title: string;
  prompt: string;
  /** The user's comments, given to every run as notes. */
  comments?: { at: string; text: string }[];
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

export type ApprovalMode = 'ask' | 'accept_edits' | 'plan' | 'auto';

export interface ApprovalRequest {
  request_id: string;
  connector_id: string | null;
  tool_name: string;
  input: unknown;
  reason: { kind: string; sources: string[]; destination?: string; detail?: string } | null;
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
  saveProvider: (provider: { vendor: string; enabled: boolean; baseUrl?: string | null; customModels?: string[]; apiKey?: string; clearKey?: boolean }) => call<Provider>('chat_save_provider', { provider }),
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
    /** Built-in tools turned off in the composer's Tools menu. */
    toolsOff?: string[];
  }) => call<void>('chat_send', { request }),
  cancel: (conversationId: string) => call<boolean>('chat_cancel', { conversationId }),
  answer: (callId: string, answer: string) => call<boolean>('chat_answer', { callId, answer }),
  updateConversation: (update: { id: string; title?: string; pinned?: boolean; unread?: boolean; projectId?: string | null; rating?: [number, number]; archived?: boolean; tags?: string[] }) =>
    call<void>('chat_update_conversation', { update }),
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
  previewInstructions: (instructions: Instructions, model: string, prompt: string) => call<string>('chat_preview_instructions', { instructions, model, prompt }),

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
  /** Drafts an agent, or, with `current`, revises that draft as `description` asks. */
  draftAgent: (description: string, current: Agent | null = null) => call<Agent>('chat_draft_agent', { description, current }),
  usageMonth: () => call<AgentSpend>('chat_usage_month'),
  subagentTranscript: (callId: string, index: number) => call<{ kind: string; tool: string; input: string; output: string; isError: boolean }[]>('chat_subagent_transcript', { callId, index }),
  saveAgent: (agent: Agent) => call<Agent[]>('chat_save_agent', { agent }),
  deleteAgent: (id: string) => call<Agent[]>('chat_delete_agent', { id }),

  tasks: () => call<Task[]>('chat_list_tasks'),
  saveTask: (task: Omit<Task, 'schedule'>) => call<Task[]>('chat_save_task', { task }),
  deleteTask: (id: string) => call<Task[]>('chat_delete_task', { id }),
  runTask: (id: string) => call<void>('chat_run_task', { id }),

  respondApproval: (requestId: string, approved: boolean, always = false) => call<void>('respond_tool_approval', { requestId, approved, always }),

  connectors: () => call<ConnectorManifest[]>('get_tool_registry', {}),
  setConnectorEnabled: (connectorId: string, enabled: boolean) => call<void>('set_connector_enabled', { connectorId, enabled }),
  setToolPolicy: (connectorId: string, toolName: string, policy: string) => call<void>('set_tool_policy', { connectorId, toolName, policy }),

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

export function newId(): string {
  return crypto.randomUUID();
}
