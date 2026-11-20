/**
 * All chats in one place (LobeHub's topic panel): status tabs (all, unread, pinned, scheduled
 * runs, archived), a search, and a list or a card view, with the actions each chat needs.
 */
import { api, type ConversationSummary } from './core/api';
import { clear, h, icon, timeLabel } from './ui/dom';
import { navigate, state, changed } from './core/state';
import { pageHeader } from './ui/ui';
import { choice, textPref } from './core/prefs';

type Status = 'all' | 'unread' | 'pinned' | 'runs' | 'archived';
type Layout = 'list' | 'cards';
const STATUSES: [Status, string][] = [
  ['all', 'All'],
  ['unread', 'Unread'],
  ['pinned', 'Pinned'],
  ['runs', 'Scheduled runs'],
  ['archived', 'Archived'],
];

function stored<T extends string>(key: string, fallback: T, allowed: readonly T[]): T {
  return choice(key, allowed, fallback).get();
}

function keep(key: string, value: string) {
  textPref(key).set(value);
}

/** The chats a status tab shows. Pure, for the check script. */
export function inStatus(c: Pick<ConversationSummary, 'archived' | 'unread' | 'pinned' | 'taskId'>, status: Status): boolean {
  if (status === 'archived') return c.archived;
  if (c.archived) return false;
  if (status === 'unread') return c.unread;
  if (status === 'pinned') return c.pinned;
  if (status === 'runs') return Boolean(c.taskId);
  return true;
}

export class ChatsPage {
  readonly el = h('div', { class: 'page chats-page' });
  readonly key = 'page:chats';
  private status: Status = stored(
    'cortex.chatsTab',
    'all',
    STATUSES.map(([s]) => s)
  );
  private layout: Layout = stored('cortex.chatsLayout', 'list', ['list', 'cards'] as const);
  private query = '';
  private tabs = h('div', { class: 'tabs' });
  private views = h('div', { class: 'view-switch' });
  private body = h('div', {});

  constructor() {
    const search = h('input', { class: 'input', placeholder: 'Search chat titles and tags' });
    search.addEventListener('input', () => {
      this.query = search.value.trim().toLowerCase();
      this.update();
    });
    this.el.append(
      pageHeader('Chats', 'Every conversation, by status. Archived chats stay searchable here.'),
      h('div', { class: 'tasks-bar' }, this.tabs, this.views),
      h('div', { class: 'toolbar' }, h('div', { class: 'search-wrap' }, icon('search', 16), search)),
      this.body
    );
    this.update();
  }

  update() {
    const all = state.conversations;
    clear(
      this.tabs,
      STATUSES.map(([s, label]) => {
        const n = all.filter((c) => inStatus(c, s)).length;
        return h(
          'button',
          {
            class: `tab${this.status === s ? ' active' : ''}`,
            on: {
              click: () => {
                this.status = s;
                keep('cortex.chatsTab', s);
                this.update();
              },
            },
          },
          label,
          h('span', { class: 'tab-count' }, String(n))
        );
      })
    );
    clear(
      this.views,
      (
        [
          ['list', 'list', 'List'],
          ['cards', 'grid', 'Cards'],
        ] as [Layout, string, string][]
      ).map(([l, ic, title]) =>
        h(
          'button',
          {
            class: `icon-btn${this.layout === l ? ' active' : ''}`,
            title,
            'aria-pressed': String(this.layout === l),
            on: {
              click: () => {
                this.layout = l;
                keep('cortex.chatsLayout', l);
                this.update();
              },
            },
          },
          icon(ic, 16)
        )
      )
    );
    const shown = all
      .filter((c) => inStatus(c, this.status))
      .filter((c) => !this.query || c.title.toLowerCase().includes(this.query) || c.tags.some((t) => t.toLowerCase().includes(this.query)))
      .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
    if (!shown.length) return void clear(this.body, h('p', { class: 'muted empty-line' }, this.query ? 'No chats match.' : 'Nothing here.'));
    const agentOf = (c: ConversationSummary) => state.agents.find((a) => a.id === c.agentId);
    const where = (c: ConversationSummary) => agentOf(c)?.name ?? state.projects.find((p) => p.id === c.projectId)?.name ?? (c.workdir ? c.workdir.split(/[\\/]/).pop() : '') ?? '';
    const archiveBtn = (c: ConversationSummary) =>
      h(
        'button',
        {
          class: 'icon-btn small',
          title: c.archived ? 'Restore from the archive' : 'Archive',
          on: {
            click: async (e: MouseEvent) => {
              e.stopPropagation();
              await api.updateConversation({ id: c.id, archived: !c.archived });
              state.conversations = await api.conversations();
              changed();
            },
          },
        },
        icon('archive', 15)
      );
    if (this.layout === 'cards') {
      clear(
        this.body,
        h(
          'div',
          { class: 'chat-cards' },
          shown.map((c) =>
            h(
              'button',
              { class: 'chat-card', on: { click: () => navigate({ kind: 'chat', id: c.id }) } },
              h('div', { class: 'chat-card-head' }, c.unread ? h('span', { class: 'unread-dot' }) : null, c.pinned ? icon('pin', 13) : null, h('strong', { class: 'grow' }, c.title || 'Untitled'), archiveBtn(c)),
              h('p', { class: 'chat-card-preview' }, c.preview || ''),
              h(
                'div',
                { class: 'chat-card-foot muted' },
                where(c) ? h('span', {}, where(c)) : null,
                c.tags.map((t) => h('span', { class: 'tag' }, `#${t}`)),
                h('span', { class: 'spacer' }),
                h('span', {}, timeLabel(c.updatedAt))
              )
            )
          )
        )
      );
      return;
    }
    clear(
      this.body,
      h(
        'div',
        { class: 'run-list' },
        shown.map((c) =>
          h(
            'button',
            { class: 'run-row', on: { click: () => navigate({ kind: 'chat', id: c.id }) } },
            c.unread ? h('span', { class: 'unread-dot' }) : icon(c.taskId ? 'clock' : c.pinned ? 'pin' : 'chat', 15),
            h('span', { class: 'run-title' }, c.title || 'Untitled'),
            where(c) ? h('span', { class: 'muted' }, where(c)) : null,
            h('span', { class: 'muted' }, timeLabel(c.updatedAt)),
            archiveBtn(c)
          )
        )
      )
    );
  }
}
