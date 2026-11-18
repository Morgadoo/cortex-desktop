/**
 * The native chat app: a sidebar of conversations and projects, and a main area showing the home
 * screen, a conversation or a context page. Model calls happen in Rust (`src-tauri/src/chat`);
 * this page renders what it streams back.
 */
import { openSearchMode } from './searchmode';
import { actionFor } from './shortcuts';
import { api, listen, type ApprovalRequest, type ChatEvent } from './api';
import { tauriInvoke } from '../../shared/tauri';
import { ChatView, CompareView, HomeView, composer, conversationMenu, recordStepTimes, refreshConversations, scrollToTurn, turnEnded, viewCache } from './chat';
import { clear, h, icon, modal, popover, promptText, confirmDialog, toast } from './dom';
import { startUiLanguage } from './i18n';
import { startWakeWord, stopWakeWord, wakePrefs } from './wakeword';
import { ConnectorsPage, PlaceholderPage, openInstructions } from './pages';
import { KnowledgePage, SkillsPage, reviewRecording } from './context';
import { AgentPage, AgentsPage, avatar, editAgent, newAgent, pinnedAgents, reloadAgents } from './agents';
import { TasksPage } from './tasks';
import { LibrariesPage } from './libraries';
import { AppsPage } from './apps';
import { WorkflowsPage } from './workflows';
import { applyTheme, openSettings, reloadModels, setTheme, storedTheme } from './settings';
import { stepZoom, applyBackdrop } from './appearance';
import { applyEvent, back, changed, finishNotificationsOn, finishSoundOn, forward, navigate, playChime, state, subscribe, type Page } from './state';
import { button, emptyState, iconButton, modalHeader } from './ui';
import { refreshAudioSettings } from './audio';

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
const SOLO_CHAT: string | null = (window as unknown as { __LUMO_SOLO_CHAT?: string }).__LUMO_SOLO_CHAT ?? null;

const sidebar = h('aside', { class: 'sidebar' });
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
  clear(main, view.el);
}

// ---------------------------------------------------------------------------------------------
// Sidebar

function navItem(ic: string, label: string, active: boolean, onClick: () => void, extra?: HTMLElement | null): HTMLElement {
  return h('button', { class: `nav-link${active ? ' active' : ''}`, on: { click: onClick } }, icon(ic, 18), h('span', { class: 'nav-label' }, label), extra ?? null);
}

const CONTEXT_PAGES: { page: Page; label: string; preview?: boolean }[] = [
  { page: 'skills', label: 'Skills' },
  { page: 'agents', label: 'Agents' },
  { page: 'knowledge', label: 'Knowledge' },
  { page: 'connectors', label: 'Connectors' },
  { page: 'libraries', label: 'Libraries' },
  { page: 'workflows', label: 'Workflows' },
];

/** The app menu at the top left (Vibe's hamburger menu): the app's own actions in one place. */
function openAppMenu(anchor: HTMLElement) {
  const item = (ic: Parameters<typeof icon>[0], label: string, keys: string | null, run: () => void) =>
    h('button', { class: 'menu-item', on: { click: () => { close(); run(); } } }, icon(ic, 16), h('span', { class: 'grow' }, label), keys ? h('small', { class: 'muted' }, keys) : null);
  const action = (name: string) => () => void tauriInvoke('app_menu_action', { action: name }).catch((e) => toast(String(e), 'error'));
  const close = popover(
    anchor,
    h(
      'div',
      { class: 'menu app-menu' },
      item('plus', 'New chat', 'Ctrl+N', newChat),
      item('search', 'Search chats and actions', 'Ctrl+K', openSearch),
      item('search', 'Search everything', 'Ctrl+Shift+F', () => openSearchMode()),
      h('div', { class: 'menu-sep' }),
      item('settings', 'Settings', 'Ctrl+,', () => void openSettings()),
      item('sparkle', "What's new", null, openWhatsNew),
      item('download', 'Check for updates', null, action('check-updates')),
      item('user', 'About Lumo', null, action('about')),
      h('div', { class: 'menu-sep' }),
      item('close', 'Quit', null, action('quit'))
    ),
    'below'
  );
}

