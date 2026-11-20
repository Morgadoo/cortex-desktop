/** A conversation's menu, and choosing messages to act on. */
import { openChatBeside } from './side-panels';
import { exportImage, printConversation } from './turn-actions';
import { refreshConversations } from './send';
import { moveToProject } from './composer/pickers';
import { api } from '../core/api';
import { h } from '../ui/h';
import { icon } from '../ui/icons';
import { confirmDialog, popover, promptText, toast } from '../ui/overlay';
import { navigate, state } from '../core/state';
import { call, reported, shown } from '../core/call';
import { openChatSettings } from './chat-settings';
import { composer } from './composer/current';
import { viewCache } from './views';

export function conversationMenu(anchor: HTMLElement, id: string, title: string, pinned: boolean) {
  const item = (ic: string, label: string, action: () => void, danger = false) =>
    h(
      'button',
      {
        class: `menu-item${danger ? ' danger' : ''}`,
        on: {
          click: () => {
            close();
            action();
          },
        },
      },
      icon(ic, 16),
      label
    );
  const close = popover(
    anchor,
    h(
      'div',
      { class: 'menu' },
      item('edit', 'Rename', async () => {
        const next = await promptText('Rename conversation', title);
        if (next) {
          await api.updateConversation({ id, title: next });
          await refreshConversations();
        }
      }),
      item('pin', pinned ? 'Unpin' : 'Pin', async () => {
        await api.updateConversation({ id, pinned: !pinned });
        await refreshConversations();
      }),
      item('folder', 'Move to project', () => void moveToProject(id)),
      item('branch', 'Branch into a new chat', async () => {
        const branch = await api.forkConversation(id);
        await refreshConversations();
        navigate({ kind: 'chat', id: branch });
        toast('Branched. This chat continues from the same point.');
      }),
      ...(state.agents.length
        ? [
            item('sparkle', 'Continue with an agent…', () => {
              // Hand the conversation to an agent (LobeHub's forward): a copy it carries on.
              const close = popover(
                anchor,
                h(
                  'div',
                  { class: 'menu' },
                  h('div', { class: 'menu-heading' }, 'Continue with'),
                  state.agents.map((a) =>
                    h(
                      'button',
                      {
                        class: 'menu-item',
                        on: {
                          click: async () => {
                            close();
                            try {
                              const next = await reported(api.forkConversation(id, null, false, a.id));
                              await reported(refreshConversations());
                              state.mode = 'agents';
                              navigate({ kind: 'chat', id: next });
                              toast(`${a.name} has the conversation now.`);
                            } catch {
                              // Already shown.
                            }
                          },
                        },
                      },
                      h('span', { class: 'agent-avatar small' }, a.avatar || a.name.charAt(0)),
                      h('span', {}, a.name)
                    )
                  )
                )
              );
            }),
          ]
        : []),
      item('check', 'Select messages', () => {
        if (state.view.kind !== 'chat' || state.view.id !== id) navigate({ kind: 'chat', id });
        setTimeout(() => selectMessages(id), state.view.kind === 'chat' && state.view.id === id ? 0 : 400);
      }),
      item('download', 'Export as Markdown', () => void api.exportConversation(id).then((saved) => saved && toast('Conversation exported.'))),
      item('link', 'Copy link', () => void navigator.clipboard.writeText(`cortex://chat/${id}`).then(() => toast('Link copied. It opens this chat in Cortex on this computer.'))),
      ...(state.view.kind === 'chat' && state.view.id !== id ? [item('columns', 'Open beside this chat', () => openChatBeside(id))] : []),
      item('expand', 'Open in new window', () => void call('chat_open_chat_window', { id }).catch(shown)),
      item('download', 'Print or save as PDF', () => void printConversation(id, title)),
      item('image', 'Export as image', () => void exportImage(id)),
      item(
        'code',
        'Export as JSON',
        () =>
          void call<boolean>('chat_export_conversation_json', { id })
            .then((saved) => saved && toast('Conversation exported.'))
            .catch(shown)
      ),
      item('settings', 'Chat settings…', () => void openChatSettings(id)),
      item('dot', 'Mark as unread', async () => {
        await api.updateConversation({ id, unread: true });
        await refreshConversations();
        if (state.view.kind === 'chat' && state.view.id === id) navigate({ kind: 'home' });
      }),
      item('tag', 'Tags…', async () => {
        const current = state.conversations.find((c) => c.id === id)?.tags ?? [];
        const next = await promptText('Tags (comma separated)', current.join(', '));
        if (next === null || next === undefined) return;
        await api.updateConversation({ id, tags: next.split(',') });
        await refreshConversations();
      }),
      item('archive', state.conversations.find((c) => c.id === id)?.archived ? 'Unarchive' : 'Archive', async () => {
        const archived = !state.conversations.find((c) => c.id === id)?.archived;
        await api.updateConversation({ id, archived });
        if (archived && state.view.kind === 'chat' && state.view.id === id) navigate({ kind: 'home' });
        await refreshConversations();
        toast(archived ? 'Archived. Find it under Archived at the end of the list.' : 'Back in your conversations.');
      }),
      h('div', { class: 'menu-sep' }),
      item(
        'trash',
        'Delete',
        async () => {
          if (await confirmDialog('Delete conversation?', `“${title}” moves to Recently deleted, where it stays for 30 days (Settings → Profile).`)) {
            await api.deleteConversation(id);
            if (state.view.kind === 'chat' && state.view.id === id) navigate({ kind: 'home' });
            await refreshConversations();
          }
        },
        true
      )
    )
  );
}

/**
 * Select several messages (LobeHub's multi-select): tick them, then copy them, forward them to a
 * new chat or an agent as an attachment, or delete them with their replies. Esc leaves.
 */
