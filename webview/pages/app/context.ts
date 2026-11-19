/** Context pages: Skills and Knowledge. */
import { tauriInvoke } from '../../shared/tauri';
import { api, type Memories, type Skill } from './api';
import { clear, confirmDialog, h, icon, markdown, modal, popover, promptText, toast, clockHour12 } from './dom';
import { improveButton } from './improve';
import { navigate, state } from './state';
import { button, emptyState, iconButton, modalHeader, pageHeader, switchInput, tabs } from './ui';




// ---------------------------------------------------------------------------------------------
// Skills

/** Editor for a new or existing skill. A built-in skill opens as a copy the user can customize. */
export function editSkill(skill: Skill | null, onSaved: (skills: Skill[]) => void, draft = false): Promise<void> {
  return modal((close) => {
    const builtin = skill?.origin === 'builtin';
    const name = h('input', { class: 'input', value: skill?.name ?? '', placeholder: 'weekly-report', disabled: Boolean(skill && !builtin && !draft) });
    const description = h('textarea', { class: 'input textarea short', rows: 2, placeholder: 'When should the assistant use this skill? e.g. “Use when the user asks for the Friday status report.”' });
    description.value = skill?.description ?? '';
    const body = h('textarea', { class: 'input textarea mono', rows: 14, placeholder: 'Step-by-step instructions the assistant follows when the skill applies.' });
    body.value = skill?.body ?? '';
    const error = h('p', { class: 'error-text' });
    const save = async () => {
      try {
        const slug = name.value.trim().toLowerCase().replace(/[^a-z0-9-]+/g, '-').replace(/^-+|-+$/g, '');
        onSaved(await api.saveSkill(slug, description.value, body.value));
        toast(builtin ? `Your copy of ${slug} replaces the built-in one.` : draft ? `Skill saved. Call it with /${slug}, or the assistant uses it when a request matches.` : 'Skill saved.');
        close();
      } catch (e) {
        error.textContent = String(e);
      }
    };
    const remove =
      skill && !builtin && !draft
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
      skill && !builtin && !draft
        ? h('button', {
            class: 'btn btn-secondary',
            on: {
              click: async () => {
                const versions = await tauriInvoke<{ id: string; description: string; body: string }[]>('chat_skill_history', { name: skill.name });
                if (!versions.length) return void toast('No earlier versions yet: each save keeps the one it replaces.');
                const when = (id: string) => {
                  const m = id.match(/^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})/);
                  return m ? new Date(Date.UTC(+m[1]!, +m[2]! - 1, +m[3]!, +m[4]!, +m[5]!, +m[6]!)).toLocaleString(undefined, { hour12: clockHour12() }) : id;
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
    // Supporting files (templates, reference notes, scripts) the assistant can read when the
    // instructions call for them; only a saved skill of the user's own has a folder for them.
    let current = skill;
    const filesList = h('div', { class: 'skill-files' });
    const renderFiles = () => {
      const files = current?.files ?? [];
      clear(
        filesList,
        files.length
          ? files.map((f) =>
              h(
                'div',
                { class: 'skill-file' },
                icon('file', 14),
                h('button', { class: 'btn-link grow', title: 'View', on: { click: () => void viewSkillFile(current!.name, f) } }, f),
                h('button', { class: 'icon-btn small', title: 'Remove', on: { click: async () => { try { const all = await tauriInvoke<Skill[]>('chat_skill_remove_file', { name: current!.name, path: f }); current = all.find((s) => s.name === current!.name) ?? current; onSaved(all); renderFiles(); } catch (e) { toast(String(e), 'error'); } } } }, icon('trash', 14))
              )
            )
          : h('p', { class: 'muted small' }, 'None. Templates, reference notes or scripts the instructions mention go here; the assistant reads them when it needs them.')
      );
    };
    renderFiles();
    const ownSkill = Boolean(skill && !builtin && !draft);
    const addFiles = ownSkill
      ? h('div', { class: 'skill-file-actions' },
          button({ variant: 'secondary', size: 'sm', icon: 'plus', label: 'Add files…', onClick: async () => {
            try {
              const all = await tauriInvoke<Skill[]>('chat_skill_add_files', { name: skill!.name, folder: null });
              current = all.find((s) => s.name === skill!.name) ?? current;
              onSaved(all);
              renderFiles();
            } catch (e) {
              toast(String(e), 'error');
            }
          } }),
          button({ variant: 'subtle', size: 'sm', icon: 'pencil', label: 'New text file', onClick: async () => {
            const path = await promptText('New file', '', 'Path inside the skill, e.g. templates/report.md');
            if (!path) return;
            try {
              const all = await tauriInvoke<Skill[]>('chat_skill_write_file', { name: skill!.name, path, text: '' });
              current = all.find((s) => s.name === skill!.name) ?? current;
              onSaved(all);
              renderFiles();
              void viewSkillFile(skill!.name, path.trim());
            } catch (e) {
              toast(String(e), 'error');
            }
          } })
        )
      : null;
    return h(
      'div',
      { class: 'dialog-small wide skill-editor' },
      modalHeader(draft ? 'Review the recorded skill' : skill ? (builtin ? `Customize ${skill.name}` : `Edit ${skill.name}`) : 'New skill', close, { flush: true }),
      builtin ? h('p', { class: 'hint' }, 'Built-in skills are read-only. Saving creates your own copy with the same name, which takes its place.') : null,
      h('label', { class: 'field-label' }, 'Name'),
      name,
      h('label', { class: 'field-label' }, 'Description'),
      description,
      h('label', { class: 'field-label' }, 'Instructions'),
      body,
      ownSkill || (current?.files.length ?? 0) > 0 ? h('label', { class: 'field-label' }, 'Files') : null,
      ownSkill || (current?.files.length ?? 0) > 0 ? filesList : null,
      addFiles,
      error,
      h('div', { class: 'dialog-actions' }, remove, historyBtn, h('span', { class: 'spacer' }), h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'), h('button', { class: 'btn btn-primary', on: { click: () => void save() } }, 'Save'))
    );
  });
}

/** A skill's supporting file, editable when it is text. */
async function viewSkillFile(name: string, path: string): Promise<void> {
  let text: string;
  try {
    text = await tauriInvoke<string>('chat_skill_read_file', { name, path });
  } catch (e) {
    return void toast(String(e), 'error');
  }
  const binary = /^\S.* is a binary file \(\d+ bytes\) at /.test(text);
  return modal((close) => {
    const area = h('textarea', { class: 'input textarea mono', rows: 18, disabled: binary });
    area.value = text;
    const save = button({ variant: 'primary', label: 'Save', onClick: async () => {
      try {
        await tauriInvoke('chat_skill_write_file', { name, path, text: area.value });
        toast('File saved.');
        close();
      } catch (e) {
        toast(String(e), 'error');
      }
    } });
    return h('div', { class: 'dialog-small wide' }, modalHeader(`${name} / ${path}`, close, { flush: true }), area, binary ? null : h('div', { class: 'dialog-actions' }, h('span', { class: 'spacer' }), save));
  });
}

/** Learning on its own (learning.rs): a weekly knowledge review, and skills from agents' work. */
function learningRows(): HTMLElement {
  type Learning = { reviewKnowledge: boolean; skillsFromWork: boolean; lastReview?: string | null };
  const box = h('div', {});
  void tauriInvoke<Learning>('chat_learning', { change: null }).then((l) => {
    const change = async (next: Partial<Learning>) => {
      try {
        l = await tauriInvoke<Learning>('chat_learning', { change: { ...l, ...next } });
      } catch (e) {
        toast(String(e), 'error');
      }
    };
    clear(
      box,
      h(
        'div',
        { class: 'setting-row' },
        h('div', {}, h('strong', {}, 'Review my knowledge weekly'), h('small', { class: 'muted' }, `Once a week, a background review looks for duplicates, contradictions, stale and sensitive facts, and leaves its proposals in an unread chat. Nothing changes until you answer there.${l.lastReview ? ` Last review: ${new Date(l.lastReview).toLocaleDateString()}.` : ''}`)),
        switchInput(l.reviewKnowledge, (v) => void change({ reviewKnowledge: v }))
      ),
      h(
        'div',
        { class: 'setting-row' },
        h('div', {}, h('strong', {}, 'Agents learn skills from their work'), h('small', { class: 'muted' }, 'After an agent works through several steps, it considers whether the procedure is worth reusing and, if so, saves it as a skill, turned off until you review it.')),
        switchInput(l.skillsFromWork, (v) => void change({ skillsFromWork: v }))
      )
    );
  }).catch(() => undefined);
  return box;
}

export class SkillsPage {
  readonly el = h('div', { class: 'page' });
  readonly key = 'page:skills';
  private skills: Skill[] = [];
  private tab: 'mine' | 'builtin' = 'mine';
  private filter = '';
  private list = h('div', { class: 'skill-list' });
  private tabs: HTMLElement = h('div', { class: 'tabs' });

  /**
   * Claude plugins (or a marketplace of them): skills and commands become skills, subagents
   * become agents, `.mcp.json` servers become tool servers under Connectors.
   */
  private async importPlugin(link: string | null) {
    type Imported = { plugins: string[]; skills: string[]; agents: string[]; servers: string[]; hooks: number; skipped: string[] };
    if (link) toast('Downloading…');
    try {
      const out = await tauriInvoke<Imported | null>('chat_import_plugin', { link });
      if (!out) return;
      await this.load();
      const parts = [
        out.skills.length ? `${out.skills.length} skill${out.skills.length === 1 ? '' : 's'} (${out.skills.slice(0, 6).map((n) => `/${n}`).join(', ')}${out.skills.length > 6 ? '…' : ''})` : '',
        out.agents.length ? `${out.agents.length} agent${out.agents.length === 1 ? '' : 's'} (${out.agents.join(', ')})` : '',
        out.servers.length ? `the tool server${out.servers.length === 1 ? '' : 's'} ${out.servers.join(', ')} under Connectors` : '',
        out.hooks ? `${out.hooks} hook${out.hooks === 1 ? '' : 's'}, turned off until you review ${out.hooks === 1 ? 'it' : 'them'} in Settings → Permissions → Hooks` : '',
      ].filter(Boolean);
      void modal((close) =>
        h(
          'div',
          { class: 'dialog-small' },
          modalHeader(`Imported ${out.plugins.join(', ')}`, close, { flush: true }),
          h('p', {}, parts.length ? `Added ${parts.join('; ')}.` : 'Nothing could be added from it.'),
          out.skipped.length ? h('p', { class: 'muted small' }, `Left out: ${out.skipped.join('; ')}.`) : null,
          h('div', { class: 'dialog-actions' }, h('span', { class: 'spacer' }), button({ variant: 'primary', label: 'Done', onClick: close }))
        )
      );
    } catch (e) {
      toast(String(e), 'error');
    }
  }

  /**
   * A marketplace (Osaurus's plugin registry): its plugins listed to tick and import. A link to a
   * single plugin imports it at once.
   */
  private async openMarketplace(link: string) {
    type Entry = { name: string; description: string; category: string; link: string | null };
    let entries: Entry[] | null;
    try {
      entries = await tauriInvoke<Entry[] | null>('chat_plugin_marketplace', { link });
    } catch (e) {
      return toast(String(e), 'error');
    }
    if (!entries) return void this.importPlugin(link);
    if (!entries.length) return toast('That marketplace lists no plugins.', 'error');
    const list = entries;
    void modal((close) => {
      const search = h('input', { class: 'input', placeholder: `Search ${list.length} plugins` });
      const rows = h('div', { class: 'market-list' });
      const chosen = new Set<string>();
      const go = button({ variant: 'primary', label: 'Import', onClick: () => void run() });
      const update = () => {
        go.disabled = !chosen.size;
        go.textContent = chosen.size ? `Import ${chosen.size}` : 'Import';
      };
      const render = () => {
        const q = search.value.toLowerCase();
        clear(
          rows,
          list
            .filter((e) => !q || `${e.name} ${e.description} ${e.category}`.toLowerCase().includes(q))
            .map((e) => {
              const box = h('input', { type: 'checkbox', checked: chosen.has(e.name), disabled: !e.link });
              box.addEventListener('change', () => {
                if (box.checked) chosen.add(e.name);
                else chosen.delete(e.name);
                update();
              });
              return h(
                'label',
                { class: `market-row${e.link ? '' : ' disabled'}` },
                box,
                h('span', { class: 'market-text' }, h('strong', {}, e.name), e.category ? h('small', { class: 'muted' }, ` · ${e.category}`) : null, h('span', { class: 'muted small' }, e.description || (e.link ? '' : 'Its source is not on GitHub; it cannot be fetched from here.')))
              );
            })
        );
      };
      const run = async () => {
        const picked = list.filter((e) => chosen.has(e.name) && e.link);
        close();
        const done: string[] = [];
        for (const e of picked) {
          toast(`Importing ${e.name}…`);
          try {
            await tauriInvoke('chat_import_plugin', { link: e.link });
            done.push(e.name);
          } catch (err) {
            toast(`${e.name}: ${String(err)}`, 'error');
          }
        }
        await this.load();
        if (done.length) toast(`Imported ${done.join(', ')}. Their skills are below, agents under Agents, tool servers under Connectors.`);
      };
      search.addEventListener('input', render);
      render();
      update();
      setTimeout(() => search.focus());
      return h(
        'div',
        { class: 'dialog-small wide' },
        modalHeader('Plugins in this marketplace', close, { flush: true }),
        search,
        rows,
        h('div', { class: 'dialog-actions' }, h('span', { class: 'spacer' }), button({ variant: 'secondary', label: 'Cancel', onClick: close }), go)
      );
    });
  }

  constructor() {
    // Import a SKILL.md from this computer, or from a link (a skill's folder on GitHub).
    const importBtn = h('button', { class: 'btn btn-secondary' }, 'Import', icon('chevron', 14));
    importBtn.addEventListener('click', () => {
      const done = async (name: string | null) => {
        if (!name) return;
        toast(`Imported ${name}.`);
        await this.load();
      };
      const close = popover(
        importBtn,
        h(
          'div',
          { class: 'menu' },
          h('button', { class: 'menu-item', on: { click: () => { close(); void api.importSkill().then(done).catch((e) => toast(String(e), 'error')); } } }, icon('upload', 15), h('span', {}, 'From a file…')),
          h(
            'button',
            {
              class: 'menu-item',
              on: {
                click: async () => {
                  close();
                  const url = await promptText('Import a skill from a link', '', 'https://github.com/owner/repo/tree/main/skills/name');
                  if (!url) return;
                  toast('Downloading the skill…');
                  await tauriInvoke<string>('chat_import_skill_url', { url }).then(done).catch((e) => toast(String(e), 'error'));
                },
              },
            },
            icon('link', 15),
            h('span', {}, 'From a link (GitHub)…')
          ),
          h('button', { class: 'menu-item', on: { click: () => { close(); void openSkillCatalog(() => void this.load()); } } }, icon('globe', 15), h('span', {}, 'Browse skill catalogues…')),
          h('div', { class: 'menu-sep' }),
          h('button', { class: 'menu-item', on: { click: () => { close(); void this.importPlugin(null); } } }, icon('folder', 15), h('span', {}, 'Claude plugin from a folder…')),
          h(
            'button',
            {
              class: 'menu-item',
              on: {
                click: async () => {
                  close();
                  const link = await promptText('Import Claude plugins from GitHub', '', 'https://github.com/owner/repo (a plugin or a marketplace)');
                  if (link) void this.openMarketplace(link);
                },
              },
            },
            icon('plug', 15),
            h('span', {}, 'Claude plugins from GitHub…')
          )
        )
      );
    });
    const record = button({ variant: 'secondary', icon: 'play', label: 'Record a task', title: 'Do a task once while Lumo watches; it becomes a skill you review', onClick: () => void recordTask() });
    const create = h('button', { class: 'btn btn-primary' }, icon('plus', 16), 'New skill');
    create.addEventListener('click', () => void editSkill(null, (skills) => this.set(skills)));
    const search = h('input', { class: 'input', placeholder: 'Search skills' });
    search.addEventListener('input', () => {
      this.filter = search.value.toLowerCase();
      this.render();
    });
    this.el.append(
      pageHeader('Skills', 'Instructions the assistant can reuse. Set them up once and it applies them when a request matches, or call one with /name.', importBtn, record, create),
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
          skill.files?.length ? h('span', { class: 'tag', title: skill.files.join('\n') }, `${skill.files.length} file${skill.files.length === 1 ? '' : 's'}`) : null,
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

const MEMORY_CATEGORIES: [string, string][] = [
  ['preference', 'Preference'],
  ['work', 'Work'],
  ['person', 'Person'],
  ['project', 'Project'],
  ['habit', 'Habit'],
  ['fact', 'Fact'],
];

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
    const improve = improveButton(area, renderPreview);
    return h(
      'div',
      { class: 'canvas-modal' },
      modalHeader(title, close),
      h('div', { class: 'canvas-split' }, area, preview),
      page?.sources.length ? h('p', { class: 'muted knowledge-sources' }, `Sources: ${page.sources.join(' · ')}`) : null,
      h('div', { class: 'dialog-actions canvas-actions' }, improve, h('span', { class: 'hint' }, 'Markdown'), h('span', { class: 'spacer' }), button({ variant: 'secondary', label: 'Cancel', onClick: close }), button({ variant: 'primary', label: 'Save', onClick: () => void save() }))
    );
  }, true);
}

export class KnowledgePage {
  readonly el = h('div', { class: 'page' });
  readonly key = 'page:knowledge';
  private memories: Memories = { enabled: true, items: [], keepSensitiveOut: true };
  private memoryQuery = '';
  private memorySource: 'all' | 'you' | 'assistant' = 'all';
  private memoryCategory = '';
  private memoryDays = 0;
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
        h('div', {}, h('strong', {}, 'Use knowledge'), h('small', { class: 'muted' }, 'Let the assistant use the facts and pages below in new conversations.')),
        switchInput(m.enabled, async (v) => this.set(await api.setMemoryEnabled(v)))
      ),
      h(
        'div',
        { class: 'setting-row' },
        h('div', {}, h('strong', {}, 'Save new facts'), h('small', { class: 'muted' }, 'Let the assistant add, change and remove facts and pages as you talk. Off: it uses what is here but changes nothing; you can still edit it yourself.')),
        switchInput(m.update ?? true, async (v) => this.set(await tauriInvoke<Memories>('chat_set_memory_update', { update: v })))
      ),
      h(
        'div',
        { class: 'setting-row' },
        h('div', {}, h('strong', {}, 'Keep sensitive data out'), h('small', { class: 'muted' }, 'The assistant does not save card numbers, bank accounts, passwords, PINs or keys to knowledge, and tells you when it left something out.')),
        switchInput(m.keepSensitiveOut ?? true, async (v) => this.set(await tauriInvoke<Memories>('chat_set_keep_sensitive_out', { on: v })))
      ),
      learningRows()
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
    // Filters (LobeHub): who saved a fact, when, and words in it.
    const query = h('input', { class: 'input small memory-find', type: 'search', placeholder: 'Find a fact', value: this.memoryQuery });
    query.addEventListener('input', () => {
      this.memoryQuery = query.value;
      this.render();
      const again = this.el.querySelector<HTMLInputElement>('.memory-find');
      again?.focus();
      again?.setSelectionRange(again.value.length, again.value.length);
    });
    const chip = (value: 'all' | 'you' | 'assistant', label: string) =>
      h('button', { class: `chip-btn${this.memorySource === value ? ' active' : ''}`, on: { click: () => { this.memorySource = value; this.render(); } } }, label);
    const sinceChip = (days: number, label: string) =>
      h('button', { class: `chip-btn${this.memoryDays === days ? ' active' : ''}`, on: { click: () => { this.memoryDays = days; this.render(); } } }, label);
    const filters = m.items.length > 3
      ? h(
          'div',
          { class: 'memory-filters' },
          query,
          chip('all', 'All'),
          chip('you', 'Added by you'),
          chip('assistant', 'Saved by the assistant'),
          sinceChip(0, 'Any time'),
          sinceChip(30, 'Last 30 days'),
          (() => {
            const pick = h('select', { class: 'memory-category' }, h('option', { value: '' }, 'Every subject'), MEMORY_CATEGORIES.map(([v, l]) => h('option', { value: v, selected: this.memoryCategory === v }, l)));
            pick.addEventListener('change', () => { this.memoryCategory = pick.value; this.render(); });
            return pick;
          })()
        )
      : null;
    const since = this.memoryDays ? Date.now() - this.memoryDays * 86_400_000 : 0;
    const words = this.memoryQuery.toLowerCase().split(/\s+/).filter(Boolean);
    const shown = m.items.filter(
      (i) =>
        (this.memorySource === 'all' || (this.memorySource === 'assistant') === (i.source === 'assistant')) &&
        (!since || Date.parse(i.createdAt) >= since) &&
        (!this.memoryCategory || i.category === this.memoryCategory) &&
        words.every((w) => i.text.toLowerCase().includes(w))
    );
    const list = m.items.length
      ? h(
          'div',
          { class: 'memory-list' },
          filters,
          shown.length ? null : h('p', { class: 'muted' }, 'No fact matches.'),
          [...shown].reverse().map((item) => {
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
            // Filing (LobeHub's memory categories, weight and confidence).
            const file = (category: string, importance: number, confirmed = false) =>
              void tauriInvoke<Memories>('chat_file_memory', { id: item.id, category, importance, confirmed }).then((m) => this.set(m)).catch((e) => toast(String(e), 'error'));
            const category = h('select', { class: 'memory-category', title: 'What it is about' }, h('option', { value: '' }, 'Other'), MEMORY_CATEGORIES.map(([v, l]) => h('option', { value: v, selected: item.category === v }, l)));
            category.addEventListener('change', () => file(category.value, item.importance ?? 2));
            const weight = item.importance ?? 2;
            const importance = h('button', { class: `memory-weight w${weight}`, title: `Importance: ${['', 'a detail', 'normal', 'always keep in mind'][weight]} (click to change)` }, '●'.repeat(weight) + '○'.repeat(3 - weight));
            importance.addEventListener('click', () => file(item.category ?? '', (weight % 3) + 1));
            const inferred = item.confidence === 'inferred'
              ? h('button', { class: 'memory-inferred', title: 'The assistant concluded this. Click to confirm it is right.', on: { click: () => file(item.category ?? '', weight, true) } }, 'inferred')
              : null;
            const source = item.conversation && state.conversations.some((c) => c.id === item.conversation)
              ? h('button', { class: 'icon-btn small', title: 'Open the chat it came from', on: { click: () => navigate({ kind: 'chat', id: item.conversation! }) } }, icon('chat', 15))
              : null;
            return h(
              'div',
              { class: 'memory-row' },
              icon(item.source === 'assistant' ? 'sparkle' : 'user', 16),
              text,
              inferred,
              category,
              importance,
              h('span', { class: 'muted memory-date' }, new Date(item.createdAt).toLocaleDateString()),
              source,
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

/**
 * Teach a task by recording (OpenBot): Lumo steps aside while the user does the task once; a
 * Recording pill keeps time and stops it. Screenshots are taken when the screen changes, never
 * keystrokes; see src-tauri/src/chat/recorder.rs.
 */
export async function recordTask() {
  const ok = await confirmDialog(
    'Record a task?',
    'Lumo will step aside and take a screenshot whenever your screen changes, with the name of the window in front (never what you type). Do the task once, then press Stop on the Recording pill. Close anything private first: the screenshots go to your chosen model to write the skill, and are dropped afterwards.',
    'Start recording',
    false
  );
  if (!ok) return;
  try {
    await tauriInvoke('chat_record_start');
  } catch (e) {
    toast(String(e), 'error');
  }
}

/** After Stop: what the task was, then the model's draft to review and save. */
export function reviewRecording(status: { seconds: number; steps: number }) {
  void modal((close) => {
    const minutes = `${Math.floor(status.seconds / 60)}:${String(status.seconds % 60).padStart(2, '0')}`;
    const note = h('textarea', { class: 'input textarea short', rows: 3, placeholder: 'What was the task, and anything to know? e.g. “Export last month’s invoices as PDF and email them to accounting.”' });
    const error = h('p', { class: 'error-text' });
    const discard = async () => {
      await tauriInvoke('chat_record_discard').catch(() => undefined);
      close();
    };
    const go = button({
      variant: 'primary',
      icon: 'sparkle',
      label: 'Write the skill',
      onClick: async () => {
        if (!state.model) return void (error.textContent = 'Choose a model that reads images first.');
        clear(error, h('span', { class: 'dots' }), ` ${state.model} is looking at the steps…`);
        try {
          const draft = await tauriInvoke<{ name: string; description: string; body: string }>('chat_record_draft', { model: state.model, note: note.value });
          close();
          void editSkill({ ...draft, origin: 'user' } as Skill, (skills) => {
            state.skills = skills;
          }, true);
        } catch (e) {
          error.textContent = String(e);
        }
      },
    });
    return h(
      'div',
      { class: 'dialog-small' },
      modalHeader('Turn the recording into a skill', () => void discard(), { flush: true }),
      h('p', { class: 'muted' }, status.steps ? `${status.steps} steps seen in ${minutes}.` : 'Nothing changed on screen while recording, so there is nothing to learn from. Try again.'),
      h('label', { class: 'field-label' }, 'What was the task? (optional, helps the model)'),
      note,
      error,
      h('div', { class: 'dialog-actions' }, button({ variant: 'secondary', label: 'Discard', onClick: () => void discard() }), status.steps ? go : null)
    );
  });
}

interface CatalogSkill {
  catalog: string;
  branch: string;
  path: string;
  name: string;
  description: string;
  group: string;
}

/**
 * Skill catalogues (LobeHub's skills marketplace): Anthropic's public skills and any other
 * repository that lists its skills in .claude-plugin/marketplace.json, searched and added here.
 */
async function openSkillCatalog(onAdded: () => void) {
  await modal((close) => {
    const search = h('input', { class: 'input', placeholder: 'Search skills: pdf, design, testing…', 'aria-label': 'Search skills' });
    const grid = h('div', { class: 'market-grid' }, h('p', { class: 'muted' }, 'Reading the catalogues…'));
    const sources = h('div', { class: 'catalog-sources' });
    const count = h('small', { class: 'muted' });
    let all: CatalogSkill[] = [];
    let installed = new Set(state.skills?.map((s: Skill) => s.name) ?? []);
    const draw = () => {
      const q = search.value.trim().toLowerCase();
      const shown = all.filter((s) => !q || `${s.name} ${s.description} ${s.group}`.toLowerCase().includes(q));
      count.textContent = `${shown.length} of ${all.length} skills`;
      clear(
        grid,
        shown.length
          ? shown.map((s) => {
              const add = button({ variant: 'secondary', label: installed.has(s.name) ? 'Added' : 'Add', onClick: async () => {
                add.setAttribute('disabled', '');
                try {
                  const r = await tauriInvoke<{ name: string; files: number; note: string | null }>('chat_install_catalog_skill', { catalog: s.catalog, branch: s.branch, path: s.path });
                  installed.add(r.name);
                  add.textContent = 'Added';
                  toast(`Added ${r.name}${r.files ? ` with ${r.files} files` : ''}.${r.note ? ` ${r.note}` : ''}`);
                  onAdded();
                } catch (e) {
                  add.removeAttribute('disabled');
                  toast(String(e), 'error');
                }
              } });
              if (installed.has(s.name)) add.setAttribute('disabled', '');
              return h(
                'div',
                { class: 'market-card' },
                h('div', { class: 'market-head' }, h('span', { class: 'agent-avatar' }, icon('skill', 16)), h('div', { class: 'grow' }, h('strong', {}, s.name), h('small', { class: 'muted' }, `${s.group} · ${s.catalog}`))),
                h('p', { class: 'market-desc' }, s.description),
                h('div', { class: 'market-foot' }, h('span', { class: 'spacer' }), add)
              );
            })
          : h('p', { class: 'muted' }, 'No skill matches.')
      );
    };
    const load = async (refresh: boolean) => {
      try {
        const r = await tauriInvoke<{ catalogs: string[]; listings: CatalogSkill[]; errors: string[] }>('chat_skill_catalog', { refresh });
        all = r.listings;
        installed = new Set((await api.skills().catch(() => [] as Skill[])).map((s) => s.name));
        clear(
          sources,
          r.catalogs.map((c) =>
            h('span', { class: 'chip' }, c, c === 'anthropics/skills' ? null : h('button', { class: 'chip-x', title: 'Remove this catalogue', on: { click: async () => { await tauriInvoke('chat_remove_skill_catalog', { repo: c }); void load(true); } } }, icon('close', 12)))
          ),
          h('button', { class: 'chip add', on: { click: async () => {
            const repo = await promptText('Add a skill catalogue', '', 'owner/repo (a GitHub repository with .claude-plugin/marketplace.json)');
            if (!repo) return;
            try {
              await tauriInvoke('chat_add_skill_catalog', { repo });
              void load(true);
            } catch (e) {
              toast(String(e), 'error');
            }
          } } }, icon('plus', 12), 'Add catalogue')
        );
        if (r.errors.length) toast(r.errors.slice(0, 2).join(' '), 'error');
        draw();
      } catch (e) {
        clear(grid, h('p', { class: 'error-text' }, String(e)));
      }
    };
    search.addEventListener('input', draw);
    void load(false);
    return h(
      'div',
      { class: 'dialog-small wide market-dialog' },
      modalHeader('Skill catalogues', close, { flush: true }),
      h('p', { class: 'muted' }, 'Skills others publish on GitHub, starting with Anthropic\'s. Adding one copies its instructions, and its scripts and files where GitHub lists them, into your skills, to read and change like your own.'),
      sources,
      h('div', { class: 'row-inline' }, h('div', { class: 'grow' }, search), button({ variant: 'secondary', icon: 'retry', label: 'Refresh', onClick: () => void load(true) }), button({ variant: 'secondary', icon: 'share', label: 'Publish my skills…', onClick: () => void publishSkills() })),
      count,
      grid
    );
  }, true);
}

/** Publishes the user's skills as a catalogue folder to push to GitHub, for others to add. */
async function publishSkills() {
  const mine = (await api.skills().catch(() => [] as Skill[])).filter((s) => s.origin === 'user');
  if (!mine.length) return void toast('You have no skills of your own to publish yet.', 'error');
  await modal((close) => {
    const picked = new Set<string>();
    const title = h('input', { class: 'input', value: 'My skills', 'aria-label': 'Catalogue name' });
    const author = h('input', { class: 'input', placeholder: 'Your name (optional)', 'aria-label': 'Author' });
    const list = h(
      'div',
      { class: 'hook-list' },
      mine.map((s) => {
        const box = h('input', { type: 'checkbox', 'aria-label': s.name });
        box.addEventListener('change', () => (box.checked ? picked.add(s.name) : picked.delete(s.name)));
        return h('label', { class: 'hook-row publish-row' }, box, h('div', { class: 'grow' }, h('strong', {}, s.name), h('small', { class: 'muted' }, s.description)));
      })
    );
    const go = button({ variant: 'primary', label: 'Choose a folder and publish', onClick: async () => {
      try {
        const folder = await tauriInvoke<string | null>('chat_export_skill_catalog', { names: [...picked], title: title.value, author: author.value });
        if (!folder) return;
        close();
        toast(`Written to ${folder}. Push that folder to a GitHub repository; anyone can then add it as a catalogue by owner/repo.`);
      } catch (e) {
        toast(String(e), 'error');
      }
    } });
    return h(
      'div',
      { class: 'dialog-small' },
      modalHeader('Publish skills', close, { flush: true }),
      h('p', { class: 'muted' }, 'Lumo writes the chosen skills as a catalogue (a .claude-plugin/marketplace.json and a folder per skill). Put it in a GitHub repository, and others add it in Lumo, Claude Code or any app that reads that format.'),
      h('label', { class: 'field-label' }, 'Catalogue name'),
      title,
      h('label', { class: 'field-label' }, 'Author'),
      author,
      h('label', { class: 'field-label' }, 'Skills'),
      list,
      h('div', { class: 'dialog-actions' }, h('span', { class: 'spacer' }), go)
    );
  });
}
