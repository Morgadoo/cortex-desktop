/** Context pages (connectors, skills, scheduled tasks…) and the instructions dialog. */
import { api, type ConnectorManifest, type Instructions } from './api';
import { clear, h, icon, markdown, modal, toast } from './dom';
import { state, type Page } from './state';
import { openSettings } from './settings';
import { listen, tauriInvoke } from '../../shared/tauri';
import { CONNECTOR_CATEGORIES, FEATURED_MCP_SERVERS, type ConnectorCatalogEntry } from '../../shared/connector-catalog';
import { emptyState, modalHeader } from './ui';

const TONES: { id: string; label: string; description: string }[] = [
  { id: '', label: 'Default', description: 'Natural style, no personalization.' },
  { id: 'formal', label: 'Polite', description: 'Professional and courteous.' },
  { id: 'casual', label: 'Casual', description: 'Relaxed, like a friendly colleague.' },
  { id: 'empathetic', label: 'Empathetic', description: 'Warm; acknowledges feelings first.' },
  { id: 'objective', label: 'Objective', description: 'Neutral, factual, weighs trade-offs.' },
  { id: 'concise', label: 'Concise', description: 'Leads with the answer, skips filler.' },
  { id: 'witty', label: 'Witty', description: 'Light and good-humoured where it fits.' },
  { id: 'motivational', label: 'Motivational', description: 'Encouraging and energising.' },
  { id: 'reflective', label: 'Reflective', description: 'Surfaces assumptions and second-order effects.' },
];

const STYLE_CHIPS = ['Be concise', 'Use bullet points', 'Explain step by step', 'Cite your sources', 'Ask before assuming', 'Use metric units', 'Answer in Portuguese'];

const PREVIEW_PROMPT = 'Recommend a book for me to read.';

export function openInstructions(): Promise<void> {
  const draft: Instructions = { ...state.instructions };
  return modal((close) => {
    const toneSelect = h(
      'select',
      { class: 'input' },
      TONES.map((t) => h('option', { value: t.id, selected: t.id === draft.tone }, t.label))
    );
    const toneHint = h('p', { class: 'hint' }, TONES.find((t) => t.id === draft.tone)?.description ?? '');
    toneSelect.addEventListener('change', () => {
      draft.tone = toneSelect.value;
      toneHint.textContent = TONES.find((t) => t.id === draft.tone)?.description ?? '';
    });
    const name = h('input', { class: 'input', value: draft.name, placeholder: 'How should the assistant call you?' });
    name.addEventListener('input', () => (draft.name = name.value));
    const custom = h('textarea', { class: 'input textarea', rows: 6, placeholder: 'Be concise, use a friendly tone, etc.' });
    custom.value = draft.custom;
    custom.addEventListener('input', () => (draft.custom = custom.value));
    const chips = h(
      'div',
      { class: 'chip-row' },
      STYLE_CHIPS.map((chip) =>
        h(
          'button',
          {
            class: 'chip add',
            on: {
              click: () => {
                custom.value = custom.value.trim() ? `${custom.value.trim()}\n${chip}.` : `${chip}.`;
                draft.custom = custom.value;
              },
            },
          },
          icon('plus', 13),
          chip
        )
      )
    );

    const previewBody = h('div', { class: 'preview-body' });
    const runPreview = async () => {
      if (!state.model) {
        clear(previewBody, h('p', { class: 'muted' }, 'Choose a model to preview replies.'));
        return;
      }
      clear(previewBody, h('div', { class: 'bubble preview-q' }, PREVIEW_PROMPT), h('div', { class: 'status-line' }, h('span', { class: 'dots' }), 'Writing'));
      try {
        const reply = await api.previewInstructions(draft, state.model, PREVIEW_PROMPT);
        const answer = h('div', { class: 'markdown' });
        answer.append(markdown(reply));
        clear(previewBody, h('div', { class: 'bubble preview-q' }, PREVIEW_PROMPT), answer);
      } catch (e) {
        clear(previewBody, h('div', { class: 'bubble preview-q' }, PREVIEW_PROMPT), h('p', { class: 'error-text' }, String(e)));
      }
    };
    const refresh = h('button', { class: 'icon-btn round', title: 'Preview again', on: { click: () => void runPreview() } }, icon('retry', 16));
    clear(previewBody, h('div', { class: 'bubble preview-q' }, PREVIEW_PROMPT), h('p', { class: 'muted' }, 'Press ↻ to see a reply with these instructions.'));

    const save = async () => {
      await api.saveInstructions(draft);
      state.instructions = { ...draft };
      toast('Instructions saved.');
      close();
    };

    return h(
      'div',
      { class: 'instructions' },
      modalHeader('Instructions', close),
      h(
        'div',
        { class: 'instructions-grid' },
        h(
          'div',
          { class: 'instructions-form' },
          h('label', { class: 'field-label' }, 'Your name'),
          name,
          h('label', { class: 'field-label' }, 'Tone'),
          toneSelect,
          toneHint,
          h('hr'),
          h('label', { class: 'field-label' }, 'Additional instructions'),
          custom,
          chips,
          h('div', { class: 'grow' }),
          h('button', { class: 'btn btn-primary', on: { click: () => void save() } }, 'Save')
        ),
        h('div', { class: 'preview' }, h('div', { class: 'preview-tools' }, refresh), previewBody)
      )
    );
  }, true);
}

