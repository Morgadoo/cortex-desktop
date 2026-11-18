/** Context pages: Skills and Knowledge. */
import { tauriInvoke } from '../../shared/tauri';
import { api, type Memories, type Skill } from './api';
import { clear, confirmDialog, h, icon, markdown, modal, toast } from './dom';
import { state } from './state';
import { button, emptyState, iconButton, modalHeader, pageHeader, switchInput, tabs } from './ui';




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
            class: 'btn btn-danger',
            on: {
              click: async () => {
                if (!(await confirmDialog('Delete skill?', `“${skill.name}” will be removed from this device.`))) return;
                onSaved(await api.deleteSkill(skill.name));
                close();
              },
            },
          }, 'Delete')
        : null;
    // Earlier versions of the user's own skill, newest first, each restorable.
    const historyBtn =
      skill && !builtin
        ? h('button', {
            class: 'btn btn-secondary',
            on: {
              click: async () => {
                const versions = await tauriInvoke<{ id: string; description: string; body: string }[]>('chat_skill_history', { name: skill.name });
                if (!versions.length) return void toast('No earlier versions yet: each save keeps the one it replaces.');
                const when = (id: string) => {
                  const m = id.match(/^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})/);
                  return m ? new Date(Date.UTC(+m[1]!, +m[2]! - 1, +m[3]!, +m[4]!, +m[5]!, +m[6]!)).toLocaleString() : id;
                };
                void modal((closeHistory) =>
                  h(
                    'div',
                    { class: 'dialog-small wide' },
                    modalHeader(`Versions of ${skill.name}`, closeHistory, { flush: true }),
                    versions.map((v) =>
                      h(
                        'details',
                        { class: 'bubble-file' },
                        h('summary', {}, icon('clock', 14), h('span', {}, when(v.id)), h('small', { class: 'muted' }, v.description.slice(0, 60))),
                        h('pre', {}, v.body),
                        h('div', { class: 'dialog-actions' }, h('button', { class: 'btn btn-secondary btn-sm', on: { click: async () => { onSaved(await tauriInvoke('chat_restore_skill', { name: skill.name, version: v.id })); toast('Restored. The version it replaced is kept.'); closeHistory(); close(); } } }, 'Restore this version'))
                      )
                    )
                  )
                );
              },
            },
          }, 'History')
        : null;
    return h(
      'div',
      { class: 'dialog-small wide skill-editor' },
      modalHeader(skill ? (builtin ? `Customize ${skill.name}` : `Edit ${skill.name}`) : 'New skill', close, { flush: true }),
      builtin ? h('p', { class: 'hint' }, 'Built-in skills are read-only. Saving creates your own copy with the same name, which takes its place.') : null,
      h('label', { class: 'field-label' }, 'Name'),
      name,
      h('label', { class: 'field-label' }, 'Description'),
      description,
      h('label', { class: 'field-label' }, 'Instructions'),
      body,
      error,
      h('div', { class: 'dialog-actions' }, remove, historyBtn, h('span', { class: 'spacer' }), h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'), h('button', { class: 'btn btn-primary', on: { click: () => void save() } }, 'Save'))
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
  private tabs: HTMLElement = h('div', { class: 'tabs' });

  constructor() {
    const importBtn = h('button', { class: 'btn btn-secondary' }, 'Import');
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
    const create = h('button', { class: 'btn btn-primary' }, icon('plus', 16), 'New skill');
    create.addEventListener('click', () => void editSkill(null, (skills) => this.set(skills)));
    const search = h('input', { class: 'input', placeholder: 'Search skills' });
    search.addEventListener('input', () => {
      this.filter = search.value.toLowerCase();
      this.render();
    });
    this.el.append(
      pageHeader('Skills', 'Instructions the assistant can reuse. Set them up once and it applies them when a request matches, or call one with /name.', importBtn, create),
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
    this.tabs.replaceWith(
      (this.tabs = tabs(
        [
          { value: 'mine', label: 'My skills', count: mine.length },
          { value: 'builtin', label: 'Built-in', count: builtin.length },
        ],
        this.tab,
        (tab) => {
          this.tab = tab;
          this.render();
        }
      ))
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

interface KnowledgePageData {
  id: string;
  title: string;
  body: string;
  author: string;
  sources: string[];
  updatedAt: string;
}

const knowledgeApi = {
  pages: () => tauriInvoke<KnowledgePageData[]>('chat_knowledge_pages'),
  save: (id: string | null, title: string, body: string) => tauriInvoke<KnowledgePageData[]>('chat_save_knowledge_page', { id, title, body }),
  remove: (id: string) => tauriInvoke<KnowledgePageData[]>('chat_delete_knowledge_page', { id }),
};

/** A knowledge page's editor: title and Markdown, with a live preview. */
function editKnowledgePage(page: KnowledgePageData | null, onSaved: (pages: KnowledgePageData[]) => void) {
  void modal((close) => {
    const title = h('input', { class: 'input', value: page?.title ?? '', placeholder: 'Acme account' });
    const area = h('textarea', { class: 'input textarea mono canvas-editor' });
    area.value = page?.body ?? '';
    const preview = h('div', { class: 'markdown canvas-preview' });
    const renderPreview = () => preview.replaceChildren(markdown(area.value || '*Nothing yet.*'));
    area.addEventListener('input', renderPreview);
    renderPreview();
    const save = async () => {
      try {
        onSaved(await knowledgeApi.save(page?.id ?? null, title.value, area.value));
        close();
      } catch (e) {
        toast(String(e), 'error');
      }
    };
    return h(
      'div',
      { class: 'canvas-modal' },
      modalHeader(title, close),
      h('div', { class: 'canvas-split' }, area, preview),
      page?.sources.length ? h('p', { class: 'muted knowledge-sources' }, `Sources: ${page.sources.join(' · ')}`) : null,
      h('div', { class: 'dialog-actions canvas-actions' }, h('span', { class: 'hint' }, 'Markdown'), h('span', { class: 'spacer' }), button({ variant: 'secondary', label: 'Cancel', onClick: close }), button({ variant: 'primary', label: 'Save', onClick: () => void save() }))
    );
  }, true);
}

export class KnowledgePage {
  readonly el = h('div', { class: 'page' });
  readonly key = 'page:knowledge';
  private memories: Memories = { enabled: true, items: [] };
  private pages: KnowledgePageData[] = [];
  private tab: 'pages' | 'facts' = 'pages';
  private body = h('div', { class: 'knowledge' });

  constructor() {
    const clearAll = h('button', { class: 'btn btn-danger' }, 'Delete all');
    clearAll.addEventListener('click', async () => {
      if (await confirmDialog('Delete all knowledge?', 'The assistant will forget everything it saved about you.', 'Delete all')) {
        this.set(await api.deleteMemory(null));
      }
    });
    const newPage = button({ variant: 'primary', icon: 'plus', label: 'New page', onClick: () => editKnowledgePage(null, (p) => this.setPages(p)) });
    this.el.append(pageHeader('Knowledge', 'Pages the assistant recalls when they are relevant, and short facts it always keeps in mind. It writes them as you chat; you can edit both.', newPage, clearAll), this.body);
    void api.memories().then((m) => this.set(m));
    void knowledgeApi.pages().then((p) => this.setPages(p));
  }

  private setPages(pages: KnowledgePageData[]) {
    this.pages = pages;
    this.render();
  }

  private renderPages(): HTMLElement {
    if (!this.pages.length) {
      return emptyState('book', 'No pages yet', 'Pages hold longer context: a project, a client, how you like reports. Ask “keep this as a knowledge page”, or create one.');
    }
    const sorted = [...this.pages].sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
    return h(
      'div',
      { class: 'knowledge-pages' },
      sorted.map((p) =>
        h(
          'div',
          { class: 'knowledge-card', on: { click: () => editKnowledgePage(p, (all) => this.setPages(all)) } },
          h('div', { class: 'knowledge-card-head' }, icon('book', 16), h('strong', {}, p.title), h('span', { class: 'spacer' }), iconButton({ icon: 'trash', title: 'Delete page', size: 'sm', onClick: async (e) => { e.stopPropagation(); if (await confirmDialog(`Delete “${p.title}”?`, 'The assistant will no longer recall it.', 'Delete')) this.setPages(await knowledgeApi.remove(p.id)); } })),
          h('p', { class: 'muted knowledge-excerpt' }, p.body.replace(/[#*_`>]/g, '').slice(0, 220) || 'Empty page'),
          h('div', { class: 'knowledge-meta' }, h('span', {}, p.author === 'assistant' ? 'Written by the assistant' : 'Written by you'), h('span', {}, new Date(p.updatedAt).toLocaleDateString()), p.sources.length ? h('span', {}, `${p.sources.length} source${p.sources.length === 1 ? '' : 's'}`) : null)
        )
      )
    );
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
    const switcher = tabs(
      [
        { value: 'pages', label: 'Pages', count: this.pages.length },
        { value: 'facts', label: 'Facts', count: m.items.length },
      ],
      this.tab,
      (t) => {
        this.tab = t;
        this.render();
      }
    );
    clear(this.body, toggle, switcher, this.tab === 'pages' ? this.renderPages() : h('div', {}, adder, list));
  }

  update() {}
}
