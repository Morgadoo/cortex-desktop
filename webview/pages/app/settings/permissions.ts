/** Settings → Permissions: what the assistant may reach (files, the computer, secrets, calendars, MCP and OpenAPI servers) and the tools it may run. */
import { tauriInvoke } from '../../../shared/tauri';
import { api, type AppSettings } from '../core/api';
import { clear, h } from '../ui/h';
import { icon } from '../ui/icons';
import { confirmDialog, modal, toast } from '../ui/overlay';
import { state } from '../core/state';
import { button, card, modalHeader, switchInput } from '../ui/ui';
import { customToolsCard, functionsCard, hooksCard } from './tools';
import { row } from './row';

/** Computer use: the assistant may see the screen and use the mouse and keyboard. */
/**
 * The agent workbench (workbench.rs): code from the assistant and agents runs in a container with
 * Python, Node.js, media tools and Blender, seeing only its workspace and the agent's own home.
 */
export async function workbenchCard(): Promise<HTMLElement> {
  type Status = { state: 'unavailable'; reason: string } | { state: 'missing' } | { state: 'building' } | { state: 'ready'; gpu: boolean } | { state: 'failed'; error: string };
  type Info = { enabled: boolean; image: string; status: Status };
  const info = await tauriInvoke<Info>('chat_workbench_status');
  const status = h('div', { class: 'workbench-status' });
  const actions = h('div', { class: 'card-actions start' });
  const render = () => {
    const s = info.status;
    const line =
      s.state === 'unavailable'
        ? `Not available here: ${s.reason} Code runs in the basic sandbox.`
        : s.state === 'missing'
          ? 'Not built yet. Building downloads Fedora and the tools (about 2 GB) and takes several minutes; until then code runs in the basic sandbox.'
          : s.state === 'building'
            ? 'Building… this takes several minutes. You can keep working.'
            : s.state === 'ready'
              ? s.gpu
                ? 'Ready. Runs can use the NVIDIA GPU.'
                : 'Ready. Runs use the CPU: the GPU is not reachable from containers (the host needs a current NVIDIA CDI spec).'
              : 'The build failed.';
    clear(status, h('p', { class: s.state === 'failed' ? 'error-text' : 'muted' }, line), s.state === 'failed' ? h('pre', { class: 'mono workbench-log' }, s.error) : null);
    const canBuild = s.state === 'missing' || s.state === 'failed';
    clear(
      actions,
      canBuild || s.state === 'building'
        ? button({
            variant: 'secondary',
            icon: 'download',
            label: s.state === 'building' ? 'Building…' : 'Build the workbench',
            disabled: s.state === 'building',
            onClick: async () => {
              info.status = { state: 'building' };
              render();
              try {
                info.status = await tauriInvoke<Status>('chat_workbench_build');
                toast(info.status.state === 'ready' ? 'The agent workbench is ready.' : 'The workbench could not be built.', info.status.state === 'ready' ? 'info' : 'error');
              } catch (e) {
                info.status = { state: 'failed', error: String(e) };
              }
              render();
            },
          })
        : null
    );
  };
  render();
  return card(
    h('h3', {}, 'Agent workbench'),
    h(
      'p',
      { class: 'muted' },
      'Code from the assistant and your agents runs in a container of its own: Python with its data libraries, Node.js, ffmpeg, ImageMagick and Blender for 3D. It sees only the chat’s workspace and the agent’s own home, never your files; the network is on only for agents you allow. Needs Podman.'
    ),
    row(
      'Run code in the workbench',
      'When it is built. Off, code runs in the basic sandbox with this computer’s programs.',
      switchInput(info.enabled, (v) => {
        info.enabled = v;
        void tauriInvoke('chat_workbench_set_enabled', { enabled: v })
          .then(() => toast(v ? 'Code runs in the workbench.' : 'Code runs in the basic sandbox.'))
          .catch((e) => toast(String(e), 'error'));
      })
    ),
    status,
    actions
  );
}