// ---------------------------------------------------------------------------------------------

function pageHeader(title: string, subtitle: string, ...actions: (HTMLElement | null)[]): HTMLElement {
  return h('header', { class: 'page-header' }, h('div', {}, h('h1', {}, title), h('p', { class: 'muted' }, subtitle)), h('div', { class: 'page-actions' }, actions));
}

const READINESS: Record<string, string> = {
  ready: 'Ready',
  blocked: 'Needs setup',
  needs_auth: 'Sign-in needed',
};

/** Connectors the user has expanded, kept while the list re-renders. */
const openConnectors = new Set<string>();

export class ConnectorsPage {
  readonly el = h('div', { class: 'page' });
  readonly key = 'page:connectors';
  private connectors: ConnectorManifest[] = [];
  private filter = '';
  private tab: 'enabled' | 'all' | 'catalog' = 'all';
  private list = h('div', { class: 'connector-list' });
  private unlisten: Promise<() => void>;

  constructor() {
    const search = h('input', { class: 'input search-input', placeholder: 'Search connectors…' });
    search.addEventListener('input', () => {
      this.filter = search.value.toLowerCase();
      this.render();
    });
    const tabs = h('div', { class: 'tabs' });
    const renderTabs = () =>
      clear(
        tabs,
        (['all', 'enabled', 'catalog'] as const).map((t) =>
          h(
            'button',
            {
              class: `tab${this.tab === t ? ' active' : ''}`,
              on: {
                click: () => {
                  this.tab = t;
                  renderTabs();
                  this.render();
                },
              },
            },
            t === 'all' ? `Yours ${this.connectors.length}` : t === 'enabled' ? `Enabled ${this.connectors.filter((c) => c.enabled).length}` : `Catalog ${FEATURED_MCP_SERVERS.length}`
          )
        )
      );
    this.renderTabs = renderTabs;
    const addBtn = h('button', { class: 'btn btn-primary' }, icon('plus', 16), 'Add MCP server');
    addBtn.addEventListener('click', () => void openSettings('permissions'));
    this.el.append(
      pageHeader('Connectors', 'Give the assistant your tools: files, the terminal, the web and any MCP server.', addBtn),
      h('div', { class: 'toolbar' }, h('div', { class: 'search-wrap' }, icon('search', 16), search)),
      tabs,
      this.list
    );
    this.unlisten = listen('connectors:changed', () => void this.load());
    void this.load();
  }

  private renderTabs = () => {};

  async load() {
    try {
      this.connectors = await api.connectors();
    } catch (e) {
      clear(this.list, emptyState('plug', 'Connectors are unavailable', String(e)));
      return;
    }
    this.renderTabs();
    this.render();
  }

  private render() {
    if (this.tab === 'catalog') {
      this.renderCatalog();
      return;
    }
    const shown = this.connectors.filter((c) => (this.tab === 'all' || c.enabled) && (!this.filter || `${c.display_name} ${c.description ?? ''}`.toLowerCase().includes(this.filter)));
    if (!shown.length) {
      clear(this.list, emptyState('plug', 'Bring your tools to the AI', 'Connect apps and data sources so the assistant can search, answer and act with the context you use every day.'));
      return;
    }
    clear(
      this.list,
      shown.map((c) => this.row(c))
    );
  }

  /** Ready-made MCP servers, grouped by category; adding one registers it for OAuth sign-in. */
  private renderCatalog() {
    const installed = new Set(this.connectors.map((c) => c.id));
    const entries = FEATURED_MCP_SERVERS.filter((e) => !this.filter || `${e.name} ${e.description}`.toLowerCase().includes(this.filter));
    const groups = CONNECTOR_CATEGORIES.map((cat) => ({ cat, items: entries.filter((e) => e.category === cat.id) })).filter((g) => g.items.length);
    clear(
      this.list,
      groups.length ? null : emptyState('plug', 'No match', 'Try another search.'),
      groups.flatMap((g) => [
        h('div', { class: 'section-title' }, g.cat.label),
        h(
          'div',
          { class: 'catalog-grid' },
          g.items.map((entry) => this.catalogCard(entry, installed.has(entry.id)))
        ),
      ])
    );
  }

