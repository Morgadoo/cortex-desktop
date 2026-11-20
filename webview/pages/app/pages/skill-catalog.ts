/** Skill catalogues to search and add from, and publishing the user's own skills. */
import { tauriInvoke } from '../../../shared/tauri';
import { api, type Skill } from '../core/api';
import { clear, h } from '../ui/h';
import { icon } from '../ui/icons';
import { modal, promptText, toast } from '../ui/overlay';
import { state } from '../core/state';
import { button, modalHeader } from '../ui/ui';
import { call } from '../core/call';

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
export async function openSkillCatalog(onAdded: () => void) {
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
                    const r = await call<{ name: string; files: number; note: string | null }>('chat_install_catalog_skill', { catalog: s.catalog, branch: s.branch, path: s.path });
                    installed.add(r.name);
                    add.textContent = 'Added';
                    toast(`Added ${r.name}${r.files ? ` with ${r.files} files` : ''}.${r.note ? ` ${r.note}` : ''}`);
                    onAdded();
                  } catch {
                    add.removeAttribute('disabled');
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
                    await call('chat_add_skill_catalog', { repo });
                    void load(true);
                  } catch {
                    // Already shown.
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
          const folder = await call<string | null>('chat_export_skill_catalog', { names: [...picked], title: title.value, author: author.value });
          if (!folder) return;
          close();
          toast(`Written to ${folder}. Push that folder to a GitHub repository; anyone can then add it as a catalogue by owner/repo.`);
        } catch {
          // Already shown.
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