function renderSidebar() {
  document.body.classList.toggle('sidebar-collapsed', state.sidebarCollapsed);
  const v = state.view;
  const isPage = (page: Page) => v.kind === 'page' && v.page === page;
  const top = h(
    'div',
    { class: 'sidebar-top' },
    h('button', { class: 'icon-btn app-menu-btn', title: 'Menu', on: { click: (e: MouseEvent) => openAppMenu(e.currentTarget as HTMLElement) } }, icon('menu', 18)),
    h('button', { class: 'icon-btn', title: 'Toggle sidebar', on: { click: () => { state.sidebarCollapsed = !state.sidebarCollapsed; changed(); } } }, icon('sidebar', 18)),
    h('button', { class: 'icon-btn', title: 'Back', disabled: state.history.length === 0, on: { click: back } }, icon('back', 18)),
    h('button', { class: 'icon-btn', title: 'Forward', disabled: state.future.length === 0, on: { click: forward } }, icon('forward', 18)),
    h('span', { class: 'spacer' }),
    h('button', { class: 'icon-btn', title: 'Search conversations (Ctrl+K)', on: { click: openSearch } }, icon('search', 18))
  );
  if (state.sidebarCollapsed) {
    clear(sidebar, top, h('button', { class: 'icon-btn', title: 'New chat', on: { click: newChat } }, icon('plus', 18)));
    return;
  }

  const modes = h(
    'div',
    { class: 'mode-switch' },
    (['work', 'code', 'agents'] as const).map((mode) =>
      h(
        'button',
        { class: state.mode === mode ? 'active' : '', on: { click: () => { state.mode = mode; navigate(mode === 'agents' ? { kind: 'page', page: 'agents' } : { kind: 'home' }); } } },
        mode === 'work' ? 'Work' : mode === 'code' ? 'Code' : 'Agents'
      )
    )
  );

  const nav = h('div', { class: 'nav' });
  if (state.mode === 'agents') {
    nav.append(
      navItem('plus', 'New agent', false, () => void newAgent()),
      navItem('apps', 'All agents', isPage('agents'), () => navigate({ kind: 'page', page: 'agents' })),
      navItem('clock', 'Scheduled', isPage('scheduled'), () => navigate({ kind: 'page', page: 'scheduled' }))
    );
    const scroll = h('div', { class: 'sidebar-scroll' }, nav, renderAgents());
    clear(sidebar, top, modes, scroll, renderFooter());
    return;
  }
  nav.append(navItem('plus', state.mode === 'code' ? 'New Session' : 'New Chat', v.kind === 'home', newChat));
  if (state.mode === 'work') {
    nav.append(navItem('apps', 'Apps', isPage('apps'), () => navigate({ kind: 'page', page: 'apps' }), h('span', { class: 'badge' }, 'Preview')));
  }
  const contextActive = CONTEXT_PAGES.some((c) => isPage(c.page));
  nav.append(navItem('context', 'Context', contextActive && !state.contextOpen, () => { state.contextOpen = !state.contextOpen; changed(); }));
  // Code mode's Extensions, as in Vibe: the tools a session can plug in (MCP connectors).
  if (state.mode === 'code') nav.append(navItem('apps', 'Extensions', v.kind === 'page' && v.page === 'connectors', () => navigate({ kind: 'page', page: 'connectors' })));
  if (state.contextOpen || contextActive) {
    const sub = h('div', { class: 'subnav' });
    for (const c of CONTEXT_PAGES) {
      sub.append(h('button', { class: `nav-link sub${isPage(c.page) ? ' active' : ''}`, on: { click: () => navigate({ kind: 'page', page: c.page }) } }, h('span', { class: 'nav-label' }, c.label), c.preview ? h('span', { class: 'badge' }, 'Preview') : null));
    }
    sub.append(h('button', { class: 'nav-link sub', on: { click: () => void openInstructions() } }, h('span', { class: 'nav-label' }, 'Instructions')));
    nav.append(sub);
  }
  if (state.mode === 'work') nav.append(navItem('clock', 'Scheduled', isPage('scheduled'), () => navigate({ kind: 'page', page: 'scheduled' })));

  const scroll = h('div', { class: 'sidebar-scroll' }, nav, renderProjects(), renderConversations());
  clear(sidebar, top, modes, scroll, renderFooter());
}

function renderAgents(): HTMLElement {
  const section = h('div', { class: 'sidebar-section' }, sectionHeading('Agents'));
  if (!state.agents.length) section.append(h('p', { class: 'sidebar-empty' }, 'No agents yet'));
  for (const agent of state.agents) {
    const active = (state.view.kind === 'agent' && state.view.id === agent.id) || (state.view.kind === 'home' && state.view.agentId === agent.id);
    const runs = state.conversations.filter((c) => c.agentId === agent.id);
    const unread = runs.some((c) => c.unread);
    section.append(
      h(
        'div',
        { class: `conv-link project${active ? ' active' : ''}`, on: { click: () => navigate({ kind: 'agent', id: agent.id }) } },
        avatar(agent),
        h('span', { class: 'conv-title' }, agent.name),
        unread ? h('span', { class: 'unread-dot' }) : null
      )
    );
    for (const c of runs.slice(0, 4)) section.append(conversationLink(c.id, c.title, c.pinned, true));
  }
  return section;
}

function switchMode(mode: 'work' | 'code' | 'agents') {
  state.mode = mode;
  navigate(mode === 'agents' ? { kind: 'page', page: 'agents' } : { kind: 'home' });
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
    }
    else toast('That conversation is not here.', 'error');
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
      void tauriInvoke('chat_show_window').catch(() => undefined);
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

function newChat() {
  navigate({ kind: 'home' });
  composer.setText('');
  setTimeout(() => composer.focus());
}

function sectionHeading(label: string, action?: HTMLElement): HTMLElement {
  return h('div', { class: 'section-heading' }, h('span', {}, label), action ?? null);
}

function conversationLink(id: string, title: string, pinned: boolean, nested = false): HTMLElement {
  const active = state.view.kind === 'chat' && state.view.id === id;
  const more = h('button', { class: 'icon-btn tiny row-more', title: 'More' }, icon('more', 16));
  more.addEventListener('click', (e) => {
    e.stopPropagation();
    conversationMenu(more, id, title, pinned);
  });
  const running = state.live.has(id) && !state.live.get(id)?.error;
  // Hovering shows the title and how the conversation left off.
  const preview = state.conversations.find((c) => c.id === id)?.preview;
  return h(
    'div',
    { class: `conv-link${active ? ' active' : ''}${nested ? ' nested' : ''}`, 'data-chat-id': id, on: { click: () => navigate({ kind: 'chat', id }) }, title: preview ? `${title}\n\n${preview}` : title },
    pinned ? icon('pin', 13) : null,
    h('span', { class: 'conv-title' }, title),
    state.conversations.find((c) => c.id === id)?.unread ? h('span', { class: 'unread-dot' }) : null,
    running ? h('span', { class: 'dots small' }) : null,
    more
  );
}

