/**
 * The native chat app: a sidebar of conversations and projects, and a main area showing the home
 * screen, a conversation or a context page. Model calls happen in Rust (`src-tauri/src/chat`);
 * this page renders what it streams back.
 */
import '../core/storage-migration';
import './code-blocks';
import { openSearchMode } from '../chat/searchmode';
import { actionFor } from '../core/shortcuts';
import { api, type ApprovalRequest, type ChatEvent } from '../core/api';
import { listen, tauriInvoke } from '../../../shared/tauri';
import { recordStepTimes } from '../chat/transcript';
import { refreshConversations } from '../chat/send';
import { composer } from '../chat/composer/index';
import { HomeView } from '../chat/home-view';
import { CompareView } from '../chat/compare-view';
import { viewCache } from '../chat/chat-view';
import { ChatView } from '../chat/chat-view';
import { turnEnded } from '../chat/queue';
import { clear, h } from '../ui/h';
import { toast } from '../ui/overlay';
import { startUiLanguage } from '../core/i18n';
import { startWakeWord, stopWakeWord, wakePrefs } from '../voice/wakeword';
import { ConnectorsPage, PlaceholderPage } from '../pages/connectors';
import { SkillsPage, reviewRecording } from '../pages/skills';
import { KnowledgePage } from '../pages/knowledge';
import { AgentPage, AgentsPage } from '../pages/agents';
import { reloadAgents } from '../pages/agent-list';
import { TasksPage } from '../pages/tasks';
import { LibrariesPage } from '../pages/libraries';
import { AppsPage } from '../pages/apps';
import { ResourcesPage } from '../canvas/outputs';
import { ChatsPage } from '../pages/chatspage';
import { WorkflowsPage } from '../pages/workflows';
import { openSettings } from '../settings/index';
import { reloadModels } from '../core/models';
import { applyTheme, storedTheme } from '../theme/theme';
import { stepZoom, applyBackdrop, backdropLayer } from '../theme/appearance';
import { applyEvent, changed, finishNotificationsOn, finishSoundOn, navigate, playChime, state, subscribe } from '../core/state';
import { startFluentEmoji } from '../ui/emoji-fluent';
import { refreshAudioSettings } from '../voice/audio';
import { workdir } from '../core/prefs';
import { callQuiet, reported, shown } from '../core/call';
import { openSearch } from './search';
import { renderSidebar } from './sidebar';
import { switchMode } from './nav';
import { newChat, sidebar } from './nav';

interface View {
  readonly el: HTMLElement;
  readonly key: string;
  update(): void;
  dispose?(): void;
}

/**
 * Set when this page runs in a chat's own window (chat_open_chat_window): it shows that chat with
 * the sidebar folded, and leaves app-wide duties (links, wake word, tray, notifications) to the
 * main window so nothing happens twice.
 */
const SOLO_CHAT: string | null = (window as unknown as { __CORTEX_SOLO_CHAT?: string }).__CORTEX_SOLO_CHAT ?? null;

const main = h('main', { class: 'main' });
let view: View | null = null;

function viewKey(): string {
  const v = state.view;
  if (v.kind === 'chat') return `chat:${v.id}`;
  if (v.kind === 'compare') return `compare:${v.columns.map((c) => c.id).join(',')}`;
  if (v.kind === 'page') return v.page === 'scheduled' ? 'page:scheduled:' : `page:${v.page}`;
  if (v.kind === 'agent') return `agent:${v.id}`;
  return `home:${v.projectId ?? ''}:${v.agentId ?? ''}:${state.mode}`;
}

function renderMain() {
  const key = viewKey();
  if (view && view.key === key) {
    view.update();
    return;
  }
  view?.dispose?.();
  viewCache.chat = null;
  viewCache.home = null;
  const v = state.view;
  if (v.kind === 'chat') {
    const chat = new ChatView(v.id);
    viewCache.chat = chat;
    view = chat;
  } else if (v.kind === 'page') {
    view =
      v.page === 'connectors'
        ? new ConnectorsPage()
        : v.page === 'skills'
          ? new SkillsPage()
          : v.page === 'agents'
            ? new AgentsPage()
            : v.page === 'libraries'
              ? new LibrariesPage()
              : v.page === 'apps'
                ? new AppsPage()
                : v.page === 'workflows'
                  ? new WorkflowsPage()
                  : v.page === 'chats'
                    ? new ChatsPage()
                    : v.page === 'resources'
                      ? new ResourcesPage()
                      : v.page === 'scheduled'
                        ? new TasksPage()
                        : v.page === 'knowledge'
                          ? new KnowledgePage()
                          : new PlaceholderPage(v.page);
  } else if (v.kind === 'compare') {
    view = new CompareView(v.prompt, v.columns);
  } else if (v.kind === 'agent') {
    view = new AgentPage(v.id, (agentId) => navigate({ kind: 'home', agentId }));
  } else {
    const home = new HomeView(v.projectId ?? null, v.agentId ?? null);
    viewCache.home = home;
    view = home;
  }
  // A shader backdrop's canvas stays across views, so its WebGL context is not rebuilt.
  clear(main, backdropLayer(), view.el);
}

