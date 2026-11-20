/** The Skills page, the skill editor and catalogues, and teaching a task by recording it. */
import { api, type Skill } from '../core/api';
import { clear, h } from '../ui/h';
import { icon } from '../ui/icons';
import { confirmDialog, modal, popover, promptText, toast } from '../ui/overlay';
import { clockHour12 } from '../core/clock';
import { state } from '../core/state';
import { button, emptyState, modalHeader, pageHeader, switchInput, tabs } from '../ui/ui';
import { call, callQuiet, invoke, reported, shown } from '../core/call';
import { openSkillCatalog } from './skill-catalog';

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
        const slug = name.value
          .trim()
          .toLowerCase()
          .replace(/[^a-z0-9-]+/g, '-')
          .replace(/^-+|-+$/g, '');
        onSaved(await api.saveSkill(slug, description.value, body.value));
        toast(builtin ? `Your copy of ${slug} replaces the built-in one.` : draft ? `Skill saved. Call it with /${slug}, or the assistant uses it when a request matches.` : 'Skill saved.');
        close();
      } catch (e) {
        error.textContent = String(e);
      }
    };
    const remove =
      skill && !builtin && !draft
        ? h(
            'button',
            {
              class: 'btn btn-danger',
              on: {
                click: async () => {
                  if (!(await confirmDialog('Delete skill?', `“${skill.name}” will be removed from this device.`))) return;
                  onSaved(await api.deleteSkill(skill.name));
                  close();
                },
              },
            },
            'Delete'
          )
        : null;
    // Earlier versions of the user's own skill, newest first, each restorable.
    const historyBtn =
      skill && !builtin && !draft
        ? h(
            'button',
            {
              class: 'btn btn-secondary',
              on: {
                click: async () => {
                  const versions = await invoke<{ id: string; description: string; body: string }[]>('chat_skill_history', { name: skill.name });
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
                          h(
                            'div',
                            { class: 'dialog-actions' },
                            h(
                              'button',
                              {
                                class: 'btn btn-secondary btn-sm',
                                on: {
                                  click: async () => {
                                    onSaved(await invoke('chat_restore_skill', { name: skill.name, version: v.id }));
                                    toast('Restored. The version it replaced is kept.');
                                    closeHistory();
                                    close();
                                  },
                                },
                              },
                              'Restore this version'
                            )
                          )
                        )
                      )
                    )
                  );
                },
              },
            },
            'History'
          )
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
                h(
                  'button',
                  {
                    class: 'icon-btn small',
                    title: 'Remove',
                    on: {
                      click: async () => {
                        try {
                          const all = await call<Skill[]>('chat_skill_remove_file', { name: current!.name, path: f });
                          current = all.find((s) => s.name === current!.name) ?? current;
                          onSaved(all);
                          renderFiles();
                        } catch {
                          // Already shown.
                        }
                      },
                    },
                  },
                  icon('trash', 14)
                )
              )
            )
          : h('p', { class: 'muted small' }, 'None. Templates, reference notes or scripts the instructions mention go here; the assistant reads them when it needs them.')
      );
    };
    renderFiles();
    const ownSkill = Boolean(skill && !builtin && !draft);
    const addFiles = ownSkill
      ? h(
          'div',
          { class: 'skill-file-actions' },
          button({
            variant: 'secondary',
            size: 'sm',
            icon: 'plus',
            label: 'Add files…',
            onClick: async () => {
              try {
                const all = await call<Skill[]>('chat_skill_add_files', { name: skill!.name, folder: null });
                current = all.find((s) => s.name === skill!.name) ?? current;
                onSaved(all);
                renderFiles();
              } catch {
                // Already shown.
              }
            },
          }),
          button({
            variant: 'subtle',
            size: 'sm',
            icon: 'pencil',
            label: 'New text file',
            onClick: async () => {
              const path = await promptText('New file', '', 'Path inside the skill, e.g. templates/report.md');
              if (!path) return;
              try {
                const all = await call<Skill[]>('chat_skill_write_file', { name: skill!.name, path, text: '' });
                current = all.find((s) => s.name === skill!.name) ?? current;
                onSaved(all);
                renderFiles();
                void viewSkillFile(skill!.name, path.trim());
              } catch {
                // Already shown.
              }
            },
          })
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
      h(
        'div',
        { class: 'dialog-actions' },
        remove,
        historyBtn,
        h('span', { class: 'spacer' }),
        h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'),
        h('button', { class: 'btn btn-primary', on: { click: () => void save() } }, 'Save')
      )
    );
  });
}