function renderProjects(): HTMLElement {
  const add = h('button', { class: 'icon-btn tiny', title: 'New project' }, icon('plus', 15));
  add.addEventListener('click', async () => {
    const name = await promptText('New project', '', 'Project name');
    if (!name) return;
    state.projects = await api.saveProject({ id: crypto.randomUUID(), name, instructions: '', tone: '', icon: '', libraryId: null, shareChats: false });
    changed();
  });
  const section = h('div', { class: 'sidebar-section' }, sectionHeading('Projects', add));
  for (const project of state.projects) {
    const active = state.view.kind === 'home' && state.view.projectId === project.id;
    const more = h('button', { class: 'icon-btn tiny row-more', title: 'More' }, icon('more', 16));
    more.addEventListener('click', (e) => {
      e.stopPropagation();
      projectMenu(more, project.id);
    });
    section.append(
      h('div', { class: `conv-link project${active ? ' active' : ''}`, on: { click: () => navigate({ kind: 'home', projectId: project.id }) } }, project.icon ? h('span', { class: 'project-icon' }, project.icon) : icon('folder', 16), h('span', { class: 'conv-title' }, project.name), more)
    );
    for (const c of state.conversations.filter((c) => !c.archived && c.projectId === project.id)) {
      section.append(conversationLink(c.id, c.title, c.pinned, true));
    }
  }
  return section;
}

function projectMenu(anchor: HTMLElement, id: string) {
  const project = state.projects.find((p) => p.id === id);
  if (!project) return;
  const close = popover(
    anchor,
    h(
      'div',
      { class: 'menu' },
      h('button', { class: 'menu-item', on: { click: async () => { close(); const name = await promptText('Rename project', project.name); if (name) { state.projects = await api.saveProject({ ...project, name }); changed(); } } } }, icon('edit', 16), 'Rename'),
      h('button', { class: 'menu-item', on: { click: () => { close(); void editProject(id); } } }, icon('settings', 16), 'Project settings'),
      h('div', { class: 'menu-sep' }),
      h('button', { class: 'menu-item danger', on: { click: async () => { close(); if (await confirmDialog('Delete project?', `Its conversations stay, outside any project.`)) { state.projects = await api.deleteProject(id); await refreshConversations(); } } } }, icon('trash', 16), 'Delete')
    )
  );
}

