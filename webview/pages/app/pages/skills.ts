/** The Skills page, the skill editor and catalogues, and teaching a task by recording it. */
import { tauriInvoke } from '../../../shared/tauri';
import { api, type Skill } from '../core/api';
import { clear, confirmDialog, h, icon, modal, popover, promptText, toast, clockHour12 } from '../ui/dom';
import { state } from '../core/state';
import { button, emptyState, modalHeader, pageHeader, switchInput, tabs } from '../ui/ui';
import { call, callQuiet, shown } from '../core/call';

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
                          h(
                            'div',
                            { class: 'dialog-actions' },
                            h(
                              'button',
                              {
                                class: 'btn btn-secondary btn-sm',
                                on: {
                                  click: async () => {
                                    onSaved(await tauriInvoke('chat_restore_skill', { name: skill.name, version: v.id }));
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
                          const all = await tauriInvoke<Skill[]>('chat_skill_remove_file', { name: current!.name, path: f });
                          current = all.find((s) => s.name === current!.name) ?? current;
                          onSaved(all);
                          renderFiles();
                        } catch (e) {
                          toast(String(e), 'error');
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
                const all = await tauriInvoke<Skill[]>('chat_skill_add_files', { name: skill!.name, folder: null });
                current = all.find((s) => s.name === skill!.name) ?? current;
                onSaved(all);
                renderFiles();
              } catch (e) {
                toast(String(e), 'error');
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
                const all = await tauriInvoke<Skill[]>('chat_skill_write_file', { name: skill!.name, path, text: '' });
                current = all.find((s) => s.name === skill!.name) ?? current;
                onSaved(all);
                renderFiles();
                void viewSkillFile(skill!.name, path.trim());
              } catch (e) {
                toast(String(e), 'error');
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
    text = await tauriInvoke<string>('chat_skill_read_file', { name, path });
  } catch (e) {
    return void toast(String(e), 'error');
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
          await tauriInvoke('chat_skill_write_file', { name, path, text: area.value });
          toast('File saved.');
          close();
        } catch (e) {
          toast(String(e), 'error');
        }
      },
    });
    return h('div', { class: 'dialog-small wide' }, modalHeader(`${name} / ${path}`, close, { flush: true }), area, binary ? null : h('div', { class: 'dialog-actions' }, h('span', { class: 'spacer' }), save));
  });
}

/** Learning on its own (learning.rs): a weekly knowledge review, and skills from agents' work. */
export function learningRows(): HTMLElement {
  type Learning = { reviewKnowledge: boolean; skillsFromWork: boolean; lastReview?: string | null; nightlyEvals: boolean; lastEvals?: string | null };
  const box = h('div', {});
  void tauriInvoke<Learning>('chat_learning', { change: null })
    .then((l) => {
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
          h(
            'div',
            {},
            h('strong', {}, 'Review my knowledge weekly'),
            h(
              'small',
              { class: 'muted' },
              `Once a week, a background review looks for duplicates, contradictions, stale and sensitive facts, and leaves its proposals in an unread chat. Nothing changes until you answer there.${l.lastReview ? ` Last review: ${new Date(l.lastReview).toLocaleDateString()}.` : ''}`
            )
          ),
          switchInput(l.reviewKnowledge, (v) => void change({ reviewKnowledge: v }))
        ),
        h(
          'div',
          { class: 'setting-row' },
          h(
            'div',
            {},
            h('strong', {}, 'Agents learn skills from their work'),
            h('small', { class: 'muted' }, 'After an agent works through several steps, it considers whether the procedure is worth reusing and, if so, saves it as a skill, turned off until you review it.')
          ),
          switchInput(l.skillsFromWork, (v) => void change({ skillsFromWork: v }))
        ),
        h(
          'div',
          { class: 'setting-row' },
          h(
            'div',
            {},
            h('strong', {}, 'Run my eval suites daily'),
            h(
              'small',
              { class: 'muted' },
              `Once a day, the suites in the evals folder run on your default model, and you are told when a score drops. Each case costs a model call.${l.lastEvals ? ` Last run: ${new Date(l.lastEvals).toLocaleDateString()}.` : ''}`
            )
          ),
          switchInput(l.nightlyEvals, (v) => void change({ nightlyEvals: v }))
        ),
        h(
          'div',
          { class: 'setting-row' },
          h('div', {}, h('strong', {}, 'Eval cases'), h('small', { class: 'muted' }, 'What a good answer does, case by case. You own these: the assistant cannot read or change them.')),
          button({ variant: 'secondary', size: 'sm', label: 'Edit cases…', onClick: () => editEvalCases() })
        ),
        artifactRows()
      );
    })
    .catch(() => undefined);
  return box;
}

type EvalCase = { id: string; prompt: string; tools: string[]; notTools: string[]; facts: string[]; needsExpectation: boolean; note: string };
type EvalSuite = { suite: string; cases: EvalCase[]; lastScore?: number | null };