  private catalogCard(entry: ConnectorCatalogEntry, installed: boolean): HTMLElement {
    const action = installed
      ? h('span', { class: 'status-badge ok' }, 'Added')
      : entry.url
        ? h('button', { class: 'btn btn-secondary btn-sm' }, icon('plus', 14), 'Add')
        : h('span', { class: 'muted small-note', title: 'Runs a local command; add it under Settings → Permissions → MCP servers.' }, 'Manual setup');
    if (!installed && entry.url && action instanceof HTMLButtonElement) {
      action.addEventListener('click', async () => {
        action.disabled = true;
        try {
          await tauriInvoke('save_mcp_server', {
            server: {
              id: entry.server_id ?? entry.id.replace(/^mcp-/, ''),
              display_name: entry.name,
              description: entry.description,
              transport: 'http',
              url: entry.url,
              auth: { type: 'oauth', scopes: [] },
              request_headers: {},
            },
          });
          toast(`${entry.name} added. Connect it to sign in.`);
          this.tab = 'all';
          this.renderTabs();
          await this.load();
        } catch (e) {
          action.disabled = false;
          toast(String(e), 'error');
        }
      });
    }
    return h('div', { class: 'catalog-card' }, h('img', { class: 'connector-icon', src: entry.iconSrc, alt: '' }), h('div', { class: 'catalog-text' }, h('strong', {}, entry.name), h('small', { class: 'muted' }, entry.description)), action);
  }

  private row(c: ConnectorManifest): HTMLElement {
    const toggle = h('input', { type: 'checkbox', class: 'switch', checked: c.enabled, title: c.enabled ? 'Disable' : 'Enable' });
    toggle.addEventListener('change', async () => {
      try {
        await api.setConnectorEnabled(c.id, toggle.checked);
        c.enabled = toggle.checked;
        this.renderTabs();
      } catch (e) {
        toggle.checked = !toggle.checked;
        toast(String(e), 'error');
      }
    });
    const ready = c.readiness.state === 'ready';
    // Stays open across re-renders (a policy change reloads the list).
    const details = h('details', { class: 'connector', open: openConnectors.has(c.id) });
    details.addEventListener('toggle', () => (details.open ? openConnectors.add(c.id) : openConnectors.delete(c.id)));
    details.append(
      h(
        'summary',
        {},
        c.icon && c.icon.startsWith('data:')
          ? h('img', { class: 'connector-icon', src: c.icon, alt: '' })
          : h('span', { class: 'connector-icon fallback' }, icon(c.id === 'bash' ? 'terminal' : c.id === 'filesystem' ? 'file' : c.id === 'browser' ? 'globe' : 'plug', 18)),
        h('span', { class: 'connector-text' }, h('strong', {}, c.display_name), h('small', { class: 'muted' }, c.description ?? `${c.tools.length} tools`)),
        h('span', { class: `status-badge ${ready ? 'ok' : 'warn'}` }, READINESS[c.readiness.state] ?? c.readiness.state),
        !ready && c.id.startsWith('mcp-') ? this.connectButton(c) : null,
        toggle
      ),
      h(
        'div',
        { class: 'tool-list' },
        !ready && c.readiness.reason ? h('p', { class: 'hint' }, c.readiness.reason) : null,
        // As on Vibe's connector page: read-only and interactive functions apart, each group with
        // its own "Always allow", and every function's own setting under it.
        (
          [
            ['Interactive', c.tools.filter((t) => !t.read_only)],
            ['Read-only', c.tools.filter((t) => t.read_only)],
          ] as const
        )
          .filter(([, tools]) => tools.length)
          .map(([label, tools]) => {
            const selects: HTMLSelectElement[] = [];
            const rows = tools.map((t) => {
              const select = h(
                'select',
                { class: 'input small' },
                h('option', { value: 'auto_approve', selected: t.policy === 'auto_approve' }, 'Allow'),
                h('option', { value: 'ask', selected: t.policy === 'ask' }, 'Ask'),
                h('option', { value: 'disable', selected: t.policy === 'disable' }, 'Off')
              );
              selects.push(select);
              select.addEventListener('change', () => {
                void api.setToolPolicy(c.id, t.name, select.value).catch((e) => toast(String(e), 'error'));
                all.checked = selects.every((x) => x.value === 'auto_approve');
              });
              return h('div', { class: 'tool-row' }, h('div', {}, h('strong', {}, t.title ?? t.name), h('small', { class: 'muted' }, t.description)), select);
            });
            const all = h('input', { type: 'checkbox', class: 'switch', checked: tools.every((t) => t.policy === 'auto_approve'), title: `Always allow every ${label.toLowerCase()} function` });
            all.addEventListener('change', async () => {
              const value = all.checked ? 'auto_approve' : 'ask';
              try {
                await Promise.all(tools.map((t) => api.setToolPolicy(c.id, t.name, value)));
                selects.forEach((x) => (x.value = value));
              } catch (e) {
                toast(String(e), 'error');
              }
            });
            return h(
              'div',
              { class: 'tool-group' },
              h(
                'div',
                { class: 'tool-group-head' },
                h('strong', {}, `${label} functions`),
                h('span', { class: 'muted' }, String(tools.length)),
                h('span', { class: 'spacer' }),
                h('label', { class: 'tool-group-all' }, h('span', {}, 'Always allow'), all)
              ),
              rows
            );
          }),
        h('div', { class: 'tool-list-foot' }, h('button', { class: 'btn btn-secondary btn-sm', on: { click: () => void this.load().then(() => toast('Tools refreshed.')) } }, icon('retry', 14), 'Refresh tools'))
      )
    );
    return details;
  }