function editProject(id: string) {
  const project = state.projects.find((p) => p.id === id);
  if (!project) return;
  return modal((close) => {
    const name = h('input', { class: 'input', value: project.name });
    const projIcon = h('input', { class: 'input avatar-input', value: project.icon || '📁', maxlength: 4 });
    const tone = h(
      'select',
      { class: 'input' },
      [['', 'Same as my instructions'], ['formal', 'Polite'], ['casual', 'Casual'], ['empathetic', 'Empathetic'], ['objective', 'Objective'], ['concise', 'Concise'], ['witty', 'Witty'], ['motivational', 'Motivational'], ['reflective', 'Reflective']].map(([v, l]) =>
        h('option', { value: v, selected: project.tone === v }, l)
      )
    );
    const text = h('textarea', { class: 'input textarea', rows: 6, placeholder: 'Context and rules for every conversation in this project' });
    text.value = project.instructions;
    const share = h('input', { type: 'checkbox', class: 'switch', checked: project.shareChats });
    const files = h('div', { class: 'doc-list' });
    const renderFiles = async () => {
      const lib = await tauriInvoke<{ id: string; documents: { id: string; name: string }[] }>('chat_project_library', { projectId: project.id });
      project.libraryId = lib.id;
      clear(
        files,
        lib.documents.length
          ? lib.documents.map((d) => h('div', { class: 'doc-row compact' }, icon('file', 15), h('span', { class: 'doc-name' }, d.name)))
          : h('p', { class: 'muted empty-line' }, 'No files yet.')
      );
      return lib;
    };
    void renderFiles();
    const addFiles = h('button', { class: 'btn btn-secondary' }, icon('upload', 15), 'Add files');
    addFiles.addEventListener('click', async () => {
      const lib = await renderFiles();
      const result = await tauriInvoke<{ errors: string[] } | null>('chat_add_documents', { id: lib.id });
      if (result?.errors.length) toast(result.errors.join(' '), 'error');
      await renderFiles();
    });
    const save = async () => {
      state.projects = await api.saveProject({
        ...project,
        name: name.value.trim() || project.name,
        icon: projIcon.value.trim(),
        tone: tone.value,
        instructions: text.value,
        shareChats: share.checked,
      });
      close();
      changed();
    };
    return h(
      'div',
      { class: 'dialog-small wide' },
      modalHeader('Project settings', close, { flush: true }),
      h('div', { class: 'freq-row' }, h('div', { class: 'grow' }, h('label', { class: 'field-label' }, 'Name'), name), h('div', {}, h('label', { class: 'field-label' }, 'Icon'), projIcon)),
      h('label', { class: 'field-label' }, 'Tone'),
      tone,
      h('label', { class: 'field-label' }, 'Instructions'),
      text,
      h('label', { class: 'field-label' }, 'Files', addFiles),
      files,
      h('div', { class: 'setting-row' }, h('div', {}, h('strong', {}, 'Use other chats as context'), h('small', { class: 'muted' }, "New conversations see short summaries of this project's other conversations.")), share),
      h('div', { class: 'dialog-actions' }, h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'), h('button', { class: 'btn btn-primary', on: { click: () => void save() } }, 'Save'))
    );
  });
}

type ChatSort = 'recent' | 'created' | 'title' | 'unread';
const CHAT_SORTS: Record<ChatSort, string> = { recent: 'Recent activity', created: 'Newest first', title: 'A–Z', unread: 'Unread first' };
let chatSort: ChatSort = (() => {
  try {
    const v = localStorage.getItem('lumo.chatSort');
    return v && v in CHAT_SORTS ? (v as ChatSort) : 'recent';
  } catch {
    return 'recent';
  }
})();

function checkMark(on: boolean): SVGElement {
  const mark = icon('check', 15);
  if (!on) mark.style.visibility = 'hidden';
  return mark;
}

function setChatSort(sort: ChatSort) {
  chatSort = sort;
  try {
    localStorage.setItem('lumo.chatSort', sort);
  } catch {
    // A convenience only.
  }
  changed();
}

/** The sidebar's chats in the chosen order (the list arrives newest activity first). */
function sortChats<T extends { title: string; createdAt: string; updatedAt: string; unread: boolean }>(list: T[]): T[] {
  const out = [...list];
  if (chatSort === 'created') out.sort((a, b) => b.createdAt.localeCompare(a.createdAt));
  if (chatSort === 'title') out.sort((a, b) => (a.title || 'Untitled').localeCompare(b.title || 'Untitled', undefined, { sensitivity: 'base' }));
  if (chatSort === 'unread') out.sort((a, b) => Number(b.unread) - Number(a.unread));
  return out;
}

/** What the sidebar's chat list shows (LibreChat's and Open WebUI's filters). */
type ChatFilter = { unread: boolean; tag: string | null; within: 0 | 1 | 7 | 30; hideRuns: boolean };
const NO_FILTER: ChatFilter = { unread: false, tag: null, within: 0, hideRuns: false };
let chatFilter: ChatFilter = (() => {
  try {
    return { ...NO_FILTER, ...(JSON.parse(localStorage.getItem('lumo.chatFilter') ?? '{}') as Partial<ChatFilter>) };
  } catch {
    return { ...NO_FILTER };
  }
})();

function setChatFilter(change: Partial<ChatFilter>) {
  chatFilter = { ...chatFilter, ...change };
  try {
    localStorage.setItem('lumo.chatFilter', JSON.stringify(chatFilter));
  } catch {
    // A convenience only.
  }
  changed();
}

const filtering = () => chatFilter.unread || chatFilter.tag !== null || chatFilter.within !== 0 || chatFilter.hideRuns;

function filterChats<T extends { unread: boolean; tags: string[]; updatedAt: string; taskId: string | null }>(list: T[]): T[] {
  const since = chatFilter.within ? Date.now() - chatFilter.within * 86_400_000 : 0;
  return list.filter(
    (c) =>
      (!chatFilter.unread || c.unread) &&
      (!chatFilter.tag || c.tags.includes(chatFilter.tag)) &&
      (!since || Date.parse(c.updatedAt) >= since) &&
      (!chatFilter.hideRuns || !c.taskId)
  );
}

/** "Today", "Yesterday", "Previous 7 days", "Previous 30 days", then month and year. */
function dateGroup(iso: string, now = new Date()): string {
  const d = new Date(iso);
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const t = d.getTime();
  if (t >= startOfToday) return 'Today';
  if (t >= startOfToday - 86_400_000) return 'Yesterday';
  if (t >= startOfToday - 7 * 86_400_000) return 'Previous 7 days';
  if (t >= startOfToday - 30 * 86_400_000) return 'Previous 30 days';
  return d.toLocaleDateString(undefined, { month: 'long', ...(d.getFullYear() === now.getFullYear() ? {} : { year: 'numeric' }) });
}

/** Chats listed before "Show more"; search (Ctrl+K) reaches all of them. */
const SIDEBAR_LIMIT = 30;
let sidebarExpanded = false;

function renderConversations(): HTMLElement {
  const code = state.mode === 'code';
  const section = h('div', { class: 'sidebar-section' });
  const all = state.conversations.filter((c) => !c.archived && !c.projectId && !c.agentId && Boolean(c.workdir) === code);
  const pinned = all.filter((c) => c.pinned);
  const loose = sortChats(filterChats(all.filter((c) => !c.pinned)));
  // Pinned agents (Work mode) start a new chat with the agent.
  const agents = code ? [] : pinnedAgents().map((id) => state.agents.find((a) => a.id === id)).filter((a): a is NonNullable<typeof a> => !!a);
  if (pinned.length || agents.length) {
    section.append(sectionHeading('Pinned'));
    for (const a of agents) {
      const active = state.view.kind === 'home' && state.view.agentId === a.id;
      section.append(h('div', { class: `conv-link project${active ? ' active' : ''}`, title: `New chat with ${a.name}`, on: { click: () => navigate({ kind: 'home', agentId: a.id }) } }, avatar(a), h('span', { class: 'conv-title' }, a.name)));
    }
    for (const c of pinned) section.append(conversationLink(c.id, c.title, true));
  }
  const sortBtn = h('button', { class: `icon-btn tiny${filtering() ? ' active' : ''}`, title: `Sort: ${CHAT_SORTS[chatSort]}${filtering() ? ' · filtered' : ''}` }, icon('list', 14));
  sortBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    const close = popover(
      sortBtn,
      h(
        'div',
        { class: 'menu' },
        h('div', { class: 'menu-heading' }, 'Sort chats'),
        (Object.keys(CHAT_SORTS) as ChatSort[]).map((k) =>
          h('button', { class: `menu-item${chatSort === k ? ' selected' : ''}`, on: { click: () => { close(); setChatSort(k); } } }, checkMark(chatSort === k), h('span', {}, CHAT_SORTS[k]))
        ),
        h('div', { class: 'menu-sep' }),
        h('div', { class: 'menu-heading' }, 'Show'),
        h('button', { class: 'menu-item', on: { click: () => { close(); setChatFilter({ unread: !chatFilter.unread }); } } }, checkMark(chatFilter.unread), h('span', {}, 'Unread only')),
        h('button', { class: 'menu-item', on: { click: () => { close(); setChatFilter({ hideRuns: !chatFilter.hideRuns }); } } }, checkMark(chatFilter.hideRuns), h('span', {}, 'Hide scheduled runs')),
        ([[0, 'Any time'], [1, 'Last 24 hours'], [7, 'Last 7 days'], [30, 'Last 30 days']] as [ChatFilter['within'], string][]).map(([days, label]) =>
          h('button', { class: 'menu-item', on: { click: () => { close(); setChatFilter({ within: days }); } } }, checkMark(chatFilter.within === days), h('span', {}, label))
        ),
        tags.length ? h('div', { class: 'menu-heading' }, 'Tag') : null,
        tags.map((t) =>
          h('button', { class: 'menu-item', on: { click: () => { close(); setChatFilter({ tag: chatFilter.tag === t ? null : t }); } } }, checkMark(chatFilter.tag === t), h('span', {}, `#${t}`))
        ),
        filtering() ? h('div', { class: 'menu-sep' }) : null,
        filtering() ? h('button', { class: 'menu-item', on: { click: () => { close(); setChatFilter({ ...NO_FILTER }); } } }, icon('close', 15), h('span', {}, 'Clear filters')) : null
      )
    );
  });
  const tags = [...new Set(all.flatMap((c) => c.tags))].sort();
  section.append(sectionHeading(code ? 'Sessions' : 'Chats', sortBtn));
  if (!loose.length) {
    section.append(
      filtering()
        ? h('p', { class: 'sidebar-empty' }, 'No chats match the filters. ', h('button', { class: 'link-btn', on: { click: () => setChatFilter({ ...NO_FILTER }) } }, 'Clear'))
        : h('p', { class: 'sidebar-empty' }, code ? 'No sessions yet' : 'No chats yet')
    );
  }
  // A history of years stays a short list: the newest, the pinned and the open one, then more on request.
  const openId = state.view.kind === 'chat' ? state.view.id : null;
  const shown = sidebarExpanded ? loose : loose.filter((c, i) => i < SIDEBAR_LIMIT || c.id === openId);
  // By recent activity, chats sit under date headings, as in ChatGPT, LibreChat and Open WebUI.
  let group = '';
  for (const c of shown) {
    if (chatSort === 'recent' || chatSort === 'created') {
      const g = dateGroup(chatSort === 'recent' ? c.updatedAt : c.createdAt);
      if (g !== group) {
        group = g;
        section.append(h('div', { class: 'sidebar-date' }, g));
      }
    }
    section.append(conversationLink(c.id, c.title, c.pinned));
  }
  if (loose.length > shown.length || (sidebarExpanded && loose.length > SIDEBAR_LIMIT)) {
    section.append(
      h(
        'button',
        { class: 'nav-link sub show-more', on: { click: () => { sidebarExpanded = !sidebarExpanded; changed(); } } },
        icon(sidebarExpanded ? 'chevron' : 'more', 15),
        h('span', { class: 'nav-label' }, sidebarExpanded ? 'Show fewer' : `Show ${loose.length - shown.length} more`)
      )
    );
  }
  const archived = state.conversations.filter((c) => c.archived);
  if (archived.length) {
    section.append(h('button', { class: 'nav-link sub archived-link', on: { click: openArchived } }, icon('archive', 15), h('span', { class: 'nav-label' }, `Archived (${archived.length})`)));
  }
  return section;
}