export async function computerCard(): Promise<HTMLElement> {
  type Settings = { enabled: boolean; blockedApps: string[]; onlyApps: string[] };
  let current = await tauriInvoke<Settings>('chat_computer_settings');
  const save = (next: Partial<Settings>, said: string) => {
    current = { ...current, ...next };
    void tauriInvoke('chat_save_computer_settings', { settings: current })
      .then(() => toast(said))
      .catch((e) => toast(String(e), 'error'));
  };
  const lines = (s: string) =>
    s
      .split('\n')
      .map((l) => l.trim())
      .filter(Boolean);
  const blocked = h('textarea', { class: 'input textarea short', rows: 3, placeholder: 'One per line, e.g.\nBank\n1Password\nKeePassXC' });
  blocked.value = (current.blockedApps ?? []).join('\n');
  blocked.addEventListener('change', () => save({ blockedApps: lines(blocked.value) }, 'Saved.'));
  const only = h('textarea', { class: 'input textarea short', rows: 3, placeholder: 'Empty: any app not blocked. One per line, e.g.\nGIMP\nLibreOffice Calc' });
  only.value = (current.onlyApps ?? []).join('\n');
  only.addEventListener('change', () => save({ onlyApps: lines(only.value) }, 'Saved.'));
  return card(
    h('h3', {}, 'Computer use'),
    row(
      'Allow computer use',
      'Lets the assistant take screenshots and click, type and scroll on this computer. Screenshots are sent to your model. Each action follows the composer’s mode: Ask confirms every click, Auto checks each one against your request. Needs xdotool and ImageMagick on Linux, or cliclick on macOS.',
      switchInput(current.enabled, (v) => save({ enabled: v }, v ? 'Computer use on.' : 'Computer use off.'))
    ),
    h('label', { class: 'field-label' }, 'Never act in these apps'),
    h('p', { class: 'muted small' }, 'Words matched in the title of the window in front. A click or keystroke there is refused, and the assistant asks you to do that step.'),
    blocked,
    h('label', { class: 'field-label' }, 'Only act in these apps'),
    only
  );
}

interface OpenApiServer {
  id: string;
  name: string;
  specUrl: string;
  baseUrl: string;
  enabled: boolean;
  hasKey: boolean;
  operations: number;
}

/** OpenAPI tool servers: each operation of a service's spec becomes a tool (Open WebUI). */
export async function openApiCard(): Promise<HTMLElement> {
  const list = h('div', { class: 'openapi-list' });
  const render = (servers: OpenApiServer[]) =>
    clear(
      list,
      servers.length
        ? servers.map((s) =>
            h(
              'div',
              { class: 'setting-row' },
              h('div', {}, h('strong', {}, s.name), h('small', { class: 'muted' }, `${s.operations} tools · ${s.baseUrl}${s.hasKey ? ' · key saved' : ''}`)),
              h(
                'div',
                { class: 'row-actions' },
                h(
                  'button',
                  {
                    class: 'link-btn',
                    title: 'Fetch the spec again',
                    on: {
                      click: () =>
                        void tauriInvoke<OpenApiServer[]>('chat_refresh_openapi_server', { id: s.id })
                          .then((v) => {
                            render(v);
                            toast('Refreshed.');
                          })
                          .catch((e) => toast(String(e), 'error')),
                    },
                  },
                  'Refresh'
                ),
                h(
                  'button',
                  {
                    class: 'link-btn',
                    on: {
                      click: () =>
                        void tauriInvoke<OpenApiServer[]>('chat_remove_openapi_server', { id: s.id })
                          .then(render)
                          .catch((e) => toast(String(e), 'error')),
                    },
                  },
                  'Remove'
                ),
                switchInput(s.enabled, (v) => void tauriInvoke<OpenApiServer[]>('chat_toggle_openapi_server', { id: s.id, enabled: v }).then(render))
              )
            )
          )
        : h('p', { class: 'muted' }, 'No API servers yet.')
    );
  render(await tauriInvoke<OpenApiServer[]>('chat_openapi_servers'));
  const name = h('input', { class: 'input', placeholder: 'Name (optional; the spec’s title otherwise)' });
  const url = h('input', { class: 'input', placeholder: 'https://api.example.com/openapi.json' });
  const key = h('input', { class: 'input', type: 'password', placeholder: 'Bearer token, if the API needs one' });
  const add = h('button', { class: 'btn btn-primary' }, 'Add API');
  add.addEventListener('click', async () => {
    if (!url.value.trim()) return void toast('Give the address of the OpenAPI spec (JSON).', 'error');
    add.setAttribute('disabled', '');
    try {
      render(await tauriInvoke<OpenApiServer[]>('chat_add_openapi_server', { name: name.value, specUrl: url.value, key: key.value.trim() || null }));
      name.value = url.value = key.value = '';
      toast('Added. Its operations are now tools; reads run freely, changes follow your mode.');
    } catch (e) {
      toast(String(e), 'error');
    } finally {
      add.removeAttribute('disabled');
    }
  });
  return card(
    h('h3', {}, 'API tool servers (OpenAPI)'),
    h('p', { class: 'muted' }, 'Point to any service’s OpenAPI spec: each operation becomes a tool. GET operations run as reads; anything that changes data follows the approval mode.'),
    list,
    h('label', { class: 'field-label' }, 'Spec address'),
    url,
    h('label', { class: 'field-label' }, 'Name'),
    name,
    h('label', { class: 'field-label' }, 'API key'),
    key,
    h('div', { class: 'card-actions' }, add)
  );
}