  /** Starts the connector's sign-in (OAuth in the browser) or connection. */
  private connectButton(c: ConnectorManifest): HTMLElement {
    const button = h('button', { class: 'btn btn-secondary btn-sm' }, 'Connect');
    button.addEventListener('click', async (e) => {
      e.preventDefault();
      e.stopPropagation();
      button.textContent = 'Connecting…';
      try {
        await tauriInvoke('connect_connector', { connectorId: c.id });
        toast(`${c.display_name} connected.`);
      } catch (err) {
        toast(String(err), 'error');
      }
      await this.load();
    });
    return button;
  }

  update() {}

  dispose() {
    void this.unlisten.then((f) => f());
  }
}

/** Sections that exist in the navigation but have no local implementation yet. */
const PLACEHOLDERS: Record<Page, { title: string; subtitle: string; icon: string; empty: string; body: string }> = {
  connectors: { title: 'Connectors', subtitle: '', icon: 'plug', empty: '', body: '' },
  chats: { title: 'Chats', subtitle: '', icon: 'chat', empty: '', body: '' },
  resources: { title: 'Resources', subtitle: '', icon: 'outputs', empty: '', body: '' },
  skills: {
    title: 'Skills',
    subtitle: 'Reusable instructions the assistant can follow. Set them up once, use them whenever you need.',
    icon: 'skill',
    empty: 'No skills yet',
    body: 'Local skills are on the roadmap. Until then, put standing guidance in Instructions or in a project.',
  },
  scheduled: {
    title: 'Scheduled tasks',
    subtitle: 'Run prompts on a schedule.',
    icon: 'clock',
    empty: 'Nothing scheduled',
    body: 'Scheduled prompts are coming to the native app. The scheduling engine is in place; prompts and runs are next.',
  },
  workflows: {
    title: 'Workflows',
    subtitle: 'Step-by-step recipes for work you repeat.',
    icon: 'workflow',
    empty: 'No workflows yet',
    body: 'Create one from the + menu.',
  },
  apps: {
    title: 'Apps',
    subtitle: 'Small apps the assistant builds for you.',
    icon: 'apps',
    empty: 'No apps yet',
    body: 'This section is a preview and not available yet.',
  },
  agents: {
    title: 'Agents',
    subtitle: 'Assistants with their own instructions, tools and model.',
    icon: 'sparkle',
    empty: 'No agents yet',
    body: 'Agents are not available yet. Projects already give a set of conversations shared instructions.',
  },
  knowledge: {
    title: 'Knowledge',
    subtitle: 'What the assistant remembers about you.',
    icon: 'book',
    empty: 'No knowledge yet',
    body: 'Memory is not available yet. Give the assistant access to a folder instead from the + menu.',
  },
  libraries: {
    title: 'Libraries',
    subtitle: 'Document collections the assistant can search.',
    icon: 'library',
    empty: 'No libraries yet',
    body: 'Libraries are not available yet. Give the assistant access to a folder of documents from the + menu.',
  },
};

export class PlaceholderPage {
  readonly el = h('div', { class: 'page' });
  readonly key: string;
  constructor(page: Page) {
    this.key = `page:${page}`;
    const info = PLACEHOLDERS[page];
    const instructions = h('button', { class: 'btn btn-primary', on: { click: () => void openInstructions() } }, icon('book', 16), 'Instructions');
    this.el.append(pageHeader(info.title, info.subtitle, page === 'skills' || page === 'knowledge' ? instructions : null), emptyState(info.icon, info.empty, info.body));
  }
  update() {}
}