/** Archived conversations: open, restore or delete them. */
function openArchived() {
  void modal((close) => {
    const list = h('div', { class: 'archived-list' });
    const render = () => {
      const archived = state.conversations.filter((c) => c.archived);
      clear(
        list,
        archived.length
          ? archived.map((c) =>
              h(
                'div',
                { class: 'archived-row' },
                h('button', { class: 'grow archived-title', on: { click: () => { close(); navigate({ kind: 'chat', id: c.id }); } } }, c.title),
                button({ variant: 'subtle', size: 'sm', icon: 'reset', label: 'Restore', onClick: async () => { await api.updateConversation({ id: c.id, archived: false }); await refreshConversations(); render(); } }),
                iconButton({ icon: 'trash', title: 'Delete', size: 'sm', onClick: async () => { await api.deleteConversation(c.id); await refreshConversations(); render(); } })
              )
            )
          : emptyState('archive', 'Nothing archived', 'Archive a conversation from its menu to tidy the sidebar without deleting it.')
      );
    };
    render();
    return h('div', { class: 'dialog-small wide' }, modalHeader('Archived', close, { flush: true }), list);
  });
}

/** What changed in this version, as Vibe's /release-notes. */
const WHATS_NEW: { title: string; items: string[] }[] = [
  {
    title: 'Agents',
    items: [
      'Describe an agent in a sentence and it is drafted for you to review.',
      'Each agent keeps a notebook it updates itself, read in every conversation and run.',
      'Agents hand tasks to each other; a backup model takes over when a provider fails.',
      'Spend per agent this month, with an optional monthly budget.',
    ],
  },
  {
    title: 'Conversations',
    items: [
      'Apps open in a side panel beside the chat.',
      'Search (Ctrl+K) covers everything said, with highlighted matches.',
      'Up / Down recall earlier prompts; Ctrl+R searches them. Ctrl+O folds all activity.',
      'The prompt you are reading the answer to stays pinned at the top.',
      'Click the token meter to see what fills the context window. Branch a chat from its menu.',
    ],
  },
  {
    title: 'Safety and data',
    items: [
      'Commands that are hard to undo always ask, even in Full auto.',
      'Deleted chats stay 30 days in Recently deleted; daily copies protect against damage.',
      'Settings → Audio: choose speech-to-text and text-to-speech providers.',
    ],
  },
];

