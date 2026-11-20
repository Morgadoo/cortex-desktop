/** Projects in the sidebar: the list, a project's menu and its editor. */
import { api } from '../core/api';
import { tauriInvoke } from '../../../shared/tauri';
import { refreshConversations } from '../chat/send';
import { clear, h } from '../ui/h';
import { icon } from '../ui/icons';
import { modal, popover, promptText, confirmDialog, toast } from '../ui/overlay';
import { changed, navigate, state } from '../core/state';
import { modalHeader } from '../ui/ui';
import { conversationLink, sectionHeading } from './sidebar';

export function renderProjects(): HTMLElement {
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
      h(
        'div',
        { class: `conv-link project${active ? ' active' : ''}`, on: { click: () => navigate({ kind: 'home', projectId: project.id }) } },
        project.icon ? h('span', { class: 'project-icon' }, project.icon) : icon('folder', 16),
        h('span', { class: 'conv-title' }, project.name),
        more
      )
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
      h(
        'button',
        {
          class: 'menu-item',
          on: {
            click: async () => {
              close();
              const name = await promptText('Rename project', project.name);
              if (name) {
                state.projects = await api.saveProject({ ...project, name });
                changed();
              }
            },
          },
        },
        icon('edit', 16),
        'Rename'
      ),
      h(
        'button',
        {
          class: 'menu-item',
          on: {
            click: () => {
              close();
              void editProject(id);
            },
          },
        },
        icon('settings', 16),
        'Project settings'
      ),
      h('div', { class: 'menu-sep' }),
      h(
        'button',
        {
          class: 'menu-item danger',
          on: {
            click: async () => {
              close();
              if (await confirmDialog('Delete project?', `Its conversations stay, outside any project.`)) {
                state.projects = await api.deleteProject(id);
                await refreshConversations();
              }
            },
          },
        },
        icon('trash', 16),
        'Delete'
      )
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
      [
        ['', 'Same as my instructions'],
        ['formal', 'Polite'],
        ['casual', 'Casual'],
        ['empathetic', 'Empathetic'],
        ['objective', 'Objective'],
        ['concise', 'Concise'],
        ['witty', 'Witty'],
        ['motivational', 'Motivational'],
        ['reflective', 'Reflective'],
      ].map(([v, l]) => h('option', { value: v, selected: project.tone === v }, l))
    );
    const text = h('textarea', { class: 'input textarea', rows: 6, placeholder: 'Context and rules for every conversation in this project' });
    text.value = project.instructions;
    const share = h('input', { type: 'checkbox', class: 'switch', checked: project.shareChats });
    const files = h('div', { class: 'doc-list' });
    const renderFiles = async () => {
      const lib = await tauriInvoke<{ id: string; documents: { id: string; name: string }[] }>('chat_project_library', { projectId: project.id });
      project.libraryId = lib.id;
      clear(files, lib.documents.length ? lib.documents.map((d) => h('div', { class: 'doc-row compact' }, icon('file', 15), h('span', { class: 'doc-name' }, d.name))) : h('p', { class: 'muted empty-line' }, 'No files yet.'));
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
