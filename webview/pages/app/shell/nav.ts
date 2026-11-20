/** What the shell's parts share: the sidebar element, starting a new chat, switching mode, and the sidebar's links and headings. */
import { composer } from '../chat/composer/index';
import { conversationMenu } from '../chat/conversation-menu';
import { h } from '../ui/h';
import { icon } from '../ui/icons';
import { navigate, state } from '../core/state';

export const sidebar = h('aside', { class: 'sidebar' });

export function newChat() {
  navigate({ kind: 'home' });
  composer.setText('');
  setTimeout(() => composer.focus());
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