function openWhatsNew() {
  void modal((close) =>
    h(
      'div',
      { class: 'dialog-small wide whats-new' },
      modalHeader(`What's new in Lumo ${window.lumoDesktopVersion ?? ''}`, close, { flush: true }),
      WHATS_NEW.map((section) => h('section', {}, h('h4', {}, section.title), h('ul', {}, section.items.map((item) => h('li', {}, item)))))
    )
  );
}

function renderFooter(): HTMLElement {
  const name = state.instructions.name || 'You';
  const initials = name.split(/\s+/).filter(Boolean).slice(0, 2).map((w) => w[0]!.toUpperCase()).join('') || 'U';
  const model = state.models.find((m) => m.reference === state.model);
  const user = h(
    'button',
    { class: 'user-btn' },
    h('span', { class: 'avatar' }, initials),
    h('span', { class: 'user-text' }, h('strong', {}, name), h('small', {}, model ? model.providerLabel : 'No provider yet'))
  );
  user.addEventListener('click', () => {
    const theme = storedTheme();
    // This month's usage across every conversation, as Vibe's footer gauge.
    const usage = h('button', { class: 'usage-gauge', title: 'Usage by model and by agent', on: { click: () => { close(); openUsageBreakdown(); } } }, h('span', { class: 'muted' }, 'This month'), h('span', { class: 'dots' }));
    void api
      .usageMonth()
      .then((u) => {
        const tokens = u.tokens >= 1_000_000 ? `${(u.tokens / 1_000_000).toFixed(1)}M` : `${Math.round(u.tokens / 1000)}k`;
        clear(usage, h('span', { class: 'muted' }, 'This month'), h('span', {}, `${tokens} tokens${u.cost > 0 ? ` · $${u.cost.toFixed(2)}` : ''} · ${u.runs} chats`));
      })
      .catch(() => clear(usage));
    const themeBtn = (t: 'light' | 'dark' | 'system', ic: string) =>
      h('button', { class: `icon-btn${theme === t ? ' active' : ''}`, title: t, on: { click: () => { setTheme(t); close(); } } }, icon(ic, 17));
    const close = popover(
      user,
      h(
        'div',
        { class: 'menu user-menu' },
        h('div', { class: 'user-head' }, h('span', { class: 'avatar' }, initials), h('div', {}, h('strong', {}, name), h('small', { class: 'muted' }, 'Local profile'))),
        h('div', { class: 'menu-sep' }),
        h('button', { class: 'menu-item', on: { click: () => { close(); void openSettings('profile'); } } }, icon('settings', 16), h('span', {}, 'Settings'), h('kbd', {}, 'Ctrl+,')),
        h('button', { class: 'menu-item', on: { click: () => { close(); void openSettings('providers'); } } }, icon('key', 16), 'Providers & models'),
        h('button', { class: 'menu-item', on: { click: () => { close(); void openSettings('permissions'); } } }, icon('shield', 16), 'Permissions'),
        h('button', { class: 'menu-item', on: { click: () => { close(); openWhatsNew(); } } }, icon('sparkle', 16), "What's new"),
        h('div', { class: 'menu-sep' }),
        usage,
        h('div', { class: 'theme-row' }, themeBtn('light', 'sun'), themeBtn('dark', 'moon'), themeBtn('system', 'monitor')),
        h('div', { class: 'menu-foot muted' }, `Lumo ${window.lumoDesktopVersion ?? ''} · open source, GPL-3.0`)
      ),
      'above'
    );
  });
  return h('div', { class: 'sidebar-footer' }, user);
}

// ---------------------------------------------------------------------------------------------
// Search

/** Opens a chat and scrolls to one of its messages once it is on screen. */
function openAt(id: string, index: number) {
  navigate({ kind: 'chat', id });
  let tries = 0;
  const attempt = () => {
    if (scrollToTurn(index) || ++tries > 30) return;
    setTimeout(attempt, 100);
  };
  setTimeout(attempt, 150);
}