export function selectMessages(conversationId: string) {
  const view = viewCache.chat;
  const transcript = view?.el.querySelector<HTMLElement>('.transcript');
  if (!view || view.key !== `chat:${conversationId}` || !transcript || transcript.classList.contains('selecting')) return;
  transcript.classList.add('selecting');
  const turns = [...transcript.querySelectorAll<HTMLElement>('.turn[data-index]')].filter((t) => Number(t.dataset.index) >= 0);
  const selected = new Set<HTMLElement>();
  const count = h('span', { class: 'select-count' });
  const textOf = (turn: HTMLElement) => {
    const user = turn.classList.contains('turn-user');
    const body = user
      ? [turn.querySelector<HTMLElement>('.bubble-text')?.innerText ?? '', ...[...turn.querySelectorAll<HTMLElement>('.bubble-file summary span')].map((f) => `[Attached: ${f.innerText}]`)].filter(Boolean).join('\n')
      : [...turn.querySelectorAll<HTMLElement>('.markdown')]
          .filter((m) => !m.closest('details, .activity') && !m.parentElement?.closest('.markdown'))
          .map((m) => m.innerText)
          .join('\n\n');
    return `${user ? 'You' : 'Assistant'}:\n${body.trim()}`;
  };
  const chosen = () => turns.filter((t) => selected.has(t));
  const transcriptText = () => chosen().map(textOf).join('\n\n---\n\n');
  const buttons: HTMLButtonElement[] = [];
  const update = () => {
    count.textContent = selected.size ? `${selected.size} selected` : 'Tick the messages';
    buttons.forEach((b) => (b.disabled = !selected.size));
  };
  const toggles: [HTMLElement, (e: MouseEvent) => void][] = [];
  const checks = turns.map((turn) => {
    const box = h('input', { type: 'checkbox', class: 'turn-check', title: 'Select' });
    box.addEventListener('change', () => {
      if (box.checked) selected.add(turn);
      else selected.delete(turn);
      turn.classList.toggle('selected', box.checked);
      update();
    });
    turn.prepend(box);
    // A click anywhere on the message ticks it, except on its links and buttons.
    const toggle = (e: MouseEvent) => {
      if (e.target === box || (e.target as HTMLElement).closest('a, button, input, summary, select, textarea')) return;
      box.checked = !box.checked;
      box.dispatchEvent(new Event('change'));
    };
    turn.addEventListener('click', toggle);
    toggles.push([turn, toggle]);
    return box;
  });
  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Escape') exit();
  };
  const exit = () => {
    checks.forEach((c) => c.remove());
    toggles.forEach(([t, f]) => t.removeEventListener('click', f));
    turns.forEach((t) => t.classList.remove('selected'));
    transcript.classList.remove('selecting');
    bar.remove();
    document.removeEventListener('keydown', onKey);
  };
  const action = (ic: string, label: string, run: (b: HTMLButtonElement) => void, danger = false) => {
    const b = h('button', { class: `btn btn-sm ${danger ? 'btn-danger' : 'btn-secondary'}` }, icon(ic, 14), label);
    b.addEventListener('click', () => run(b));
    buttons.push(b);
    return b;
  };
  const forwardTo = (agentId: string | null) => {
    const text = transcriptText();
    const n = selected.size;
    exit();
    if (agentId) state.mode = 'agents';
    navigate({ kind: 'home', agentId: agentId ?? undefined });
    setTimeout(() => {
      composer.attachText(`Forwarded messages (${n})`, text);
      composer.focus();
    }, 50);
    toast(agentId ? `Forwarded to ${state.agents.find((a) => a.id === agentId)?.name ?? 'the agent'}.` : 'Forwarded to a new chat.');
  };
  const all = h('button', { class: 'link-btn' }, 'All');
  all.addEventListener('click', () => {
    const on = selected.size < turns.length;
    checks.forEach((c) => {
      c.checked = on;
      c.dispatchEvent(new Event('change'));
    });
  });
  const bar = h(
    'div',
    { class: 'select-bar' },
    count,
    all,
    h('span', { class: 'spacer' }),
    action('copy', 'Copy', () => void navigator.clipboard.writeText(transcriptText()).then(() => toast('Copied.'))),
    action('share', 'Forward', (b) => {
      const close = popover(
        b,
        h(
          'div',
          { class: 'menu' },
          h('div', { class: 'menu-heading' }, 'Forward to'),
          h(
            'button',
            {
              class: 'menu-item',
              on: {
                click: () => {
                  close();
                  forwardTo(null);
                },
              },
            },
            icon('plus', 16),
            h('span', {}, 'A new chat')
          ),
          ...state.agents.map((a) =>
            h(
              'button',
              {
                class: 'menu-item',
                on: {
                  click: () => {
                    close();
                    forwardTo(a.id);
                  },
                },
              },
              icon('sparkle', 16),
              h('span', {}, a.name)
            )
          )
        ),
        'above'
      );
    }),
    action(
      'trash',
      'Delete',
      async () => {
        const indices = chosen().map((t) => Number(t.dataset.index));
        if (!(await confirmDialog(`Delete ${indices.length === 1 ? 'this message' : `${indices.length} messages`}?`, 'Each message goes with the question or answer it belongs to, and any other versions of them.', 'Delete'))) return;
        try {
          const n = await call<number>('chat_delete_exchanges', { id: conversationId, indices });
          exit();
          await reported(viewCache.chat?.refresh(false));
          toast(`Deleted ${n} ${n === 1 ? 'exchange' : 'exchanges'}.`);
        } catch {
          // Already shown.
        }
      },
      true
    ),
    h('button', { class: 'btn btn-sm btn-ghost', on: { click: () => exit() } }, 'Done')
  );
  document.addEventListener('keydown', onKey);
  view.el.append(bar);
  update();
}
