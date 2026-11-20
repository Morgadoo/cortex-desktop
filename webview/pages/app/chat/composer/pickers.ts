/** The composer's pickers: the model menu, a folder to grant, and the project a chat belongs to. */
import { api } from '../../core/api';
import { clear } from '../../ui/h';
import { modal, popover, toast } from '../../ui/overlay';
import { openSettings } from '../../settings/index';
import { openCompareDialog, refreshConversations } from '../send';
import { h } from '../../ui/h';
import { icon } from '../../ui/icons';
import { state } from '../../core/state';
import { composer } from './index';

export async function grantFolder() {
  const dir = await api.pickDirectory();
  if (!dir) return;
  const settings = await api.appSettings();
  if (!settings.filesystem_allowed_roots.includes(dir)) settings.filesystem_allowed_roots.push(dir);
  await api.saveAppSettings(settings);
  toast(`The assistant can now read ${dir}. Allow writing in Settings → Permissions.`);
}

export async function moveToProject(conversationId: string | null) {
  const anchorProjects = state.projects;
  if (!conversationId) {
    toast('Send a message first, then move the conversation.');
    return;
  }
  const choices = h('div', { class: 'menu' });
  let resolveDone: () => void = () => {};
  const done = new Promise<void>((r) => (resolveDone = r));
  void modal((close) => {
    const pick = (projectId: string | null) => async () => {
      await api.updateConversation({ id: conversationId, projectId });
      await refreshConversations();
      close();
      resolveDone();
    };
    anchorProjects.forEach((p) => choices.append(h('button', { class: 'menu-item', on: { click: pick(p.id) } }, icon('folder', 17), p.name)));
    choices.append(h('button', { class: 'menu-item', on: { click: pick(null) } }, icon('close', 17), 'No project'));
    return h('div', { class: 'dialog-small' }, h('h2', {}, 'Move to project'), choices);
  });
  await done;
}

export function openModelPicker(anchor: HTMLElement, onPick: () => void) {
  const search = h('input', { class: 'input small', placeholder: 'Search models' });
  const list = h('div', { class: 'model-list' });
  const render = () => {
    const q = search.value.toLowerCase();
    const groups = new Map<string, typeof state.models>();
    for (const m of state.models) {
      if (q && !`${m.name} ${m.id} ${m.providerLabel}`.toLowerCase().includes(q)) continue;
      const group = groups.get(m.providerLabel) ?? [];
      group.push(m);
      groups.set(m.providerLabel, group);
    }
    clear(list);
    for (const [label, models] of groups) {
      list.append(h('div', { class: 'menu-heading' }, label));
      for (const m of models) {
        list.append(
          h(
            'button',
            {
              class: `menu-item model-item${m.reference === state.model ? ' selected' : ''}`,
              on: {
                click: () => {
                  state.model = m.reference;
                  void api.setDefaultModel(m.reference);
                  close();
                  onPick();
                },
              },
            },
            h('span', { class: 'model-name' }, m.name),
            m.reasoning ? h('span', { class: 'tag' }, 'reasoning') : null,
            m.reference === state.model ? icon('check', 15) : null
          )
        );
      }
    }
    if (!groups.size) list.append(h('div', { class: 'empty-small' }, 'No model matches.'));
  };
  search.addEventListener('input', render);
  render();
  const close = popover(
    anchor,
    h(
      'div',
      { class: 'menu model-menu' },
      search,
      list,
      h('div', { class: 'menu-sep' }),
      state.view.kind === 'home' && state.models.length > 1
        ? h(
            'button',
            {
              class: 'menu-item',
              on: {
                click: () => {
                  close();
                  openCompareDialog(() => {
                    onPick();
                    composer.renderChips();
                  });
                },
              },
            },
            icon('apps', 16),
            'Compare models…'
          )
        : null,
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
        icon('settings', 16),
        'Manage providers'
      )
    ),
    'above'
  );
  setTimeout(() => search.focus());
}
