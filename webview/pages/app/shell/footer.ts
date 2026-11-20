/** The sidebar's footer: the account menu, what is new, and the way to usage and settings. */
import { api } from '../core/api';
import { clear, h } from '../ui/h';
import { icon } from '../ui/icons';
import { modal, popover } from '../ui/overlay';
import { openSettings } from '../settings/index';
import { setTheme, storedTheme } from '../theme/theme';
import { state } from '../core/state';
import { modalHeader } from '../ui/ui';
import { openUsageBreakdown } from './usage';

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
    items: ['Commands that are hard to undo always ask, even in Full auto.', 'Deleted chats stay 30 days in Recently deleted; daily copies protect against damage.', 'Settings → Audio: choose speech-to-text and text-to-speech providers.'],
  },
];

export function openWhatsNew() {
  void modal((close) =>
    h(
      'div',
      { class: 'dialog-small wide whats-new' },
      modalHeader(`What's new in Cortex ${window.lumoDesktopVersion ?? ''}`, close, { flush: true }),
      WHATS_NEW.map((section) =>
        h(
          'section',
          {},
          h('h4', {}, section.title),
          h(
            'ul',
            {},
            section.items.map((item) => h('li', {}, item))
          )
        )
      )
    )
  );
}

export function renderFooter(): HTMLElement {
  const name = state.instructions.name || 'You';
  const initials =
    name
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((w) => w[0]!.toUpperCase())
      .join('') || 'U';
  const model = state.models.find((m) => m.reference === state.model);
  const user = h('button', { class: 'user-btn' }, h('span', { class: 'avatar' }, initials), h('span', { class: 'user-text' }, h('strong', {}, name), h('small', {}, model ? model.providerLabel : 'No provider yet')));
  user.addEventListener('click', () => {
    const theme = storedTheme();
    // This month's usage across every conversation, as Vibe's footer gauge.
    const usage = h(
      'button',
      {
        class: 'usage-gauge',
        title: 'Usage by model and by agent',
        on: {
          click: () => {
            close();
            openUsageBreakdown();
          },
        },
      },
      h('span', { class: 'muted' }, 'This month'),
      h('span', { class: 'dots' })
    );
    void api
      .usageMonth()
      .then((u) => {
        const tokens = u.tokens >= 1_000_000 ? `${(u.tokens / 1_000_000).toFixed(1)}M` : `${Math.round(u.tokens / 1000)}k`;
        clear(usage, h('span', { class: 'muted' }, 'This month'), h('span', {}, `${tokens} tokens${u.cost > 0 ? ` · $${u.cost.toFixed(2)}` : ''} · ${u.runs} chats`));
      })
      .catch(() => clear(usage));
    const themeBtn = (t: 'light' | 'dark' | 'system', ic: string) =>
      h(
        'button',
        {
          class: `icon-btn${theme === t ? ' active' : ''}`,
          title: t,
          on: {
            click: () => {
              setTheme(t);
              close();
            },
          },
        },
        icon(ic, 17)
      );
    const close = popover(
      user,
      h(
        'div',
        { class: 'menu user-menu' },
        h('div', { class: 'user-head' }, h('span', { class: 'avatar' }, initials), h('div', {}, h('strong', {}, name), h('small', { class: 'muted' }, 'Local profile'))),
        h('div', { class: 'menu-sep' }),
        h(
          'button',
          {
            class: 'menu-item',
            on: {
              click: () => {
                close();
                void openSettings('profile');
              },
            },
          },
          icon('settings', 16),
          h('span', {}, 'Settings'),
          h('kbd', {}, 'Ctrl+,')
        ),
        h(
          'button',
          {
            class: 'menu-item',
            on: {
              click: () => {
                close();
                void openSettings('providers');
              },
            },
          },
          icon('key', 16),
          'Providers & models'
        ),
        h(
          'button',
          {
            class: 'menu-item',
            on: {
              click: () => {
                close();
                void openSettings('permissions');
              },
            },
          },
          icon('shield', 16),
          'Permissions'
        ),
        h(
          'button',
          {
            class: 'menu-item',
            on: {
              click: () => {
                close();
                openWhatsNew();
              },
            },
          },
          icon('sparkle', 16),
          "What's new"
        ),
        h('div', { class: 'menu-sep' }),
        usage,
        h('div', { class: 'theme-row' }, themeBtn('light', 'sun'), themeBtn('dark', 'moon'), themeBtn('system', 'monitor')),
        h('div', { class: 'menu-foot muted' }, `Cortex ${window.lumoDesktopVersion ?? ''} · open source, GPL-3.0`)
      ),
      'above'
    );
  });
  return h('div', { class: 'sidebar-footer' }, user);
}
