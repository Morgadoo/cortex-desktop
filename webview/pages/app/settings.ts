/** The settings dialog: profile, providers, permissions, general and appearance. */
import { tauriInvoke } from '../../shared/tauri';
import { api, type AppSettings, type Provider } from './api';
import { clear, h, icon, modal, toast } from './dom';
import { changed, state } from './state';

export type Section = 'profile' | 'providers' | 'permissions' | 'general' | 'appearance';

const SECTIONS: { id: Section; label: string; icon: string; group: string }[] = [
  { id: 'profile', label: 'Profile', icon: 'user', group: 'General settings' },
  { id: 'providers', label: 'Providers & models', icon: 'key', group: 'General settings' },
  { id: 'permissions', label: 'Permissions', icon: 'shield', group: 'App settings' },
  { id: 'general', label: 'General', icon: 'monitor', group: 'App settings' },
  { id: 'appearance', label: 'Appearance', icon: 'palette', group: 'App settings' },
];

export type Theme = 'light' | 'dark' | 'system';

export function applyTheme(theme: Theme) {
  const dark = theme === 'dark' || (theme === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches);
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
}

export function storedTheme(): Theme {
  try {
    const value = localStorage.getItem('theme');
    if (value === 'light' || value === 'dark' || value === 'system') return value;
  } catch {
    /* storage unavailable: follow the system */
  }
  return 'system';
}

export function setTheme(theme: Theme) {
  try {
    localStorage.setItem('theme', theme);
  } catch {
    /* not persisted, still applied */
  }
  applyTheme(theme);
}

/** Reloads models after providers change, keeping the chosen one when it still exists. */
export async function reloadModels() {
  const { models, defaultModel } = await api.models();
  state.models = models;
  if (!state.model || !models.some((m) => m.reference === state.model)) state.model = defaultModel;
  changed();
}

export function openSettings(section: Section = 'profile'): Promise<void> {
  let current = section;
  return modal((close) => {
    const nav = h('nav', { class: 'settings-nav' });
    const body = h('div', { class: 'settings-body' });
    const name = state.instructions.name || 'You';
    const renderNav = () => {
      clear(nav, h('div', { class: 'settings-user' }, h('span', { class: 'avatar' }, initials(name)), h('div', {}, h('strong', {}, name), h('small', { class: 'muted' }, 'Local profile'))));
      let group = '';
      for (const s of SECTIONS) {
        if (s.group !== group) {
          group = s.group;
          nav.append(h('div', { class: 'nav-heading' }, group));
        }
        nav.append(h('button', { class: `nav-item${s.id === current ? ' active' : ''}`, on: { click: () => { current = s.id; renderNav(); void renderBody(); } } }, icon(s.icon, 16), s.label));
      }
    };
    const renderBody = async () => {
      const title = SECTIONS.find((s) => s.id === current)!.label;
      clear(body, h('div', { class: 'modal-head' }, h('h2', {}, title), h('button', { class: 'icon-btn', title: 'Close', on: { click: close } }, icon('close', 18))));
      const content = h('div', { class: 'settings-content' });
      body.append(content);
      try {
        if (current === 'profile') renderProfile(content);
        if (current === 'providers') await renderProviders(content);
        if (current === 'permissions') await renderPermissions(content);
        if (current === 'general') await renderGeneral(content);
        if (current === 'appearance') renderAppearance(content);
      } catch (e) {
        content.append(h('p', { class: 'error-text' }, String(e)));
      }
    };
    renderNav();
    void renderBody();
    return h('div', { class: 'settings' }, nav, body);
  }, true);
}

function initials(name: string): string {
  return name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0]!.toUpperCase())
    .join('') || 'U';
}

function card(...children: (HTMLElement | null)[]): HTMLElement {
  return h('section', { class: 'card' }, children);
}

function row(label: string, description: string, control: HTMLElement): HTMLElement {
  return h('div', { class: 'setting-row' }, h('div', {}, h('strong', {}, label), description ? h('small', { class: 'muted' }, description) : null), control);
}

