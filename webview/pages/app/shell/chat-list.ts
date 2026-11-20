/** The sidebar's chat list: sorted, filtered and grouped by date, with the archive. */
import { api } from '../core/api';
import { refreshConversations } from '../chat/send';
import { clear, h } from '../ui/h';
import { icon } from '../ui/icons';
import { modal, popover } from '../ui/overlay';
import { avatar, pinnedAgents } from '../pages/agents';
import { changed, navigate, state } from '../core/state';
import { button, emptyState, iconButton, modalHeader } from '../ui/ui';
import { choice, json } from '../core/prefs';
import { conversationLink, sectionHeading } from './sidebar';

type ChatSort = 'recent' | 'created' | 'title' | 'unread';
const CHAT_SORTS: Record<ChatSort, string> = { recent: 'Recent activity', created: 'Newest first', title: 'A–Z', unread: 'Unread first' };
const chatSortPref = choice<ChatSort>('cortex.chatSort', Object.keys(CHAT_SORTS) as ChatSort[], 'recent');
let chatSort: ChatSort = chatSortPref.get();

function checkMark(on: boolean): SVGElement {
  const mark = icon('check', 15);
  if (!on) mark.style.visibility = 'hidden';
  return mark;
}

function setChatSort(sort: ChatSort) {
  chatSort = sort;
  chatSortPref.set(sort);
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
const chatFilterPref = json<ChatFilter>('cortex.chatFilter', () => ({ ...NO_FILTER }));
let chatFilter: ChatFilter = chatFilterPref.get();

function setChatFilter(change: Partial<ChatFilter>) {
  chatFilter = { ...chatFilter, ...change };
  chatFilterPref.set(chatFilter);
  changed();
}

const filtering = () => chatFilter.unread || chatFilter.tag !== null || chatFilter.within !== 0 || chatFilter.hideRuns;

function filterChats<T extends { unread: boolean; tags: string[]; updatedAt: string; taskId: string | null }>(list: T[]): T[] {
  const since = chatFilter.within ? Date.now() - chatFilter.within * 86_400_000 : 0;
  return list.filter((c) => (!chatFilter.unread || c.unread) && (!chatFilter.tag || c.tags.includes(chatFilter.tag)) && (!since || Date.parse(c.updatedAt) >= since) && (!chatFilter.hideRuns || !c.taskId));
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

export function renderConversations(): HTMLElement {
  const code = state.mode === 'code';
  const section = h('div', { class: 'sidebar-section' });
  const all = state.conversations.filter((c) => !c.archived && !c.projectId && !c.agentId && Boolean(c.workdir) === code);
  const pinned = all.filter((c) => c.pinned);
  const loose = sortChats(filterChats(all.filter((c) => !c.pinned)));
  // Pinned agents (Work mode) start a new chat with the agent.
  const agents = code
    ? []
    : pinnedAgents()
        .map((id) => state.agents.find((a) => a.id === id))
        .filter((a): a is NonNullable<typeof a> => !!a);
  if (pinned.length || agents.length) {
    section.append(sectionHeading('Pinned'));
    for (const a of agents) {
      const active = state.view.kind === 'home' && state.view.agentId === a.id;
      section.append(
        h('div', { class: `conv-link project${active ? ' active' : ''}`, title: `New chat with ${a.name}`, on: { click: () => navigate({ kind: 'home', agentId: a.id }) } }, avatar(a), h('span', { class: 'conv-title' }, a.name))
      );
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
          h(
            'button',
            {
              class: `menu-item${chatSort === k ? ' selected' : ''}`,
              on: {
                click: () => {
                  close();
                  setChatSort(k);
                },
              },
            },
            checkMark(chatSort === k),
            h('span', {}, CHAT_SORTS[k])
          )
        ),
        h('div', { class: 'menu-sep' }),
        h('div', { class: 'menu-heading' }, 'Show'),
        h(
          'button',
          {
            class: 'menu-item',
            on: {
              click: () => {
                close();
                setChatFilter({ unread: !chatFilter.unread });
              },
            },
          },
          checkMark(chatFilter.unread),
          h('span', {}, 'Unread only')
        ),
        h(
          'button',
          {
            class: 'menu-item',
            on: {
              click: () => {
                close();
                setChatFilter({ hideRuns: !chatFilter.hideRuns });
              },
            },
          },
          checkMark(chatFilter.hideRuns),
          h('span', {}, 'Hide scheduled runs')
        ),
        (
          [
            [0, 'Any time'],
            [1, 'Last 24 hours'],
            [7, 'Last 7 days'],
            [30, 'Last 30 days'],
          ] as [ChatFilter['within'], string][]
        ).map(([days, label]) =>
          h(
            'button',
            {
              class: 'menu-item',
              on: {
                click: () => {
                  close();
                  setChatFilter({ within: days });
                },
              },
            },
            checkMark(chatFilter.within === days),
            h('span', {}, label)
          )
        ),
        tags.length ? h('div', { class: 'menu-heading' }, 'Tag') : null,
        tags.map((t) =>
          h(
            'button',
            {
              class: 'menu-item',
              on: {
                click: () => {
                  close();
                  setChatFilter({ tag: chatFilter.tag === t ? null : t });
                },
              },
            },
            checkMark(chatFilter.tag === t),
            h('span', {}, `#${t}`)
          )
        ),
        filtering() ? h('div', { class: 'menu-sep' }) : null,
        filtering()
          ? h(
              'button',
              {
                class: 'menu-item',
                on: {
                  click: () => {
                    close();
                    setChatFilter({ ...NO_FILTER });
                  },
                },
              },
              icon('close', 15),
              h('span', {}, 'Clear filters')
            )
          : null
      )
    );
  });
  const tags = [...new Set(all.flatMap((c) => c.tags))].sort();
  const allBtn = h('button', { class: 'icon-btn tiny', title: 'All chats, by status', on: { click: () => navigate({ kind: 'page', page: 'chats' }) } }, icon('grid', 14));
  section.append(sectionHeading(code ? 'Sessions' : 'Chats', h('span', { class: 'section-actions' }, allBtn, sortBtn)));
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
        {
          class: 'nav-link sub show-more',
          on: {
            click: () => {
              sidebarExpanded = !sidebarExpanded;
              changed();
            },
          },
        },
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
export function openArchived() {
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
                h(
                  'button',
                  {
                    class: 'grow archived-title',
                    on: {
                      click: () => {
                        close();
                        navigate({ kind: 'chat', id: c.id });
                      },
                    },
                  },
                  c.title
                ),
                button({
                  variant: 'subtle',
                  size: 'sm',
                  icon: 'reset',
                  label: 'Restore',
                  onClick: async () => {
                    await api.updateConversation({ id: c.id, archived: false });
                    await refreshConversations();
                    render();
                  },
                }),
                iconButton({
                  icon: 'trash',
                  title: 'Delete',
                  size: 'sm',
                  onClick: async () => {
                    await api.deleteConversation(c.id);
                    await refreshConversations();
                    render();
                  },
                })
              )
            )
          : emptyState('archive', 'Nothing archived', 'Archive a conversation from its menu to tidy the sidebar without deleting it.')
      );
    };
    render();
    return h('div', { class: 'dialog-small wide' }, modalHeader('Archived', close, { flush: true }), list);
  });
}