function openSearch() {
  void modal((close) => {
    const input = h('input', { class: 'input', placeholder: 'Search chats and actions, or #tag' });
    const results = h('div', { class: 'search-results' });
    const row = (c: { id: string; title: string; archived: boolean; tags: string[]; updatedAt: string }, snippet: HTMLElement | null) =>
      h(
        'button',
        { class: `menu-item${snippet ? ' rich' : ''}`, on: { click: () => { close(); navigate({ kind: 'chat', id: c.id }); } } },
        icon(c.archived ? 'archive' : 'chat', 16),
        snippet ? h('span', {}, h('strong', {}, c.title), snippet) : h('span', {}, c.title),
        c.tags.length ? h('span', { class: 'tag-list' }, c.tags.map((t) => h('span', { class: 'tag' }, `#${t}`))) : null,
        h('span', { class: 'search-date' }, new Date(c.updatedAt).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' }))
      );
    /** The snippet with the searched words marked. */
    const marked = (text: string, terms: string[]): HTMLElement => {
      const out = h('small', {});
      const pattern = new RegExp(`(${terms.map((t) => t.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|')})`, 'gi');
      text.split(pattern).forEach((piece, i) => out.append(i % 2 ? h('mark', {}, piece) : piece));
      return out;
    };
    const actions: { label: string; icon: string; keys?: string; run: () => void }[] = [
      { label: 'New chat', icon: 'plus', keys: 'Ctrl+N', run: newChat },
      { label: 'Switch to Work', icon: 'chat', keys: 'Ctrl+Shift+1', run: () => switchMode('work') },
      { label: 'Switch to Code', icon: 'terminal', keys: 'Ctrl+Shift+2', run: () => switchMode('code') },
      { label: 'Switch to Agents', icon: 'sparkle', keys: 'Ctrl+Shift+3', run: () => switchMode('agents') },
      { label: 'Skills', icon: 'skill', run: () => navigate({ kind: 'page', page: 'skills' }) },
      { label: 'Knowledge', icon: 'book', run: () => navigate({ kind: 'page', page: 'knowledge' }) },
      { label: 'Connectors', icon: 'plug', run: () => navigate({ kind: 'page', page: 'connectors' }) },
      { label: 'Libraries', icon: 'library', run: () => navigate({ kind: 'page', page: 'libraries' }) },
      { label: 'Workflows', icon: 'workflow', run: () => navigate({ kind: 'page', page: 'workflows' }) },
      { label: 'Scheduled', icon: 'clock', run: () => navigate({ kind: 'page', page: 'scheduled' }) },
      { label: 'Apps', icon: 'apps', run: () => navigate({ kind: 'page', page: 'apps' }) },
      { label: 'Settings', icon: 'settings', keys: 'Ctrl+,', run: () => void openSettings() },
      { label: 'Search everything…', icon: 'search', keys: 'Ctrl+Shift+F', run: () => openSearchMode() },
      { label: 'Providers & models', icon: 'key', run: () => void openSettings('providers') },
      { label: 'Audio settings', icon: 'mic', run: () => void openSettings('audio') },
      { label: 'Appearance', icon: 'palette', run: () => void openSettings('appearance') },
      { label: 'Recently deleted', icon: 'trash', run: () => void openSettings('profile') },
      { label: 'Archived chats', icon: 'archive', run: openArchived },
      { label: 'Toggle sidebar', icon: 'sidebar', keys: 'Ctrl+B', run: () => { state.sidebarCollapsed = !state.sidebarCollapsed; changed(); } },
      { label: "What's new", icon: 'sparkle', run: openWhatsNew },
    ];
    const actionRows = (q: string) => {
      const found = actions.filter((a) => !q || a.label.toLowerCase().includes(q));
      if (!found.length) return [];
      return [
        h('div', { class: 'menu-heading' }, 'Actions'),
        ...found.slice(0, q ? 6 : 15).map((a) =>
          h('button', { class: 'menu-item', on: { click: () => { close(); a.run(); } } }, icon(a.icon, 16), h('span', {}, a.label), a.keys ? h('kbd', {}, a.keys) : null)
        ),
      ];
    };
    /** Pinned chats and your agents first, a keystroke away (LibreChat's favorites). */
    const pinnedRows = () => {
      const pinned = state.conversations.filter((c) => c.pinned && !c.archived).slice(0, 6);
      const agents = state.agents.slice(0, 6);
      if (!pinned.length && !agents.length) return [];
      return [
        h('div', { class: 'menu-heading' }, 'Pinned and agents'),
        ...pinned.map((c) => h('button', { class: 'menu-item', on: { click: () => { close(); navigate({ kind: 'chat', id: c.id }); } } }, icon('pin', 16), h('span', {}, c.title))),
        ...agents.map((a) => h('button', { class: 'menu-item', on: { click: () => { close(); state.mode = 'agents'; navigate({ kind: 'home', agentId: a.id }); } } }, h('span', { class: 'agent-avatar small' }, a.avatar || a.name.charAt(0)), h('span', {}, `Chat with ${a.name}`))),
      ];
    };
    /** Bookmarked messages across chats, matched by their snippet or chat title. */
    const bookmarkRows = (q: string) => {
      const marks = state.conversations.flatMap((c) => (c.bookmarks ?? []).map(([index, snippet]) => ({ c, index, snippet })));
      const found = marks.filter((m) => !q || m.snippet.toLowerCase().includes(q) || m.c.title.toLowerCase().includes(q));
      if (!found.length) return [];
      return [
        h('div', { class: 'menu-heading' }, 'Bookmarks'),
        ...found.slice(0, q ? 8 : 5).map((m) =>
          h(
            'button',
            { class: 'menu-item rich', on: { click: () => { close(); openAt(m.c.id, m.index); } } },
            icon('bookmark', 16),
            h('span', {}, h('strong', {}, m.c.title), h('small', {}, m.snippet))
          )
        ),
      ];
    };
    let latest = 0;
    const render = async () => {
      const q = input.value.toLowerCase().trim();
      // `#tag` matches tags; words search titles and everything said, through the index.
      if (!q || q.startsWith('#')) {
        const tag = q.slice(1);
        const found = state.conversations.filter((c) => !q || c.tags.some((t) => t.startsWith(tag))).slice(0, q ? 30 : 8);
        clear(results, q ? null : pinnedRows(), q ? null : bookmarkRows(''), q ? null : actionRows(''), found.length ? [h('div', { class: 'menu-heading' }, 'Chats'), ...found.map((c) => row(c, null))] : h('p', { class: 'muted' }, 'Nothing found.'));
        return;
      }
      const ticket = ++latest;
      const hits = await api.search(q).catch(() => []);
      if (ticket !== latest) return;
      const terms = q.split(/\s+/).filter(Boolean);
      const acts = actionRows(q);
      const marks = bookmarkRows(q);
      clear(
        results,
        acts,
        marks,
        hits.length ? [h('div', { class: 'menu-heading' }, 'Chats'), ...hits.map((hit) => row(hit, hit.snippet ? marked(hit.snippet, terms) : null))] : acts.length ? null : h('p', { class: 'muted' }, 'Nothing found in your conversations.')
      );
    };
    input.addEventListener('input', () => void render());
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') (results.querySelector('button') as HTMLButtonElement | null)?.click();
    });
    void render();
    return h('div', { class: 'dialog-small wide' }, input, results);
  });
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
  void refreshAudioSettings();
  applyTheme(storedTheme());
  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => applyTheme(storedTheme()));
  try {
    state.workdir = localStorage.getItem('workdir');
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
    const text = (live?.parts ?? []).filter((p): p is Extract<typeof p, { kind: 'text' }> => p.kind === 'text').map((p) => p.text).join(' ');
    const plain = text.replace(/```[\s\S]*?```/g, ' [code] ').replace(/[#*_`>|]/g, '').replace(/\s+/g, ' ').trim();
    const body = failed ? `The reply could not finish${live?.error ? `: ${live.error}` : '.'}` : plain || 'Your answer is ready.';
    void tauriInvoke('chat_notify', { title: summary?.title || 'Lumo', body }).catch(() => undefined);
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
    // Links into the app (lumo://new?prompt=…, lumo://chat/<id>): the prompt is typed, never sent.
    void listen('chat://link', () => void takeLink());
    // The wake word: "Hey Lumo, …" asks in a new chat; "Hey Lumo" alone starts a voice conversation.
    if (wakePrefs().enabled) void startWake();
    window.addEventListener('lumo:wake-word-changed', () => {
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

  const [instructions, projects, conversations] = await Promise.all([
    api.instructions().catch(() => state.instructions),
    api.projects().catch(() => []),
    api.conversations().catch(() => []),
  ]);
  state.instructions = instructions;
  state.projects = projects;
  state.conversations = conversations;
  state.skills = await api.skills().catch(() => []);
  await reloadAgents().catch(() => undefined);
  await reloadModels().catch((e) => toast(String(e), 'error'));
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

/** This month's usage by model and by agent (LobeHub's stats). */
function openUsageBreakdown() {
  type Line = { name: string; chats: number; tokens: number; cost: number };
  void modal((close) => {
    const body = h('div', {}, h('div', { class: 'status-line' }, h('span', { class: 'dots' })));
    const fmt = (n: number) => (n >= 1_000_000 ? `${(n / 1_000_000).toFixed(1)}M` : n >= 1000 ? `${Math.round(n / 1000)}k` : String(n));
    const table = (title: string, lines: Line[]) => {
      const max = Math.max(1, ...lines.map((l) => l.tokens));
      return h(
        'div',
        { class: 'usage-table' },
        h('h3', {}, title),
        lines.length
          ? lines.map((l) =>
              h(
                'div',
                { class: 'usage-row' },
                h('span', { class: 'usage-name no-translate' }, l.name),
                h('span', { class: 'usage-bar' }, h('span', { style: { width: `${Math.max(2, (l.tokens / max) * 100)}%` } })),
                h('span', { class: 'muted' }, `${fmt(l.tokens)} tokens`),
                h('span', { class: 'muted' }, l.cost > 0 ? `$${l.cost.toFixed(2)}` : '-'),
                h('span', { class: 'muted' }, `${l.chats} ${l.chats === 1 ? 'chat' : 'chats'}`)
              )
            )
          : h('p', { class: 'muted' }, 'Nothing yet this month.')
      );
    };
    void tauriInvoke<[Line[], Line[]]>('chat_usage_breakdown')
      .then(([models, agents]) => clear(body, table('By model', models), table('By agent', agents)))
      .catch((e) => clear(body, h('p', { class: 'error-text' }, String(e))));
    return h('div', { class: 'dialog-small wide' }, modalHeader('Usage this month', close, { flush: true }), h('p', { class: 'muted' }, 'Tokens and cost of the chats active this month. Cost is shown where the provider reports it.'), body);
  });
}