function switchInput(checked: boolean, onChange: (value: boolean) => void): HTMLInputElement {
  const input = h('input', { type: 'checkbox', class: 'switch', checked });
  input.addEventListener('change', () => onChange(input.checked));
  return input;
}

function renderProfile(content: HTMLElement) {
  const name = h('input', { class: 'input', value: state.instructions.name, placeholder: 'Your name' });
  const save = h('button', { class: 'btn btn-primary' }, 'Save');
  save.addEventListener('click', async () => {
    state.instructions = { ...state.instructions, name: name.value.trim() };
    await api.saveInstructions(state.instructions);
    toast('Profile saved.');
    changed();
  });
  content.append(
    card(
      h('h3', {}, 'Your profile'),
      h('p', { class: 'muted' }, 'Everything stays on this device. Conversations are saved as files in the app’s config folder; API keys are stored in the OS keychain.'),
      h('label', { class: 'field-label' }, 'Name'),
      name,
      h('p', { class: 'hint' }, 'Used in the greeting and shared with the model so it can address you.'),
      h('div', { class: 'card-actions' }, save)
    )
  );
}

async function renderProviders(content: HTMLElement) {
  const providers = await api.providers();
  content.append(
    h('p', { class: 'muted intro' }, 'Use any provider you have a key for, or a model running on your own machine. Keys never leave the OS keychain except to call their provider.')
  );
  const list = h('div', { class: 'provider-list' });
  content.append(list);
  const sorted = [...providers].sort((a, b) => Number(b.ready) - Number(a.ready));
  for (const provider of sorted) list.append(providerCard(provider));
}

function providerCard(provider: Provider): HTMLElement {
  const details = h('details', { class: 'card provider', open: false });
  const status = h('span', { class: `status-badge ${provider.ready ? 'ok' : ''}` }, provider.ready ? 'Active' : provider.settings.hasKey ? 'Disabled' : provider.local ? 'Local' : 'Not set up');
  details.append(h('summary', {}, h('strong', {}, provider.label), provider.local ? h('span', { class: 'tag' }, 'on this device') : null, h('span', { class: 'spacer' }), status, icon('chevron', 16)));

  const key = h('input', {
    class: 'input',
    type: 'password',
    placeholder: provider.settings.hasKey
      ? '•••••••• saved in keychain'
      : provider.envKey
        ? `Using ${provider.envKey} from the environment`
        : provider.requiresKey
          ? 'API key'
          : 'API key (optional)',
    autocomplete: 'off',
  });
  const baseUrl = h('input', { class: 'input', value: provider.settings.baseUrl ?? '', placeholder: provider.defaultBaseUrl });
  const models = h('input', { class: 'input', value: provider.settings.customModels.join(', '), placeholder: provider.local ? 'llama3.2, qwen3:8b' : 'Extra model ids, comma separated' });
  const enabled = switchInput(provider.settings.enabled || !provider.settings.hasKey, () => {});
  const result = h('p', { class: 'hint' });

  const save = async (clearKey = false) => {
    try {
      const saved = await api.saveProvider({
        vendor: provider.vendor,
        enabled: enabled.checked,
        baseUrl: baseUrl.value,
        customModels: models.value.split(',').map((m) => m.trim()).filter(Boolean),
        apiKey: key.value || undefined,
        clearKey,
      });
      Object.assign(provider, saved);
      key.value = '';
      key.placeholder = saved.settings.hasKey ? '•••••••• saved in keychain' : 'API key';
      status.textContent = saved.ready ? 'Active' : saved.settings.hasKey ? 'Disabled' : 'Not set up';
      status.classList.toggle('ok', saved.ready);
      await reloadModels();
      result.textContent = saved.ready ? `Saved. ${state.models.filter((m) => m.vendor === provider.vendor).length} models available.` : 'Saved.';
    } catch (e) {
      result.textContent = String(e);
      result.classList.add('error-text');
    }
  };

  const test = h('button', { class: 'btn btn-ghost' }, 'Test');
  test.addEventListener('click', async () => {
    const model = state.models.find((m) => m.vendor === provider.vendor);
    if (!model) {
      result.textContent = 'Save the provider first; no model of it is available yet.';
      return;
    }
    result.textContent = `Asking ${model.name}…`;
    try {
      const reply = await api.testModel(model.reference);
      result.textContent = `✓ ${model.name} answered: “${reply.trim().slice(0, 80)}”`;
    } catch (e) {
      result.textContent = `✗ ${String(e)}`;
    }
  });

  details.append(
    h(
      'div',
      { class: 'provider-form' },
      row('Enabled', 'Offer this provider’s models in the composer.', enabled),
      h('label', { class: 'field-label' }, 'API key', provider.keyUrl ? h('a', { href: provider.keyUrl, target: '_blank', class: 'link' }, 'Get a key') : null),
      key,
      h('label', { class: 'field-label' }, 'Base URL'),
      baseUrl,
      h('p', { class: 'hint' }, provider.local ? 'Where the server listens.' : 'Leave empty for the official endpoint; set it for a proxy or a compatible gateway.'),
      h('label', { class: 'field-label' }, 'Extra models'),
      models,
      result,
      h(
        'div',
        { class: 'card-actions' },
        provider.settings.hasKey ? h('button', { class: 'btn btn-ghost danger', on: { click: () => void save(true) } }, 'Remove key') : null,
        test,
        h('button', { class: 'btn btn-primary', on: { click: () => void save() } }, 'Save')
      )
    )
  );
  return details;
}