export function privacyCard(): HTMLElement {
  const toggle = switchInput(false, (v) => void tauriInvoke('chat_set_privacy', { settings: { enabled: v } }).then(() => toast(v ? 'Privacy filter on.' : 'Privacy filter off.')));
  void tauriInvoke<{ enabled: boolean }>('chat_get_privacy').then((s) => (toggle.checked = s.enabled));
  return card(
    h('h3', {}, 'Privacy filter'),
    row(
      'Hide personal data from cloud models',
      'Emails, phone numbers, IBANs and card numbers you type are replaced with placeholders before a request leaves this device, and put back in the reply. Local models (Ollama, llama.cpp) are never filtered.',
      toggle
    )
  );
}

export function mcpCard(): HTMLElement {
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
    const id =
      display
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, '-')
        .replace(/^-|-$/g, '') || 'server';
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

export async function renderPermissions(content: HTMLElement) {
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
              h(
                'button',
                {
                  class: 'icon-btn small',
                  title: 'Remove',
                  on: {
                    click: async () => {
                      settings.filesystem_allowed_roots = settings.filesystem_allowed_roots.filter((r) => r !== root);
                      renderRoots();
                      await persist(settings);
                    },
                  },
                },
                icon('close', 14)
              )
            )
          )
        : h('p', { class: 'muted' }, 'No folders yet. The assistant cannot see your files.')
    );
  renderRoots();
  const add = h('button', { class: 'btn btn-secondary' }, icon('plus', 16), 'Add folder');
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
      row(
        'Allow editing files',
        'Lets the assistant create and change files in these folders. Each change still follows the composer’s approval mode.',
        switchInput(settings.filesystem_write_enabled, (v) => {
          settings.filesystem_write_enabled = v;
          void persist(settings);
        })
      )
    ),
    privacyCard(),
    card(
      h('h3', {}, 'Terminal'),
      row(
        'Allow shell commands',
        'Lets the assistant run commands on this computer. Every command asks first unless you allow it.',
        switchInput(settings.bash_enabled, (v) => {
          settings.bash_enabled = v;
          void persist(settings);
        })
      )
    ),
    await workbenchCard(),
    await computerCard(),
    await customToolsCard(),
    await functionsCard(),
    mcpCard(),
    await calendarCard(),
    await hooksCard(),
    await vaultCard(),
    await openApiCard()
  );
}

interface VaultSecret {
  name: string;
  description: string;
  everywhere: boolean;
}