/** A skill's supporting file, editable when it is text. */
async function viewSkillFile(name: string, path: string): Promise<void> {
  let text: string;
  try {
    text = await call<string>('chat_skill_read_file', { name, path });
  } catch {
    return;
  }
  const binary = /^\S.* is a binary file \(\d+ bytes\) at /.test(text);
  return modal((close) => {
    const area = h('textarea', { class: 'input textarea mono', rows: 18, disabled: binary });
    area.value = text;
    const save = button({
      variant: 'primary',
      label: 'Save',
      onClick: async () => {
        try {
          await call('chat_skill_write_file', { name, path, text: area.value });
          toast('File saved.');
          close();
        } catch {
          // Already shown.
        }
      },
    });
    return h('div', { class: 'dialog-small wide' }, modalHeader(`${name} / ${path}`, close, { flush: true }), area, binary ? null : h('div', { class: 'dialog-actions' }, h('span', { class: 'spacer' }), save));
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

  /**
   * Claude plugins (or a marketplace of them): skills and commands become skills, subagents
   * become agents, `.mcp.json` servers become tool servers under Connectors.
   */
  private async importPlugin(link: string | null) {
    type Imported = { plugins: string[]; skills: string[]; agents: string[]; servers: string[]; hooks: number; skipped: string[] };
    if (link) toast('Downloading…');
    try {
      const out = await call<Imported | null>('chat_import_plugin', { link });
      if (!out) return;
      await reported(this.load());
      const parts = [
        out.skills.length
          ? `${out.skills.length} skill${out.skills.length === 1 ? '' : 's'} (${out.skills
              .slice(0, 6)
              .map((n) => `/${n}`)
              .join(', ')}${out.skills.length > 6 ? '…' : ''})`
          : '',
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
    } catch {
      // Already shown.
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
      entries = await call<Entry[] | null>('chat_plugin_marketplace', { link });
    } catch {
      return;
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
                h(
                  'span',
                  { class: 'market-text' },
                  h('strong', {}, e.name),
                  e.category ? h('small', { class: 'muted' }, ` · ${e.category}`) : null,
                  h('span', { class: 'muted small' }, e.description || (e.link ? '' : 'Its source is not on GitHub; it cannot be fetched from here.'))
                )
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
            await invoke('chat_import_plugin', { link: e.link });
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
          h(
            'button',
            {
              class: 'menu-item',
              on: {
                click: () => {
                  close();
                  void reported(api.importSkill())
                    .then((name) => reported(done(name)))
                    .catch(shown);
                },
              },
            },
            icon('upload', 15),
            h('span', {}, 'From a file…')
          ),
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
                  await call<string>('chat_import_skill_url', { url })
                    .then((name) => reported(done(name)))
                    .catch(shown);
                },
              },
            },
            icon('link', 15),
            h('span', {}, 'From a link (GitHub)…')
          ),
          h(
            'button',
            {
              class: 'menu-item',
              on: {
                click: () => {
                  close();
                  void openSkillCatalog(() => void this.load());
                },
              },
            },
            icon('globe', 15),
            h('span', {}, 'Browse skill catalogues…')
          ),
          h('div', { class: 'menu-sep' }),
          h(
            'button',
            {
              class: 'menu-item',
              on: {
                click: () => {
                  close();
                  void this.importPlugin(null);
                },
              },
            },
            icon('folder', 15),
            h('span', {}, 'Claude plugin from a folder…')
          ),
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
    const record = button({ variant: 'secondary', icon: 'play', label: 'Record a task', title: 'Do a task once while Cortex watches; it becomes a skill you review', onClick: () => void recordTask() });
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
      clear(this.list, this.tab === 'mine' ? emptyState('skill', 'No skills of your own yet', 'Create one, import a SKILL.md, or customize a built-in skill.') : emptyState('skill', 'No match', 'Try another search.'));
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

/**
 * Teach a task by recording (OpenBot): Cortex steps aside while the user does the task once; a
 * Recording pill keeps time and stops it. Screenshots are taken when the screen changes, never
 * keystrokes; see src-tauri/src/chat/recorder.rs.
 */
export async function recordTask() {
  const ok = await confirmDialog(
    'Record a task?',
    'Cortex will step aside and take a screenshot whenever your screen changes, with the name of the window in front (never what you type). Do the task once, then press Stop on the Recording pill. Close anything private first: the screenshots go to your chosen model to write the skill, and are dropped afterwards.',
    'Start recording',
    false
  );
  if (!ok) return;
  try {
    await call('chat_record_start');
  } catch {
    // Already shown.
  }
}

/** After Stop: what the task was, then the model's draft to review and save. */
export function reviewRecording(status: { seconds: number; steps: number }) {
  void modal((close) => {
    const minutes = `${Math.floor(status.seconds / 60)}:${String(status.seconds % 60).padStart(2, '0')}`;
    const note = h('textarea', { class: 'input textarea short', rows: 3, placeholder: 'What was the task, and anything to know? e.g. “Export last month’s invoices as PDF and email them to accounting.”' });
    const error = h('p', { class: 'error-text' });
    const discard = async () => {
      await callQuiet('chat_record_discard');
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
          const draft = await invoke<{ name: string; description: string; body: string }>('chat_record_draft', { model: state.model, note: note.value });
          close();
          void editSkill(
            { ...draft, origin: 'user' } as Skill,
            (skills) => {
              state.skills = skills;
            },
            true
          );
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
