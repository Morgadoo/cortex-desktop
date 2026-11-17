/** Context pages: Skills and Knowledge. */
import { api, type Memories, type Skill } from './api';
import { clear, confirmDialog, h, icon, modal, toast } from './dom';
import { state } from './state';

function header(title: string, subtitle: string, ...actions: (HTMLElement | null)[]): HTMLElement {
  return h('header', { class: 'page-header' }, h('div', {}, h('h1', {}, title), h('p', { class: 'muted' }, subtitle)), h('div', { class: 'page-actions' }, actions));
}

function emptyState(ic: string, title: string, body: string): HTMLElement {
  return h('div', { class: 'empty-state' }, h('div', { class: 'empty-icon' }, icon(ic, 22)), h('h3', {}, title), h('p', { class: 'muted' }, body));
}

function switchInput(checked: boolean, onChange: (value: boolean) => void, title = ''): HTMLInputElement {
  const input = h('input', { type: 'checkbox', class: 'switch', checked, title });
  input.addEventListener('click', (e) => e.stopPropagation());
  input.addEventListener('change', () => onChange(input.checked));
  return input;
}

// ---------------------------------------------------------------------------------------------
// Skills

/** Editor for a new or existing skill. A built-in skill opens as a copy the user can customize. */
export function editSkill(skill: Skill | null, onSaved: (skills: Skill[]) => void): Promise<void> {
  return modal((close) => {
    const builtin = skill?.origin === 'builtin';
    const name = h('input', { class: 'input', value: skill?.name ?? '', placeholder: 'weekly-report', disabled: Boolean(skill && !builtin) });
    const description = h('textarea', { class: 'input textarea short', rows: 2, placeholder: 'When should the assistant use this skill? e.g. “Use when the user asks for the Friday status report.”' });
    description.value = skill?.description ?? '';
    const body = h('textarea', { class: 'input textarea mono', rows: 14, placeholder: 'Step-by-step instructions the assistant follows when the skill applies.' });
    body.value = skill?.body ?? '';
    const error = h('p', { class: 'error-text' });
    const save = async () => {
      try {
        const slug = name.value.trim().toLowerCase().replace(/[^a-z0-9-]+/g, '-').replace(/^-+|-+$/g, '');
        onSaved(await api.saveSkill(slug, description.value, body.value));
        toast(builtin ? `Your copy of ${slug} replaces the built-in one.` : 'Skill saved.');
        close();
      } catch (e) {
        error.textContent = String(e);
      }
    };
    const remove =
      skill && !builtin
        ? h('button', {
            class: 'btn btn-ghost danger',
            on: {
              click: async () => {
                if (!(await confirmDialog('Delete skill?', `“${skill.name}” will be removed from this device.`))) return;
                onSaved(await api.deleteSkill(skill.name));
                close();
              },
            },
          }, 'Delete')
        : null;
    return h(
      'div',
      { class: 'dialog-small wide skill-editor' },
      h('div', { class: 'modal-head flush' }, h('h2', {}, skill ? (builtin ? `Customize ${skill.name}` : `Edit ${skill.name}`) : 'New skill'), h('button', { class: 'icon-btn', on: { click: close } }, icon('close', 18))),
      builtin ? h('p', { class: 'hint' }, 'Built-in skills are read-only. Saving creates your own copy with the same name, which takes its place.') : null,
      h('label', { class: 'field-label' }, 'Name'),
      name,
      h('label', { class: 'field-label' }, 'Description'),
      description,
      h('label', { class: 'field-label' }, 'Instructions'),
      body,
      error,
      h('div', { class: 'dialog-actions' }, remove, h('span', { class: 'spacer' }), h('button', { class: 'btn btn-ghost', on: { click: close } }, 'Cancel'), h('button', { class: 'btn btn-primary', on: { click: () => void save() } }, 'Save'))
    );
  });
}

export class SkillsPage {
  readonly el = h('div', { class: 'page' });
  readonly key = 'page:skills';
  private skills: Skill[] = [];
  private tab: 'mine' | 'builtin' = 'mine';
  private filter = '';
  private list = h('div', { class: 'skill-list' });
  private tabs = h('div', { class: 'tabs' });

  constructor() {
    const importBtn = h('button', { class: 'btn btn-ghost' }, 'Import');
    importBtn.addEventListener('click', async () => {
      try {
        const name = await api.importSkill();
        if (name) {
          toast(`Imported ${name}.`);
          await this.load();
        }
      } catch (e) {
        toast(String(e), 'error');
      }
    });
    const create = h('button', { class: 'btn btn-light' }, icon('plus', 16), 'New skill');
    create.addEventListener('click', () => void editSkill(null, (skills) => this.set(skills)));
    const search = h('input', { class: 'input', placeholder: 'Search skills' });
    search.addEventListener('input', () => {
      this.filter = search.value.toLowerCase();
      this.render();
    });
    this.el.append(
      header('Skills', 'Instructions the assistant can reuse. Set them up once and it applies them when a request matches, or call one with /name.', importBtn, create),
      this.tabs,
      h('div', { class: 'toolbar' }, h('div', { class: 'search-wrap' }, icon('search', 16), search)),
      this.list
    );
    void this.load();
  }