/**
 * The vault (OpenBot): API tokens and passwords in the OS keychain, given to the agents that may
 * use them as environment variables. The model only ever sees the names.
 */
async function vaultCard(): Promise<HTMLElement> {
  let secrets = await tauriInvoke<VaultSecret[]>('chat_vault_list').catch(() => [] as VaultSecret[]);
  const list = h('div', { class: 'hook-list' });
  const users = (name: string) => state.agents.filter((a) => a.secrets?.includes(name)).map((a) => a.name);
  const render = () =>
    clear(
      list,
      secrets.length
        ? secrets.map((s) =>
            h(
              'div',
              { class: 'hook-row' },
              icon('key', 16),
              h(
                'div',
                { class: 'grow' },
                h('strong', {}, s.name),
                h(
                  'small',
                  { class: 'muted', style: 'display:block' },
                  [s.description, s.everywhere ? 'Every chat and agent' : users(s.name).length ? `Agents: ${users(s.name).join(', ')}` : 'No agent yet: pick it in an agent’s settings'].filter(Boolean).join(' · ')
                )
              ),
              h('button', { class: 'link-btn', on: { click: () => edit(s) } }, 'Edit'),
              h(
                'button',
                {
                  class: 'icon-btn small',
                  title: 'Delete',
                  on: {
                    click: async () => {
                      if (!(await confirmDialog(`Delete ${s.name}?`, 'Its value is removed from the keychain and no agent can use it any more.'))) return;
                      try {
                        secrets = await tauriInvoke<VaultSecret[]>('chat_vault_delete', { name: s.name });
                        state.agents = await api.agents();
                        render();
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
        : h('p', { class: 'muted' }, 'Nothing stored yet.')
    );
  const edit = (existing?: VaultSecret) =>
    void modal((close) => {
      const name = h('input', { class: 'input', value: existing?.name ?? '', placeholder: 'GITHUB_TOKEN', disabled: !!existing, 'aria-label': 'Name' });
      const description = h('input', { class: 'input', value: existing?.description ?? '', placeholder: 'GitHub token for the work repositories', 'aria-label': 'What it is for' });
      const value = h('input', { class: 'input', type: 'password', autocomplete: 'off', placeholder: existing ? 'Leave empty to keep the stored value' : 'The token or password', 'aria-label': 'Value' });
      let everywhere = existing?.everywhere ?? false;
      const error = h('p', { class: 'error-text' });
      const save = async () => {
        try {
          secrets = await tauriInvoke<VaultSecret[]>('chat_vault_save', {
            secret: { name: name.value.trim().toUpperCase(), description: description.value.trim(), everywhere },
            value: value.value || null,
          });
          render();
          toast(existing ? 'Secret saved.' : `${name.value.trim().toUpperCase()} is in the vault. Pick it in an agent’s settings to let it use it.`);
          close();
        } catch (e) {
          error.textContent = String(e);
        }
      };
      return h(
        'div',
        { class: 'dialog-small' },
        modalHeader(existing ? `Edit ${existing.name}` : 'Add a secret', close, { flush: true }),
        h('label', { class: 'field-label' }, 'Name (the environment variable)'),
        name,
        h('label', { class: 'field-label' }, 'What it is for (the model sees this)'),
        description,
        h('label', { class: 'field-label' }, 'Value (stored in the keychain)'),
        value,
        row(
          'Every chat and agent',
          'Off: only the agents you pick can use it.',
          switchInput(everywhere, (v) => {
            everywhere = v;
          })
        ),
        error,
        h('div', { class: 'dialog-actions' }, button({ variant: 'secondary', label: 'Cancel', onClick: close }), button({ variant: 'primary', label: 'Save', onClick: () => void save() }))
      );
    });
  render();
  return card(
    h('h3', {}, 'Vault'),
    h(
      'p',
      { class: 'muted' },
      'API tokens and passwords for agents, kept in the OS keychain. An agent you allow gets them as environment variables in its shell commands, code and your own tools; the model only sees the names, and any value that shows up in a tool’s output is masked.'
    ),
    list,
    h('div', { class: 'card-actions start' }, button({ variant: 'secondary', icon: 'plus', label: 'Add a secret', onClick: () => edit() }))
  );
}

interface CalendarSource {
  name: string;
  location: string;
}

/**
 * Calendars and contacts (Osaurus): private ICS links or .ics files, and .vcf contact files,
 * that the assistant reads with calendar_events and find_contact.
 */
async function calendarCard(): Promise<HTMLElement> {
  let settings = await tauriInvoke<{ calendars: CalendarSource[]; contacts: CalendarSource[] }>('chat_calendar_sources').catch(() => ({ calendars: [], contacts: [] }));
  const list = h('div', { class: 'hook-list' });
  const save = async (next: typeof settings) => {
    try {
      const report = await tauriInvoke<string[]>('chat_set_calendar_sources', { settings: next });
      settings = next;
      render();
      if (report.length) toast(`Read ${report.join(', ')}.`);
    } catch (e) {
      toast(String(e), 'error');
    }
  };
  const render = () => {
    const rows = [...settings.calendars.map((c, i) => ({ c, kind: 'calendar' as const, i })), ...settings.contacts.map((c, i) => ({ c, kind: 'contacts' as const, i }))];
    clear(
      list,
      rows.length
        ? rows.map(({ c, kind, i }) =>
            h(
              'div',
              { class: 'hook-row' },
              icon(kind === 'calendar' ? 'calendar' : 'user', 16),
              h('div', { class: 'hook-text' }, h('strong', {}, c.name), h('code', {}, c.location.replace(/(https?:\/\/[^/]+\/).{12,}/, '$1…'))),
              h(
                'button',
                {
                  class: 'icon-btn small',
                  title: 'Remove',
                  on: { click: () => void save(kind === 'calendar' ? { ...settings, calendars: settings.calendars.filter((_, j) => j !== i) } : { ...settings, contacts: settings.contacts.filter((_, j) => j !== i) }) },
                },
                icon('trash', 15)
              )
            )
          )
        : h('p', { class: 'muted small' }, 'None yet.')
    );
  };
  const kind = h('select', { class: 'input select-sm' }, h('option', { value: 'calendar' }, 'Calendar'), h('option', { value: 'contacts' }, 'Contacts'));
  const name = h('input', { class: 'input', placeholder: 'Name, e.g. Work' });
  const location = h('input', { class: 'input', placeholder: 'Private ICS link (https:// or webcal://) or a file' });
  const browse = button({
    variant: 'subtle',
    size: 'sm',
    label: 'File…',
    onClick: async () => {
      const path = await tauriInvoke<string | null>('chat_pick_calendar_file', { contacts: kind.value === 'contacts' }).catch(() => null);
      if (path) {
        location.value = path;
        if (!name.value)
          name.value = path
            .split(/[\\/]/)
            .pop()!
            .replace(/\.[^.]+$/, '');
      }
    },
  });
  const add = button({
    variant: 'secondary',
    icon: 'plus',
    label: 'Add',
    onClick: () => {
      const entry = { name: name.value.trim() || (kind.value === 'calendar' ? 'Calendar' : 'Contacts'), location: location.value.trim() };
      if (!entry.location) return toast('Paste a link or choose a file.', 'error');
      void save(kind.value === 'calendar' ? { ...settings, calendars: [...settings.calendars, entry] } : { ...settings, contacts: [...settings.contacts, entry] }).then(() => {
        name.value = '';
        location.value = '';
      });
    },
  });
  render();
  return card(
    h('h3', {}, 'Calendars and contacts'),
    h(
      'p',
      { class: 'muted' },
      'Let the assistant read your schedule and look people up. Add a calendar\u2019s private ICS link (Google, Proton, Outlook and iCloud all offer one) or an .ics file, and a .vcf contacts export. It only reads them; links are fetched when it asks.'
    ),
    list,
    h('div', { class: 'calendar-add' }, kind, name, location, browse, add)
  );
}