type AppLink = { kind: 'new'; prompt: string; model: string | null; mode: 'work' | 'code' | null } | { kind: 'open'; conversationId: string };

/** Applies a link that opened the app or arrived while it ran. */
async function takeLink() {
  const link = await tauriInvoke<AppLink | null>('chat_take_link').catch(() => null);
  if (!link) return;
  if (link.kind === 'open') {
    if (!state.conversations.some((c) => c.id === link.conversationId)) await refreshConversations();
    const found = state.conversations.find((c) => c.id === link.conversationId);
    if (found) {
      state.mode = found.workdir ? 'code' : 'work';
      navigate({ kind: 'chat', id: link.conversationId });
    } else toast('That conversation is not here.', 'error');
    return;
  }
  if (link.mode) state.mode = link.mode;
  navigate({ kind: 'home' });
  if (link.model) {
    if (state.models.some((m) => m.reference === link.model)) state.model = link.model;
    else toast(`The link asks for ${link.model}, which is not set up here; using ${state.model ?? 'your default'}.`);
  }
  composer.setText(link.prompt);
  composer.renderBar();
  setTimeout(() => composer.focus());
  if (link.prompt) toast('Opened from a link. Check the message, then send it.');
}

async function startWake() {
  try {
    await startWakeWord((request) => {
      void callQuiet('chat_show_window');
      navigate({ kind: 'home' });
      setTimeout(() => {
        if (request) {
          composer.setText(request);
          void composer.submit();
        } else composer.startVoice();
      }, 100);
    });
  } catch (e) {
    toast(`The wake word needs the microphone: ${String(e)}`, 'error');
  }
}

// ---------------------------------------------------------------------------------------------
// Startup

declare global {
  interface Window {
    lumoDesktopVersion?: string;
  }
}