/** The user's eval suites (evals.rs): each case's prompt and what a good answer does. */
function editEvalCases(): void {
  void modal((close) => {
    const body = h('div', { class: 'version-list' }, h('p', { class: 'muted' }, 'Loading…'));
    const lines = (text: string) =>
      text
        .split(/\n|,/)
        .map((t) => t.trim())
        .filter(Boolean);
    const caseForm = (suite: string, c: EvalCase | null): HTMLElement => {
      const id = h('input', { class: 'input', value: c?.id ?? '', placeholder: 'case-id', disabled: Boolean(c) }) as HTMLInputElement;
      const prompt = h('textarea', { class: 'input', rows: 2, placeholder: 'The message to send' }, c?.prompt ?? '') as HTMLTextAreaElement;
      const tools = h('input', { class: 'input', value: (c?.tools ?? []).join(', '), placeholder: 'Tools it must call, comma-separated' }) as HTMLInputElement;
      const notTools = h('input', { class: 'input', value: (c?.notTools ?? []).join(', '), placeholder: 'Tools it must not call' }) as HTMLInputElement;
      const facts = h('textarea', { class: 'input', rows: 2, placeholder: 'What the answer must contain, one per line' }, (c?.facts ?? []).join('\n')) as HTMLTextAreaElement;
      const save = async () => {
        try {
          render(
            await tauriInvoke<EvalSuite[]>('chat_save_eval_case', {
              suite,
              case: {
                id: id.value.trim(),
                prompt: prompt.value,
                tools: lines(tools.value),
                notTools: lines(notTools.value),
                facts: facts.value
                  .split('\n')
                  .map((f) => f.trim())
                  .filter(Boolean),
                needsExpectation: c?.needsExpectation ?? false,
                note: c?.note ?? '',
              },
            })
          );
          toast('Saved.');
        } catch (e) {
          toast(String(e), 'error');
        }
      };
      const remove = c
        ? h(
            'button',
            {
              class: 'btn btn-danger btn-sm',
              on: {
                click: async () => {
                  if (await confirmDialog(`Delete “${c.id}”?`, 'The case is removed from its suite.', 'Delete')) render(await tauriInvoke<EvalSuite[]>('chat_delete_eval_case', { suite, id: c.id }));
                },
              },
            },
            'Delete'
          )
        : null;
      return h(
        'details',
        { class: 'version', open: !c },
        h('summary', {}, icon('check', 14), h('span', { class: 'grow' }, c ? c.id : 'New case'), c?.needsExpectation ? h('small', { class: 'muted' }, 'waiting for what a right answer contains') : null),
        c?.note ? h('p', { class: 'muted small' }, c.note) : null,
        c ? null : id,
        prompt,
        tools,
        notTools,
        facts,
        h('div', { class: 'dialog-actions' }, remove, h('button', { class: 'btn btn-secondary btn-sm', on: { click: () => void save() } }, 'Save'))
      );
    };
    const render = (suites: EvalSuite[]) => {
      const newSuite = h('input', { class: 'input', placeholder: 'new-suite-name' }) as HTMLInputElement;
      clear(
        body,
        ...suites.map((s) => h('div', {}, h('h4', {}, s.suite, s.lastScore != null ? h('small', { class: 'muted' }, ` · last score ${s.lastScore.toFixed(2)}`) : null), ...s.cases.map((c) => caseForm(s.suite, c)), caseForm(s.suite, null))),
        h(
          'div',
          {},
          h('h4', {}, 'A new suite'),
          newSuite,
          h(
            'div',
            { class: 'dialog-actions' },
            h(
              'button',
              {
                class: 'btn btn-secondary btn-sm',
                on: {
                  click: () => {
                    const name = newSuite.value.trim();
                    if (name) render([...suites, { suite: name, cases: [] }]);
                  },
                },
              },
              'Add suite'
            )
          )
        )
      );
    };
    void tauriInvoke<EvalSuite[]>('chat_eval_suites')
      .then(render)
      .catch((e) => clear(body, h('p', { class: 'error-text' }, String(e))));
    return h('div', { class: 'dialog-small wide' }, modalHeader('Eval cases', close, { flush: true }), body);
  });
}

/**
 * What the assistant made for itself (traces.rs's artifact registry): skills and tools it wrote
 * or learned, how often each is used, and a switch that keeps one however rarely it is used.
 */
