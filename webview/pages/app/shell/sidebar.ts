/** The sidebar: navigation, context pages, agents, conversation links and the app menu. */
import { popover } from '../ui/overlay';
import { openInstructions } from '../pages/instructions';
import { agentName, avatar, newAgent, orgOrder } from '../pages/agents';
import { back, forward, type Page } from '../core/state';
import { call, shown } from '../core/call';
import { openWhatsNew, renderFooter } from './footer';
import { renderConversations } from './chat-list';
import { renderProjects } from './projects';
import { newChat, sidebar } from './main';
import { openSearchMode } from '../chat/searchmode';
import { conversationMenu } from '../chat/conversation-menu';
import { clear, h } from '../ui/h';
import { icon } from '../ui/icons';
import { openSettings } from '../settings/index';
import { changed, navigate, state } from '../core/state';
import { openSearch } from './search';

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
  { page: 'resources', label: 'Resources' },
];

/** The app menu at the top left (Vibe's hamburger menu): the app's own actions in one place. */
function openAppMenu(anchor: HTMLElement) {
  const item = (ic: Parameters<typeof icon>[0], label: string, keys: string | null, run: () => void) =>
    h(
      'button',
      {
        class: 'menu-item',
        on: {
          click: () => {
            close();
            run();
          },
        },
      },
      icon(ic, 16),
      h('span', { class: 'grow' }, label),
      keys ? h('small', { class: 'muted' }, keys) : null
    );
  const action = (name: string) => () => void call('app_menu_action', { action: name }).catch(shown);
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
      item('user', 'About Cortex', null, action('about')),
      h('div', { class: 'menu-sep' }),
      item('close', 'Quit', null, action('quit'))
    ),
    'below'
  );
}

export function renderSidebar() {
  document.body.classList.toggle('sidebar-collapsed', state.sidebarCollapsed);
  const v = state.view;
  const isPage = (page: Page) => v.kind === 'page' && v.page === page;
  const top = h(
    'div',
    { class: 'sidebar-top' },
    h('button', { class: 'icon-btn app-menu-btn', title: 'Menu', on: { click: (e: MouseEvent) => openAppMenu(e.currentTarget as HTMLElement) } }, icon('menu', 18)),
    h(
      'button',
      {
        class: 'icon-btn',
        title: 'Toggle sidebar',
        on: {
          click: () => {
            state.sidebarCollapsed = !state.sidebarCollapsed;
            changed();
          },
        },
      },
      icon('sidebar', 18)
    ),
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
        {
          class: state.mode === mode ? 'active' : '',
          on: {
            click: () => {
              state.mode = mode;
              navigate(mode === 'agents' ? { kind: 'page', page: 'agents' } : { kind: 'home' });
            },
          },
        },
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
  nav.append(
    navItem('context', 'Context', contextActive && !state.contextOpen, () => {
      state.contextOpen = !state.contextOpen;
      changed();
    })
  );
  // Code mode's Extensions, as in Vibe: the tools a session can plug in (MCP connectors).
  if (state.mode === 'code') nav.append(navItem('apps', 'Extensions', v.kind === 'page' && v.page === 'connectors', () => navigate({ kind: 'page', page: 'connectors' })));
  if (state.contextOpen || contextActive) {
    const sub = h('div', { class: 'subnav' });
    for (const c of CONTEXT_PAGES) {
      sub.append(
        h(
          'button',
          { class: `nav-link sub${isPage(c.page) ? ' active' : ''}`, on: { click: () => navigate({ kind: 'page', page: c.page }) } },
          h('span', { class: 'nav-label' }, c.label),
          c.preview ? h('span', { class: 'badge' }, 'Preview') : null
        )
      );
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
  for (const agent of orgOrder(state.agents)) {
    const active = (state.view.kind === 'agent' && state.view.id === agent.id) || (state.view.kind === 'home' && state.view.agentId === agent.id);
    const runs = state.conversations.filter((c) => c.agentId === agent.id);
    const unread = runs.some((c) => c.unread);
    section.append(
      h(
        'div',
        { class: `conv-link project${active ? ' active' : ''}`, on: { click: () => navigate({ kind: 'agent', id: agent.id }) } },
        avatar(agent),
        h('span', { class: 'conv-title' }, ...agentName(agent)),
        unread ? h('span', { class: 'unread-dot' }) : null
      )
    );
    for (const c of runs.slice(0, 4)) section.append(conversationLink(c.id, c.title, c.pinned, true));
  }
  return section;
}

export function switchMode(mode: 'work' | 'code' | 'agents') {
  state.mode = mode;
  navigate(mode === 'agents' ? { kind: 'page', page: 'agents' } : { kind: 'home' });
}

export function sectionHeading(label: string, action?: HTMLElement): HTMLElement {
  return h('div', { class: 'section-heading' }, h('span', {}, label), action ?? null);
}

export function conversationLink(id: string, title: string, pinned: boolean, nested = false): HTMLElement {
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