async function renderPermissions(content: HTMLElement) {
  const settings = await api.appSettings();
  const persist = async (next: AppSettings) => {
    try {
      await api.saveAppSettings(next);
    } catch (e) {
      toast(String(e), 'error');
    }
  };
  const roots = h('div', { class: 'root-list' });
  const renderRoots = () =>
    clear(
      roots,
      settings.filesystem_allowed_roots.length
        ? settings.filesystem_allowed_roots.map((root) =>
            h(
              'div',
              { class: 'root-row' },
              icon('folder', 16),
              h('span', { class: 'root-path' }, root),
              h('button', { class: 'icon-btn small', title: 'Remove', on: { click: async () => { settings.filesystem_allowed_roots = settings.filesystem_allowed_roots.filter((r) => r !== root); renderRoots(); await persist(settings); } } }, icon('close', 14))
            )
          )
        : h('p', { class: 'muted' }, 'No folders yet. The assistant cannot see your files.')
    );
  renderRoots();
  const add = h('button', { class: 'btn btn-ghost' }, icon('plus', 16), 'Add folder');
  add.addEventListener('click', async () => {
    const dir = await api.pickDirectory();
    if (dir && !settings.filesystem_allowed_roots.includes(dir)) {
      settings.filesystem_allowed_roots.push(dir);
      renderRoots();
      await persist(settings);
    }
  });

  content.append(
    card(
      h('h3', {}, 'Files'),
      h('p', { class: 'muted' }, 'Folders the assistant may search and read.'),
      roots,
      h('div', { class: 'card-actions start' }, add),
      row('Allow editing files', 'Lets the assistant create and change files in these folders. Each change still follows the composer’s approval mode.', switchInput(settings.filesystem_write_enabled, (v) => { settings.filesystem_write_enabled = v; void persist(settings); }))
    ),
    card(
      h('h3', {}, 'Terminal'),
      row('Allow shell commands', 'Lets the assistant run commands on this computer. Every command asks first unless you allow it.', switchInput(settings.bash_enabled, (v) => { settings.bash_enabled = v; void persist(settings); }))
    ),
    mcpCard()
  );
}

