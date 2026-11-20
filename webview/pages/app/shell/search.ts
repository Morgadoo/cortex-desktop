/** The search dialog (Ctrl+K): chats, messages and commands. */
import { openSearchMode } from '../chat/searchmode';
import { api } from '../core/api';
import { scrollToTurn } from '../chat/turn-actions';
import { clear, h } from '../ui/h';
import { icon } from '../ui/icons';
import { modal } from '../ui/overlay';
import { openSettings } from '../settings/index';
import { changed, navigate, state } from '../core/state';
import { newChat } from './main';
import { switchMode } from './sidebar';
import { openArchived } from './chat-list';
import { openWhatsNew } from './footer';

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

export function openSearch() {
  void modal((close) => {
    const input = h('input', { class: 'input', placeholder: 'Search chats and actions, or #tag' });
    const results = h('div', { class: 'search-results' });
    const row = (c: { id: string; title: string; archived: boolean; tags: string[]; updatedAt: string }, snippet: HTMLElement | null) =>
      h(
        'button',
        {
          class: `menu-item${snippet ? ' rich' : ''}`,
          on: {
            click: () => {
              close();
              navigate({ kind: 'chat', id: c.id });
            },
          },
        },
        icon(c.archived ? 'archive' : 'chat', 16),
        snippet ? h('span', {}, h('strong', {}, c.title), snippet) : h('span', {}, c.title),
        c.tags.length
          ? h(
              'span',
              { class: 'tag-list' },
              c.tags.map((t) => h('span', { class: 'tag' }, `#${t}`))
            )
          : null,
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
      {
        label: 'Toggle sidebar',
        icon: 'sidebar',
        keys: 'Ctrl+B',
        run: () => {
          state.sidebarCollapsed = !state.sidebarCollapsed;
          changed();
        },
      },
      { label: "What's new", icon: 'sparkle', run: openWhatsNew },
    ];
    const actionRows = (q: string) => {
      const found = actions.filter((a) => !q || a.label.toLowerCase().includes(q));
      if (!found.length) return [];
      return [
        h('div', { class: 'menu-heading' }, 'Actions'),
        ...found.slice(0, q ? 6 : 15).map((a) =>
          h(
            'button',
            {
              class: 'menu-item',
              on: {
                click: () => {
                  close();
                  a.run();
                },
              },
            },
            icon(a.icon, 16),
            h('span', {}, a.label),
            a.keys ? h('kbd', {}, a.keys) : null
          )
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
        ...pinned.map((c) =>
          h(
            'button',
            {
              class: 'menu-item',
              on: {
                click: () => {
                  close();
                  navigate({ kind: 'chat', id: c.id });
                },
              },
            },
            icon('pin', 16),
            h('span', {}, c.title)
          )
        ),
        ...agents.map((a) =>
          h(
            'button',
            {
              class: 'menu-item',
              on: {
                click: () => {
                  close();
                  state.mode = 'agents';
                  navigate({ kind: 'home', agentId: a.id });
                },
              },
            },
            h('span', { class: 'agent-avatar small' }, a.avatar || a.name.charAt(0)),
            h('span', {}, `Chat with ${a.name}`)
          )
        ),
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
            {
              class: 'menu-item rich',
              on: {
                click: () => {
                  close();
                  openAt(m.c.id, m.index);
                },
              },
            },
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
        clear(
          results,
          q ? null : pinnedRows(),
          q ? null : bookmarkRows(''),
          q ? null : actionRows(''),
          found.length ? [h('div', { class: 'menu-heading' }, 'Chats'), ...found.map((c) => row(c, null))] : h('p', { class: 'muted' }, 'Nothing found.')
        );
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