function artifactRows(): HTMLElement {
  type Artifact = { kind: string; id: string; status: string; used: number; critical: boolean; provenance?: string | null };
  const box = h('div', {});
  void tauriInvoke<Artifact[]>('chat_artifacts')
    .then((all) => {
      const made = all.filter((a) => (a.kind === 'skill' || a.kind === 'script_tool') && a.provenance);
      clear(
        box,
        h(
          'div',
          { class: 'setting-row' },
          h('div', {}, h('strong', {}, 'What the assistant made'), h('small', { class: 'muted' }, 'Skills and tools the assistant wrote or learned. Unused ones are retired after a month; mark one critical to keep it.'))
        ),
        ...(made.length
          ? made.map((a) =>
              h(
                'div',
                { class: 'setting-row' },
                h('div', {}, h('strong', {}, a.id), h('small', { class: 'muted' }, `${a.kind === 'skill' ? 'Skill' : 'Tool'}, ${a.status}, used ${a.used} times`)),
                h(
                  'label',
                  { class: 'row-gap' },
                  h('small', { class: 'muted' }, 'Critical'),
                  switchInput(a.critical, (v) => {
                    void call('chat_set_artifact_critical', { kind: a.kind, id: a.id, critical: v }).catch(shown);
                  })
                ),
                // A tool the assistant wrote keeps only what it was granted; this takes it all back.
                a.kind === 'script_tool'
                  ? h(
                      'button',
                      {
                        class: 'btn btn-ghost btn-sm',
                        on: {
                          click: async () => {
                            if (!(await confirmDialog(`Revoke ${a.id}'s access?`, 'It keeps working but can no longer reach the network, read or write files, or call other tools, until you approve more.', 'Revoke'))) return;
                            try {
                              await tauriInvoke('chat_revoke_tool', { name: a.id });
                              toast('Access revoked.');
                            } catch (e) {
                              toast(String(e), 'error');
                            }
                          },
                        },
                      },
                      'Revoke access'
                    )
                  : null
              )
            )
          : [h('p', { class: 'muted small' }, 'Nothing yet.')])
      );
    })
    .catch(() => undefined);
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
          h(
            'button',
            {
              class: 'menu-item',
              on: {
                click: () => {
                  close();
                  void api
                    .importSkill()
                    .then(done)
                    .catch((e) => toast(String(e), 'error'));
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
                  await tauriInvoke<string>('chat_import_skill_url', { url })
                    .then(done)
                    .catch((e) => toast(String(e), 'error'));
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
          const draft = await tauriInvoke<{ name: string; description: string; body: string }>('chat_record_draft', { model: state.model, note: note.value });
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
              const add = button({
                variant: 'secondary',
                label: installed.has(s.name) ? 'Added' : 'Add',
                onClick: async () => {
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
                },
              });
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
            h(
              'span',
              { class: 'chip' },
              c,
              c === 'anthropics/skills'
                ? null
                : h(
                    'button',
                    {
                      class: 'chip-x',
                      title: 'Remove this catalogue',
                      on: {
                        click: async () => {
                          await tauriInvoke('chat_remove_skill_catalog', { repo: c });
                          void load(true);
                        },
                      },
                    },
                    icon('close', 12)
                  )
            )
          ),
          h(
            'button',
            {
              class: 'chip add',
              on: {
                click: async () => {
                  const repo = await promptText('Add a skill catalogue', '', 'owner/repo (a GitHub repository with .claude-plugin/marketplace.json)');
                  if (!repo) return;
                  try {
                    await tauriInvoke('chat_add_skill_catalog', { repo });
                    void load(true);
                  } catch (e) {
                    toast(String(e), 'error');
                  }
                },
              },
            },
            icon('plus', 12),
            'Add catalogue'
          )
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
      h('p', { class: 'muted' }, "Skills others publish on GitHub, starting with Anthropic's. Adding one copies its instructions, and its scripts and files where GitHub lists them, into your skills, to read and change like your own."),
      sources,
      h(
        'div',
        { class: 'row-inline' },
        h('div', { class: 'grow' }, search),
        button({ variant: 'secondary', icon: 'retry', label: 'Refresh', onClick: () => void load(true) }),
        button({ variant: 'secondary', icon: 'share', label: 'Publish my skills…', onClick: () => void publishSkills() })
      ),
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
    const go = button({
      variant: 'primary',
      label: 'Choose a folder and publish',
      onClick: async () => {
        try {
          const folder = await tauriInvoke<string | null>('chat_export_skill_catalog', { names: [...picked], title: title.value, author: author.value });
          if (!folder) return;
          close();
          toast(`Written to ${folder}. Push that folder to a GitHub repository; anyone can then add it as a catalogue by owner/repo.`);
        } catch (e) {
          toast(String(e), 'error');
        }
      },
    });
    return h(
      'div',
      { class: 'dialog-small' },
      modalHeader('Publish skills', close, { flush: true }),
      h(
        'p',
        { class: 'muted' },
        'Cortex writes the chosen skills as a catalogue (a .claude-plugin/marketplace.json and a folder per skill). Put it in a GitHub repository, and others add it in Cortex, Claude Code or any app that reads that format.'
      ),
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
