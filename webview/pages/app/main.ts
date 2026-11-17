/**
 * The native chat app: a sidebar of conversations and projects, and a main area showing the home
 * screen, a conversation or a context page. Model calls happen in Rust (`src-tauri/src/chat`);
 * this page renders what it streams back.
 */
import { api, listen, type ApprovalRequest, type ChatEvent } from './api';
import { tauriInvoke } from '../../shared/tauri';
import { ChatView, CompareView, HomeView, composer, conversationMenu, recordStepTimes, refreshConversations, viewCache } from './chat';
import { clear, h, icon, modal, popover, promptText, confirmDialog, toast } from './dom';
import { ConnectorsPage, PlaceholderPage, openInstructions } from './pages';
import { KnowledgePage, SkillsPage } from './context';
import { AgentPage, AgentsPage, avatar, editAgent, reloadAgents } from './agents';
import { TasksPage } from './tasks';
import { LibrariesPage } from './libraries';
import { AppsPage } from './apps';
import { WorkflowsPage } from './workflows';
import { applyTheme, openSettings, reloadModels, setTheme, storedTheme } from './settings';
import { applyEvent, back, changed, forward, navigate, state, subscribe, type Page } from './state';
import { button, emptyState, iconButton, modalHeader } from './ui';

interface View {
  readonly el: HTMLElement;
  readonly key: string;
  update(): void;
  dispose?(): void;
}

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

function renderSidebar() {
  document.body.classList.toggle('sidebar-collapsed', state.sidebarCollapsed);
  const v = state.view;
  const isPage = (page: Page) => v.kind === 'page' && v.page === page;
  const top = h(
    'div',
    { class: 'sidebar-top' },
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
      navItem('plus', 'New agent', false, () => void editAgent(null)),
      navItem('apps', 'All agents', isPage('agents'), () => navigate({ kind: 'page', page: 'agents' })),
      navItem('clock', 'Scheduled', isPage('scheduled'), () => navigate({ kind: 'page', page: 'scheduled' }))
    );
    const scroll = h('div', { class: 'sidebar-scroll' }, nav, renderAgents());
    clear(sidebar, top, modes, scroll, renderFooter());
    return;
  }
  nav.append(navItem('plus', state.mode === 'code' ? 'New session' : 'New chat', v.kind === 'home', newChat));
  if (state.mode === 'work') {
    nav.append(navItem('apps', 'Apps', isPage('apps'), () => navigate({ kind: 'page', page: 'apps' }), h('span', { class: 'badge' }, 'Preview')));
  }
  const contextActive = CONTEXT_PAGES.some((c) => isPage(c.page));
  nav.append(navItem('context', 'Context', contextActive && !state.contextOpen, () => { state.contextOpen = !state.contextOpen; changed(); }));
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
  return h(
    'div',
    { class: `conv-link${active ? ' active' : ''}${nested ? ' nested' : ''}`, on: { click: () => navigate({ kind: 'chat', id }) }, title },
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

function renderConversations(): HTMLElement {
  const code = state.mode === 'code';
  const section = h('div', { class: 'sidebar-section' }, sectionHeading(code ? 'Sessions' : 'Conversations'));
  const loose = state.conversations.filter((c) => !c.archived && !c.projectId && !c.agentId && Boolean(c.workdir) === code);
  if (!loose.length) section.append(h('p', { class: 'sidebar-empty' }, code ? 'No sessions yet' : 'No conversations yet'));
  for (const c of loose) section.append(conversationLink(c.id, c.title, c.pinned));
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
        h('div', { class: 'menu-sep' }),
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

function openSearch() {
  void modal((close) => {
    const input = h('input', { class: 'input', placeholder: 'Search conversations, or #tag' });
    const results = h('div', { class: 'search-results' });
    const render = () => {
      const q = input.value.toLowerCase().trim();
      // `#tag` matches tags only; plain words match titles and tags.
      const tagQuery = q.startsWith('#') ? q.slice(1) : null;
      const found = state.conversations
        .filter((c) =>
          !q ? true : tagQuery !== null ? c.tags.some((t) => t.startsWith(tagQuery)) : c.title.toLowerCase().includes(q) || c.tags.some((t) => t.includes(q))
        )
        .slice(0, 30);
      clear(
        results,
        found.length
          ? found.map((c) =>
              h(
                'button',
                { class: 'menu-item', on: { click: () => { close(); navigate({ kind: 'chat', id: c.id }); } } },
                icon(c.archived ? 'archive' : 'chat', 16),
                h('span', {}, c.title),
                c.tags.length ? h('span', { class: 'tag-list' }, c.tags.map((t) => h('span', { class: 'tag' }, `#${t}`))) : null
              )
            )
          : h('p', { class: 'muted' }, 'Nothing found.')
      );
    };
    input.addEventListener('input', render);
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') (results.querySelector('button') as HTMLButtonElement | null)?.click();
    });
    render();
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
  applyTheme(storedTheme());
  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => applyTheme(storedTheme()));
  try {
    state.workdir = localStorage.getItem('workdir');
  } catch {
    /* per-device convenience only */
  }

  document.body.append(h('div', { class: 'app' }, sidebar, main));
  subscribe(() => {
    renderSidebar();
    renderMain();
  });

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
      void refreshConversations();
      const chat = viewCache.chat;
      if (chat && chat.key === `chat:${id}`) {
        void chat.finished().then(() => {
          if (!failed) state.live.delete(id);
          chat.renderTranscript();
          chat.update();
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
  // The app menu's Settings item (Ctrl+,) and the tray route here.
  void listen('chat://open-settings', () => {
    if (!document.querySelector('.modal .settings')) void openSettings();
  });
  void listen<string>('chat://quick-ask', (query) => {
    navigate({ kind: 'home' });
    composer.setText(query);
    void composer.submit();
  });

  document.addEventListener('keydown', (e) => {
    const mod = e.ctrlKey || e.metaKey;
    if (mod && e.key === ',') {
      e.preventDefault();
      if (!document.querySelector('.modal .settings')) void openSettings();
    } else if (mod && e.key.toLowerCase() === 'k') {
      e.preventDefault();
      openSearch();
    } else if (mod && e.shiftKey && e.key.toLowerCase() === 'o') {
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
  changed();
  if (!state.models.length) void openSettings('providers');
}

void start();