  private async load() {
    this.set(await api.skills());
  }

  private set(skills: Skill[]) {
    this.skills = skills;
    state.skills = skills;
    this.render();
  }

  private render() {
    const mine = this.skills.filter((s) => s.origin === 'user');
    const builtin = this.skills.filter((s) => s.origin === 'builtin');
    clear(
      this.tabs,
      h('button', { class: `tab${this.tab === 'mine' ? ' active' : ''}`, on: { click: () => { this.tab = 'mine'; this.render(); } } }, `My skills ${mine.length}`),
      h('button', { class: `tab${this.tab === 'builtin' ? ' active' : ''}`, on: { click: () => { this.tab = 'builtin'; this.render(); } } }, `Built-in ${builtin.length}`)
    );
    const shown = (this.tab === 'mine' ? mine : builtin).filter((s) => !this.filter || `${s.name} ${s.description}`.toLowerCase().includes(this.filter));
    if (!shown.length) {
      clear(
        this.list,
        this.tab === 'mine'
          ? emptyState('skill', 'No skills of your own yet', 'Create one, import a SKILL.md, or customize a built-in skill.')
          : emptyState('skill', 'No match', 'Try another search.')
      );
      return;
    }
    clear(
      this.list,
      shown.map((skill) =>
        h(
          'div',
          { class: 'skill-row', on: { click: () => void editSkill(skill, (skills) => this.set(skills)) } },
          icon('skill', 18),
          h('strong', { class: 'skill-name' }, skill.name),
          skill.origin === 'user' ? h('span', { class: 'tag' }, 'Local') : null,
          h('span', { class: 'skill-desc muted' }, skill.description),
          switchInput(skill.enabled, async (enabled) => this.set(await api.setSkillEnabled(skill.name, enabled)), skill.enabled ? 'Turn off' : 'Turn on')
        )
      )
    );
  }

  update() {}
}

// ---------------------------------------------------------------------------------------------
// Knowledge

export class KnowledgePage {
  readonly el = h('div', { class: 'page' });
  readonly key = 'page:knowledge';
  private memories: Memories = { enabled: true, items: [] };
  private body = h('div', { class: 'knowledge' });

  constructor() {
    const clearAll = h('button', { class: 'btn btn-ghost danger' }, 'Delete all');
    clearAll.addEventListener('click', async () => {
      if (await confirmDialog('Delete all knowledge?', 'The assistant will forget everything it saved about you.', 'Delete all')) {
        this.set(await api.deleteMemory(null));
      }
    });
    this.el.append(header('Knowledge', 'What the assistant remembers about you across conversations. Say “remember…” in any chat, or add facts here.', clearAll), this.body);
    void api.memories().then((m) => this.set(m));
  }

  private set(memories: Memories) {
    this.memories = memories;
    this.render();
  }

  private render() {
    const m = this.memories;
    const toggle = h(
      'section',
      { class: 'card' },
      h(
        'div',
        { class: 'setting-row' },
        h('div', {}, h('strong', {}, 'Use knowledge'), h('small', { class: 'muted' }, 'Let the assistant save facts about you and use them in new conversations.')),
        switchInput(m.enabled, async (v) => this.set(await api.setMemoryEnabled(v)))
      )
    );
    const input = h('input', { class: 'input', placeholder: 'Add something the assistant should know, e.g. “I work in renewable energy regulation”' });
    const add = async () => {
      if (!input.value.trim()) return;
      try {
        this.set(await api.addMemory(input.value));
      } catch (e) {
        toast(String(e), 'error');
      }
    };
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') void add();
    });
    const adder = h('div', { class: 'memory-add' }, input, h('button', { class: 'btn btn-primary', on: { click: () => void add() } }, 'Add'));
    const list = m.items.length
      ? h(
          'div',
          { class: 'memory-list' },
          [...m.items].reverse().map((item) => {
            const text = h('div', { class: 'memory-text' }, item.text);
            const edit = h('button', { class: 'icon-btn small', title: 'Edit' }, icon('edit', 15));
            edit.addEventListener('click', () => {
              const field = h('input', { class: 'input small', value: item.text });
              const commit = async () => {
                try {
                  this.set(await api.updateMemory(item.id, field.value));
                } catch (e) {
                  toast(String(e), 'error');
                }
              };
              field.addEventListener('keydown', (e) => {
                if (e.key === 'Enter') void commit();
                if (e.key === 'Escape') this.render();
              });
              field.addEventListener('blur', () => void commit());
              text.replaceWith(field);
              field.focus();
            });
            return h(
              'div',
              { class: 'memory-row' },
              icon(item.source === 'assistant' ? 'sparkle' : 'user', 16),
              text,
              h('span', { class: 'muted memory-date' }, new Date(item.createdAt).toLocaleDateString()),
              edit,
              h('button', { class: 'icon-btn small', title: 'Delete', on: { click: async () => this.set(await api.deleteMemory(item.id)) } }, icon('trash', 15))
            );
          })
        )
      : emptyState('book', 'Nothing saved yet', 'The assistant saves lasting facts as you chat, like your role, your projects and how you like answers.');
    clear(this.body, toggle, adder, list);
  }

  update() {}
}
