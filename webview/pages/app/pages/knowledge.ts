/** The Knowledge page: pages the model and the user write and keep, and the memories beside them. */
import { tauriInvoke } from '../../../shared/tauri';
import { api, type Memories } from '../core/api';
import { clear, h } from '../ui/h';
import { icon } from '../ui/icons';
import { markdown } from '../ui/markdown';
import { confirmDialog, modal } from '../ui/overlay';
import { improveButton } from '../chat/improve';
import { restoreButton } from '../canvas/versions';
import { navigate, state } from '../core/state';
import { button, emptyState, iconButton, modalHeader, pageHeader, switchInput, tabs } from '../ui/ui';
import { learningRows } from './learning';
import { call, reported, shown } from '../core/call';

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
        onSaved(await reported(knowledgeApi.save(page?.id ?? null, title.value, area.value)));
        close();
      } catch {
        // Already shown.
      }
    };
    const improve = improveButton(area, renderPreview);
    return h(
      'div',
      { class: 'canvas-modal' },
      modalHeader(title, close),
      h('div', { class: 'canvas-split' }, area, preview),
      page?.sources.length ? h('p', { class: 'muted knowledge-sources' }, `Sources: ${page.sources.join(' · ')}`) : null,
      h(
        'div',
        { class: 'dialog-actions canvas-actions' },
        improve,
        h('span', { class: 'hint' }, 'Markdown'),
        h('span', { class: 'spacer' }),
        button({ variant: 'secondary', label: 'Cancel', onClick: close }),
        button({ variant: 'primary', label: 'Save', onClick: () => void save() })
      )
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
    // Every change to the pages or the facts keeps what it replaced; this brings one back.
    const history = restoreButton(
      () => (this.tab === 'pages' ? { kind: 'knowledge', id: 'pages' } : { kind: 'memories', id: 'all' }),
      () => {
        void api.memories().then((m) => this.set(m));
        void knowledgeApi.pages().then((p) => this.setPages(p));
      }
    );
    this.el.append(pageHeader('Knowledge', 'Pages the assistant recalls when they are relevant, and short facts it always keeps in mind. It writes them as you chat; you can edit both.', newPage, history, clearAll), this.body);
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
          h(
            'div',
            { class: 'knowledge-card-head' },
            icon('book', 16),
            h('strong', {}, p.title),
            h('span', { class: 'spacer' }),
            iconButton({
              icon: 'trash',
              title: 'Delete page',
              size: 'sm',
              onClick: async (e) => {
                e.stopPropagation();
                if (await confirmDialog(`Delete “${p.title}”?`, 'The assistant will no longer recall it.', 'Delete')) this.setPages(await knowledgeApi.remove(p.id));
              },
            })
          ),
          h('p', { class: 'muted knowledge-excerpt' }, p.body.replace(/[#*_`>]/g, '').slice(0, 220) || 'Empty page'),
          h(
            'div',
            { class: 'knowledge-meta' },
            h('span', {}, p.author === 'assistant' ? 'Written by the assistant' : 'Written by you'),
            h('span', {}, new Date(p.updatedAt).toLocaleDateString()),
            p.sources.length ? h('span', {}, `${p.sources.length} source${p.sources.length === 1 ? '' : 's'}`) : null
          )
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
        this.set(await reported(api.addMemory(input.value)));
      } catch {
        // Already shown.
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
      h(
        'button',
        {
          class: `chip-btn${this.memorySource === value ? ' active' : ''}`,
          on: {
            click: () => {
              this.memorySource = value;
              this.render();
            },
          },
        },
        label
      );
    const sinceChip = (days: number, label: string) =>
      h(
        'button',
        {
          class: `chip-btn${this.memoryDays === days ? ' active' : ''}`,
          on: {
            click: () => {
              this.memoryDays = days;
              this.render();
            },
          },
        },
        label
      );
    const filters =
      m.items.length > 3
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
              const pick = h(
                'select',
                { class: 'memory-category' },
                h('option', { value: '' }, 'Every subject'),
                MEMORY_CATEGORIES.map(([v, l]) => h('option', { value: v, selected: this.memoryCategory === v }, l))
              );
              pick.addEventListener('change', () => {
                this.memoryCategory = pick.value;
                this.render();
              });
              return pick;
            })()
          )
        : null;
    const since = this.memoryDays ? Date.now() - this.memoryDays * 86_400_000 : 0;
    const words = this.memoryQuery.toLowerCase().split(/\s+/).filter(Boolean);
    const matching = m.items.filter(
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
          matching.length ? null : h('p', { class: 'muted' }, 'No fact matches.'),
          [...matching].reverse().map((item) => {
            const text = h('div', { class: 'memory-text' }, item.text);
            const edit = h('button', { class: 'icon-btn small', title: 'Edit' }, icon('edit', 15));
            edit.addEventListener('click', () => {
              const field = h('input', { class: 'input small', value: item.text });
              const commit = async () => {
                try {
                  this.set(await reported(api.updateMemory(item.id, field.value)));
                } catch {
                  // Already shown.
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
              void call<Memories>('chat_file_memory', { id: item.id, category, importance, confirmed })
                .then((m) => this.set(m))
                .catch(shown);
            const category = h(
              'select',
              { class: 'memory-category', title: 'What it is about' },
              h('option', { value: '' }, 'Other'),
              MEMORY_CATEGORIES.map(([v, l]) => h('option', { value: v, selected: item.category === v }, l))
            );
            category.addEventListener('change', () => file(category.value, item.importance ?? 2));
            const weight = item.importance ?? 2;
            const importance = h('button', { class: `memory-weight w${weight}`, title: `Importance: ${['', 'a detail', 'normal', 'always keep in mind'][weight]} (click to change)` }, '●'.repeat(weight) + '○'.repeat(3 - weight));
            importance.addEventListener('click', () => file(item.category ?? '', (weight % 3) + 1));
            const inferred =
              item.confidence === 'inferred' ? h('button', { class: 'memory-inferred', title: 'The assistant concluded this. Click to confirm it is right.', on: { click: () => file(item.category ?? '', weight, true) } }, 'inferred') : null;
            const source =
              item.conversation && state.conversations.some((c) => c.id === item.conversation)
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