async function start() {
  // The interface language first, so the first frame is already translated.
  await startUiLanguage();
  // Microsoft's Fluent emoji, as Teams shows them (or the system's).
  void startFluentEmoji();
  void refreshAudioSettings();
  applyTheme(storedTheme());
  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => applyTheme(storedTheme()));
  try {
    state.workdir = workdir.get();
  } catch {
    /* per-device convenience only */
  }

  document.body.append(h('div', { class: 'app' }, sidebar, main));
  applyBackdrop();
  subscribe(() => {
    renderSidebar();
    renderMain();
  });

  /** A desktop notification for a reply that finished while the window was in the background. */
  const notifyFinished = (id: string, failed: boolean) => {
    const summary = state.conversations.find((c) => c.id === id);
    // Scheduled tasks post their own notification.
    if (summary?.taskId) return;
    const live = state.live.get(id);
    const text = (live?.parts ?? [])
      .filter((p): p is Extract<typeof p, { kind: 'text' }> => p.kind === 'text')
      .map((p) => p.text)
      .join(' ');
    const plain = text
      .replace(/```[\s\S]*?```/g, ' [code] ')
      .replace(/[#*_`>|]/g, '')
      .replace(/\s+/g, ' ')
      .trim();
    const body = failed ? `The reply could not finish${live?.error ? `: ${live.error}` : '.'}` : plain || 'Your answer is ready.';
    void callQuiet('chat_notify', { title: summary?.title || 'Cortex', body });
    if (finishSoundOn()) playChime();
  };
  void listen<ChatEvent>('chat://event', (event) => {
    const id = event.conversationId;
    // A scheduled run starts a conversation the page has not seen yet.
    if (event.type === 'started' && !state.conversations.some((c) => c.id === id)) void refreshConversations();
    if (applyEvent(event)) {
      const live = state.live.get(id);
      if (live) recordStepTimes(id, live.starts, live.ends);
      // A failed turn keeps its live state, so the partial reply and the error stay on screen
      // until the next send; a finished one is replaced by what was saved.
      const failed = event.type === 'error';
      if (!SOLO_CHAT && (failed || (event.type === 'done' && !event.cancelled)) && !document.hasFocus() && finishNotificationsOn()) notifyFinished(id, failed);
      void refreshConversations();
      const chat = viewCache.chat;
      if (chat && chat.key === `chat:${id}`) {
        void chat.finished().then(() => {
          if (!failed) state.live.delete(id);
          chat.renderTranscript();
          chat.update();
          if (event.type === 'done' && !event.cancelled) chat.readLatestAloud();
          turnEnded(id, event.type === 'done' && !event.cancelled);
        });
      } else {
        if (!failed) state.live.delete(id);
        if (failed) toast(event.message, 'error');
      }
    }
    changed();
  });
  window.addEventListener('chat:conversations-changed', () => void refreshConversations());
  void listen<ApprovalRequest>('tool:approval_request', (request) => {
    state.approvals.push(request);
    changed();
  });
  void listen<{ request_id: string }>('tool:approval_resolved', ({ request_id }) => {
    state.approvals = state.approvals.filter((a) => a.request_id !== request_id);
    changed();
  });
  // App-wide duties stay with the main window (see SOLO_CHAT).
  if (!SOLO_CHAT) {
    // The app menu's Settings item (Ctrl+,) and the tray route here.
    void listen('chat://open-settings', () => {
      if (!document.querySelector('.modal .settings')) void openSettings();
    });
    // Links into the app (cortex://new?prompt=…, cortex://chat/<id>): the prompt is typed, never sent.
    void listen('chat://link', () => void takeLink());
    // The wake word: "Hey Cortex, …" asks in a new chat; "Hey Cortex" alone starts a voice conversation.
    if (wakePrefs().enabled) void startWake();
    window.addEventListener('cortex:wake-word-changed', () => {
      stopWakeWord();
      if (wakePrefs().enabled) void startWake();
    });
    // A task recording was stopped from its pill: turn it into a skill.
    void listen<{ seconds: number; steps: number }>('chat://recorded', (status) => reviewRecording(status));
    // Background sync brought changes from another device.
    void listen<string>('chat://synced', (summary) => {
      window.dispatchEvent(new Event('chat:conversations-changed'));
      toast(`Synced from your other devices: ${summary}`);
    });
    void listen<string>('chat://quick-ask', (query) => {
      navigate({ kind: 'home' });
      composer.setText(query);
      void composer.submit();
    });
  }

  document.addEventListener('keydown', (e) => {
    const mod = e.ctrlKey || e.metaKey;
    // Rebindable shortcuts (Settings → General → Keyboard shortcuts); a field being recorded wins.
    if (document.querySelector('.shortcut-recording')) return;
    const found = actionFor(e);
    // These act where they apply (the message box, the conversation), so focus keys still work elsewhere.
    const action = found === 'promptHistory' || found === 'toggleSteps' || found === 'cycleMode' ? null : found;
    if (action) {
      e.preventDefault();
      if (action === 'settings') {
        if (!document.querySelector('.modal .settings')) void openSettings();
      } else if (/^chat[1-9]$/.test(action)) {
        // The chats in the sidebar, in order.
        const target = [...document.querySelectorAll<HTMLElement>('.sidebar [data-chat-id]')][Number(action.slice(4)) - 1];
        if (target?.dataset.chatId) navigate({ kind: 'chat', id: target.dataset.chatId });
      } else if (action === 'search') openSearch();
      else if (action === 'searchAll') openSearchMode();
      else if (action === 'newChat') newChat();
      else if (action === 'sidebar') {
        state.sidebarCollapsed = !state.sidebarCollapsed;
        changed();
      } else if (action === 'work' || action === 'code' || action === 'agents') switchMode(action);
      return;
    }
    // Zoom the interface: Ctrl+= (or Ctrl++), Ctrl+-, Ctrl+0.
    if (mod && !e.altKey && (e.key === '=' || e.key === '+' || e.key === '-' || (e.key === '0' && !e.shiftKey))) {
      e.preventDefault();
      stepZoom(e.key === '0' ? 0 : e.key === '-' ? -1 : 1);
      return;
    }
    if (mod && e.shiftKey && e.key.toLowerCase() === 'o') {
      e.preventDefault();
      newChat();
    }
  });

  const [instructions, projects, conversations] = await Promise.all([api.instructions().catch(() => state.instructions), api.projects().catch(() => []), api.conversations().catch(() => [])]);
  state.instructions = instructions;
  state.projects = projects;
  state.conversations = conversations;
  state.skills = await api.skills().catch(() => []);
  await reloadAgents().catch(() => undefined);
  // A manager agent changed its organization (manage_agents): the lists follow.
  void listen<null>('chat://agents-changed', () => void reloadAgents().catch(() => undefined));
  await reported(reloadModels()).catch(shown);
  if (SOLO_CHAT) {
    state.sidebarCollapsed = true;
    document.body.classList.add('solo-window');
    navigate({ kind: 'chat', id: SOLO_CHAT }, false);
    return;
  }
  void takeLink();
  changed();
  if (!state.models.length) void openSettings('providers');
}

void start();