function mcpCard(): HTMLElement {
  const name = h('input', { class: 'input', placeholder: 'Name, e.g. GitHub' });
  const kind = h('select', { class: 'input' }, h('option', { value: 'http' }, 'Remote server (URL)'), h('option', { value: 'stdio' }, 'Local command'));
  const target = h('input', { class: 'input', placeholder: 'https://example.com/mcp' });
  const auth = h('select', { class: 'input' }, h('option', { value: 'none' }, 'No authentication'), h('option', { value: 'oauth' }, 'OAuth sign-in'));
  const authRow = h('div', {}, h('label', { class: 'field-label' }, 'Authentication'), auth);
  kind.addEventListener('change', () => {
    target.placeholder = kind.value === 'http' ? 'https://example.com/mcp' : 'npx -y @modelcontextprotocol/server-everything';
    authRow.hidden = kind.value !== 'http';
  });
  const add = h('button', { class: 'btn btn-primary' }, 'Add server');
  add.addEventListener('click', async () => {
    const display = name.value.trim();
    const value = target.value.trim();
    if (!display || !value) return void toast('Give the server a name and an address.', 'error');
    const id = display.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'server';
    const [command = '', ...args] = value.split(/\s+/);
    const server =
      kind.value === 'http'
        ? { id, display_name: display, transport: 'http', url: value, auth: auth.value === 'oauth' ? { type: 'oauth', scopes: [] } : { type: 'none' }, request_headers: {} }
        : { id, display_name: display, transport: 'stdio', command, args, env: {}, secret_env: [] };
    try {
      await tauriInvoke('save_mcp_server', { server });
      toast(`${display} added. Enable it in Connectors.`);
      name.value = '';
      target.value = '';
    } catch (e) {
      toast(String(e), 'error');
    }
  });
  return card(
    h('h3', {}, 'MCP servers'),
    h('p', { class: 'muted' }, 'Connect any Model Context Protocol server. Its tools appear in Connectors.'),
    h('label', { class: 'field-label' }, 'Name'),
    name,
    h('label', { class: 'field-label' }, 'Type'),
    kind,
    h('label', { class: 'field-label' }, 'Address or command'),
    target,
    authRow,
    h('div', { class: 'card-actions' }, add)
  );
}

async function renderGeneral(content: HTMLElement) {
  const settings = await api.appSettings();
  const persist = () => void api.saveAppSettings(settings).then(() => toast('Saved.')).catch((e) => toast(String(e), 'error'));
  const shortcut = h('input', { class: 'input', value: settings.search_shortcut, placeholder: 'Ctrl+L' });
  shortcut.addEventListener('change', () => {
    settings.search_shortcut = shortcut.value.trim();
    persist();
  });
  content.append(
    card(
      row('Quick ask shortcut', 'Opens a search bar from anywhere.', shortcut),
      row('Enable quick ask', '', switchInput(settings.search_shortcut_enabled, (v) => { settings.search_shortcut_enabled = v; persist(); })),
      row('Settings shortcut', 'Ctrl+, opens settings.', switchInput(settings.settings_hotkey_enabled, (v) => { settings.settings_hotkey_enabled = v; persist(); })),
      row('Launch at startup', 'Open the app when you sign in to your computer.', switchInput(settings.launch_at_startup, (v) => { settings.launch_at_startup = v; persist(); }))
    )
  );
}

function renderAppearance(content: HTMLElement) {
  const current = storedTheme();
  const option = (theme: Theme, label: string, ic: string) =>
    h(
      'button',
      {
        class: `theme-option${current === theme ? ' active' : ''}`,
        on: {
          click: () => {
            setTheme(theme);
            content.replaceChildren();
            renderAppearance(content);
          },
        },
      },
      icon(ic, 22),
      label
    );
  content.append(card(h('h3', {}, 'Theme'), h('div', { class: 'theme-options' }, option('light', 'Light', 'sun'), option('dark', 'Dark', 'moon'), option('system', 'System', 'monitor'))));
}
