/** The settings dialog: profile, providers, permissions, general and appearance. */
import { tauriInvoke } from '../../shared/tauri';
import { api, type AppSettings, type Provider } from './api';
import { clear, h, icon, modal, toast } from './dom';
import { changed, state } from './state';
import { button, card, modalHeader, switchInput } from './ui';
import { audioApi, refreshAudioSettings, speakText, type AudioView } from './audio';
import { ACCENTS, BACKGROUNDS, accentHex, applyAppearance, setAccent, setBackground, storedAccent, storedBackground } from './appearance';

export type Section = 'profile' | 'providers' | 'audio' | 'permissions' | 'general' | 'appearance';

const SECTIONS: { id: Section; label: string; icon: string; group: string }[] = [
  { id: 'profile', label: 'Profile', icon: 'user', group: 'General settings' },
  { id: 'providers', label: 'Providers & models', icon: 'key', group: 'General settings' },
  { id: 'audio', label: 'Audio', icon: 'mic', group: 'General settings' },
  { id: 'permissions', label: 'Permissions', icon: 'shield', group: 'App settings' },
  { id: 'general', label: 'General', icon: 'monitor', group: 'App settings' },
  { id: 'appearance', label: 'Appearance', icon: 'palette', group: 'App settings' },
];

export type Theme = 'light' | 'dark' | 'system';

export function applyTheme(theme: Theme) {
  const dark = theme === 'dark' || (theme === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches);
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
  // The accent and background are tuned per theme, so they follow it.
  applyAppearance();
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
      clear(body, modalHeader(title, close));
      const content = h('div', { class: 'settings-content' });
      body.append(content);
      try {
        if (current === 'profile') renderProfile(content);
        if (current === 'providers') await renderProviders(content);
        if (current === 'audio') await renderAudio(content);
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


function row(label: string, description: string, control: HTMLElement): HTMLElement {
  return h('div', { class: 'setting-row' }, h('div', {}, h('strong', {}, label), description ? h('small', { class: 'muted' }, description) : null), control);
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
    ),
    card(
      h('h3', {}, 'Your data'),
      h('p', { class: 'muted' }, 'Export every conversation as Markdown, plus a copy of your projects, agents, tasks, knowledge, skills and libraries.'),
      h(
        'div',
        { class: 'card-actions start' },
        h(
          'button',
          {
            class: 'btn btn-secondary',
            on: {
              click: async () => {
                try {
                  const done = await tauriInvoke<string | null>('chat_export_all');
                  if (done) toast(done);
                } catch (e) {
                  toast(String(e), 'error');
                }
              },
            },
          },
          icon('download', 15),
          'Export all data'
        ),
        button({
          variant: 'secondary',
          icon: 'upload',
          label: 'Import from ChatGPT or Claude',
          title: 'Choose the data export zip, or its conversations.json',
          onClick: async () => {
            try {
              const report = await tauriInvoke<{ imported: number; skipped: number; source: string } | null>('chat_import_conversations');
              if (!report) return;
              toast(`Imported ${report.imported} ${report.source} conversation${report.imported === 1 ? '' : 's'}${report.skipped ? ` (${report.skipped} empty skipped)` : ''}.`);
              window.dispatchEvent(new Event('chat:conversations-changed'));
            } catch (e) {
              toast(String(e), 'error');
            }
          },
        })
      )
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
  content.append(await mediaCard());
}

interface MediaModels {
  imageModel: string | null;
  speechModel: string | null;
  voiceModel: string | null;
  voiceVendors: string[];
  imageVendors: string[];
  speechVendors: string[];
}

/** Which models make images and transcribe voice input. */
async function mediaCard(): Promise<HTMLElement> {
  const media = await tauriInvoke<MediaModels>('chat_media_models');
  const image = h('input', { class: 'input', value: media.imageModel ?? '', placeholder: 'xai/grok-imagine-image, openai/gpt-image-1' });
  const speech = h('input', { class: 'input', value: media.speechModel ?? '', placeholder: 'openai/gpt-4o-mini-transcribe, groq/whisper-large-v3-turbo' });
  const voice = h('input', { class: 'input', value: media.voiceModel ?? '', placeholder: 'xai/grok-tts, openai/gpt-4o-mini-tts' });
  const save = h('button', { class: 'btn btn-primary' }, 'Save');
  save.addEventListener('click', async () => {
    try {
      const saved = await tauriInvoke<MediaModels>('chat_set_media_models', { imageModel: image.value.trim() || null, speechModel: speech.value.trim() || null, voiceModel: voice.value.trim() || null });
      image.value = saved.imageModel ?? '';
      speech.value = saved.speechModel ?? '';
      voice.value = saved.voiceModel ?? '';
      toast(saved.imageModel || saved.speechModel ? 'Saved.' : 'Saved. No provider set up can serve these yet.');
    } catch (e) {
      toast(String(e), 'error');
    }
  });
  const note = (vendors: string[], what: string) =>
    h('small', { class: 'muted' }, vendors.length ? `Available from: ${vendors.join(', ')}. Leave empty for the default.` : `Set up a provider that offers ${what} to use this.`);
  return card(
    h('h3', {}, 'Images and voice'),
    h('p', { class: 'muted' }, 'The assistant creates images with the image model; the microphone in the composer transcribes with the speech model; the speaker under an answer reads it with the read-aloud model.'),
    h('label', { class: 'field-label' }, 'Image model'),
    image,
    note(media.imageVendors, 'image generation (OpenAI, xAI or Together)'),
    h('label', { class: 'field-label' }, 'Speech-to-text model'),
    speech,
    note(media.speechVendors, 'speech-to-text (OpenAI or Groq)'),
    h('label', { class: 'field-label' }, 'Read-aloud model'),
    voice,
    note(media.voiceVendors, 'text-to-speech (OpenAI or xAI)'),
    h('div', { class: 'card-actions' }, save)
  );
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

  const test = h('button', { class: 'btn btn-secondary' }, 'Test');
  test.addEventListener('click', async () => {
    // The model in use, else one the user added, else the first the catalog lists.
    const ofProvider = state.models.filter((m) => m.vendor === provider.vendor);
    const model =
      ofProvider.find((m) => m.reference === state.model) ??
      ofProvider.find((m) => provider.settings.customModels.includes(m.id)) ??
      ofProvider[0];
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
        provider.settings.hasKey ? h('button', { class: 'btn btn-danger', on: { click: () => void save(true) } }, 'Remove key') : null,
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
      row('Allow editing files', 'Lets the assistant create and change files in these folders. Each change still follows the composer’s approval mode.', switchInput(settings.filesystem_write_enabled, (v) => { settings.filesystem_write_enabled = v; void persist(settings); }))
    ),
    privacyCard(),
    card(
      h('h3', {}, 'Terminal'),
      row('Allow shell commands', 'Lets the assistant run commands on this computer. Every command asks first unless you allow it.', switchInput(settings.bash_enabled, (v) => { settings.bash_enabled = v; void persist(settings); }))
    ),
    mcpCard()
  );
}

function privacyCard(): HTMLElement {
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

  const rerender = () => {
    content.replaceChildren();
    renderAppearance(content);
  };
  const dark = document.documentElement.dataset.theme !== 'light';

  // Accent: a row of swatches, then a custom colour.
  const accent = storedAccent();
  const custom = accent.startsWith('#');
  const themeSwatch = h(
    'button',
    {
      type: 'button',
      class: `swatch theme${accent === 'theme' ? ' active' : ''}`,
      title: 'Match the background palette',
      'aria-label': 'Accent from the palette',
      'aria-pressed': String(accent === 'theme'),
      style: { '--swatch': accentHex(dark, 'theme') },
      on: { click: () => { setAccent('theme'); rerender(); } },
    },
    accent === 'theme' ? icon('check', 14) : icon('sparkle', 14)
  );
  const swatches = ACCENTS.map((a) =>
    h(
      'button',
      {
        type: 'button',
        class: `swatch${accent === a.id ? ' active' : ''}`,
        title: a.label,
        'aria-label': `${a.label} accent`,
        'aria-pressed': String(accent === a.id),
        style: { '--swatch': dark ? a.dark : a.light },
        on: { click: () => { setAccent(a.id); rerender(); } },
      },
      accent === a.id ? icon('check', 14) : null
    )
  );
  const picker = h('input', { type: 'color', class: 'color-input', value: custom ? accent : accentHex(false), title: 'Custom colour', 'aria-label': 'Custom accent colour' });
  picker.addEventListener('input', () => setAccent(picker.value));
  picker.addEventListener('change', rerender);
  const customSwatch = h('label', { class: `swatch custom${custom ? ' active' : ''}`, title: 'Custom colour', style: custom ? { '--swatch': accentHex(dark) } : {} }, custom ? icon('check', 14) : icon('palette', 15), picker);
  const preview = h(
    'div',
    { class: 'accent-preview' },
    button({ variant: 'primary', size: 'sm', label: 'Primary' }),
    h('span', { class: 'chip chip-accent' }, icon('workflow', 14), h('span', { class: 'chip-label' }, 'Workflow')),
    h('span', { class: 'status-badge tone-accent' }, 'Preview'),
    h('a', { href: '#', on: { click: (e: Event) => e.preventDefault() } }, 'A link'),
    switchInput(true, () => {}, 'Example switch')
  );
  content.append(
    card(
      h('h3', {}, 'Accent colour'),
      h('p', { class: 'muted' }, 'The first swatch follows the background palette. Used for primary buttons, links, selections, focus rings and highlights.'),
      h('div', { class: 'swatches' }, themeSwatch, swatches, customSwatch),
      preview
    )
  );

  // Background: tiles that preview the palette in the current theme.
  const background = storedBackground();
  const tiles = BACKGROUNDS.map((b) => {
    const surface = dark ? b.dark : b.light;
    return h(
      'button',
      {
        type: 'button',
        class: `bg-tile${background === b.id ? ' active' : ''}`,
        'aria-pressed': String(background === b.id),
        on: { click: () => { setBackground(b.id); rerender(); } },
      },
      h(
        'span',
        { class: 'bg-thumb', style: { background: surface.bg } },
        h('span', { class: 'bg-thumb-side', style: { background: surface.sidebar } }),
        h('span', { class: 'bg-thumb-card', style: { background: surface.elevated } }, h('span', { class: 'bg-thumb-dot', style: { background: accent === 'theme' ? (dark ? b.accent.dark : b.accent.light) : accentHex(dark) } })),
        h('span', { class: 'bg-thumb-bubble', style: { background: surface.bubble } })
      ),
      h('span', { class: 'bg-label' }, b.label, background === b.id ? icon('check', 14) : null)
    );
  });
  content.append(
    card(
      h('h3', {}, 'Background and palette'),
      h('p', { class: 'muted' }, `The palette for the window, sidebar and cards. Each one has a ${dark ? 'light' : 'dark'} version that follows the theme.`),
      h('div', { class: 'bg-tiles' }, tiles),
      h('div', { class: 'card-actions start' }, button({ variant: 'subtle', size: 'sm', icon: 'reset', label: 'Reset appearance', onClick: () => { setAccent('theme'); setBackground('default'); rerender(); } }))
    )
  );
}


const STT_ENGINES: [string, string][] = [
  ['provider', 'Your chat providers'],
  ['openai', 'OpenAI-compatible server'],
  ['elevenlabs', 'ElevenLabs'],
  ['off', 'Off'],
];
const TTS_ENGINES: [string, string][] = [
  ['provider', 'Your chat providers'],
  ['openai', 'OpenAI-compatible server'],
  ['xai', 'xAI'],
  ['elevenlabs', 'ElevenLabs'],
  ['browser', 'System voice'],
  ['off', 'Off'],
];

/** Settings → Audio: the speech engines, as Open WebUI sets them up. */
async function renderAudio(content: HTMLElement) {
  let view: AudioView = await audioApi.get();
  const s = structuredClone(view.settings);
  const select = (options: [string, string][], value: string, onChange: (v: string) => void) => {
    const el = h('select', { class: 'input' }, options.map(([v, l]) => h('option', { value: v, selected: v === value }, l)));
    el.addEventListener('change', () => onChange(el.value));
    return el;
  };
  const text = (value: string, placeholder: string, onInput: (v: string) => void, type = 'text') => {
    const el = h('input', { class: 'input', type, value, placeholder, autocomplete: 'off' });
    el.addEventListener('input', () => onInput(el.value));
    return el;
  };
  const keys = { sttKey: '', ttsKey: '', clearSttKey: false, clearTtsKey: false };
  const keyField = (has: boolean, onInput: (v: string) => void, onClear: () => void) => {
    const input = text('', has ? '•••••••• saved in keychain' : 'API key', onInput, 'password');
    const clearBtn = has ? button({ variant: 'subtle', size: 'sm', label: 'Remove key', onClick: () => { onClear(); input.placeholder = 'Removed on save'; } }) : null;
    return h('div', { class: 'key-row' }, input, clearBtn);
  };
  const field = (label: string, control: HTMLElement, hint?: string) =>
    h('div', { class: 'field' }, h('label', { class: 'field-label' }, label), control, hint ? h('small', { class: 'field-hint' }, hint) : null);

  const sttBody = h('div', { class: 'audio-fields' });
  const ttsBody = h('div', { class: 'audio-fields' });
  const renderStt = () =>
    clear(
      sttBody,
      field('Engine', select(STT_ENGINES, s.stt.engine, (v) => { s.stt.engine = v as typeof s.stt.engine; renderStt(); })),
      s.stt.engine === 'provider'
        ? h('p', { class: 'muted small' }, view.providerSpeechModel ? `Uses ${view.providerSpeechModel}. Change it under Providers & models.` : 'None of your providers offers speech-to-text (OpenAI or Groq). Choose another engine.')
        : null,
      s.stt.engine === 'openai' ? field('API base URL', text(s.stt.baseUrl, 'https://api.openai.com/v1  ·  https://api.groq.com/openai/v1  ·  http://localhost:8000/v1', (v) => (s.stt.baseUrl = v)), 'Any server with /audio/transcriptions: OpenAI, Groq, Speaches, LocalAI, faster-whisper-server.') : null,
      s.stt.engine === 'openai' || s.stt.engine === 'elevenlabs' ? field('API key', keyField(s.stt.hasKey, (v) => (keys.sttKey = v), () => (keys.clearSttKey = true))) : null,
      s.stt.engine === 'openai' || s.stt.engine === 'elevenlabs' ? field('Model', text(s.stt.model, s.stt.engine === 'elevenlabs' ? 'scribe_v1' : 'whisper-1  ·  gpt-4o-mini-transcribe  ·  whisper-large-v3-turbo', (v) => (s.stt.model = v))) : null,
      s.stt.engine !== 'off' && s.stt.engine !== 'provider' ? field('Language', text(s.stt.language, 'Detect automatically, or a code like en, pt', (v) => (s.stt.language = v))) : null,
      s.stt.engine !== 'off' ? h('div', { class: 'setting-row' }, h('div', {}, h('strong', {}, 'Send after dictation'), h('small', { class: 'muted' }, 'Send the message as soon as the transcript arrives.')), switchInput(s.stt.autoSend, (v) => (s.stt.autoSend = v))) : null
    );
  const voiceList = h('datalist', { id: 'tts-voices' });
  const loadVoices = async () => {
    const voices = s.tts.engine === 'browser' && 'speechSynthesis' in window
      ? speechSynthesis.getVoices().map((v) => [v.name, `${v.name} (${v.lang})`] as [string, string])
      : await audioApi.voices(s.tts.engine, s.tts.baseUrl).catch(() => [] as [string, string][]);
    clear(voiceList, voices.map(([id, name]) => h('option', { value: id }, name)));
  };
  const renderTts = () => {
    const needsServer = ['openai', 'xai', 'elevenlabs'].includes(s.tts.engine);
    const voiceInput = h('input', { class: 'input', value: s.tts.voice, placeholder: s.tts.engine === 'elevenlabs' ? 'Voice id (pick from the list once the key is saved)' : s.tts.engine === 'xai' ? 'eve' : 'alloy', list: 'tts-voices', autocomplete: 'off' });
    voiceInput.addEventListener('input', () => (s.tts.voice = voiceInput.value));
    const speed = h('input', { type: 'range', min: '0.5', max: '2', step: '0.05', value: String(s.tts.speed), class: 'speed-range' });
    const speedLabel = h('span', { class: 'muted' }, `${s.tts.speed.toFixed(2)}×`);
    speed.addEventListener('input', () => { s.tts.speed = Number(speed.value); speedLabel.textContent = `${s.tts.speed.toFixed(2)}×`; });
    clear(
      ttsBody,
      field('Engine', select(TTS_ENGINES, s.tts.engine, (v) => { s.tts.engine = v as typeof s.tts.engine; renderTts(); void loadVoices(); })),
      s.tts.engine === 'provider'
        ? h('p', { class: 'muted small' }, view.providerVoiceModel ? `Uses ${view.providerVoiceModel}. Change it under Providers & models.` : 'None of your providers offers text-to-speech (OpenAI or xAI). Choose another engine.')
        : null,
      s.tts.engine === 'browser' ? h('p', { class: 'muted small' }, 'Uses the voices installed on this computer. Nothing leaves the device.') : null,
      needsServer ? field('API base URL', text(s.tts.baseUrl, s.tts.engine === 'elevenlabs' ? 'https://api.elevenlabs.io' : s.tts.engine === 'xai' ? 'https://api.x.ai' : 'https://api.openai.com/v1  ·  http://localhost:8880/v1', (v) => (s.tts.baseUrl = v)), s.tts.engine === 'openai' ? 'Any server with /audio/speech: OpenAI, Kokoro-FastAPI, LocalAI, Speaches.' : 'Leave empty for the service default.') : null,
      needsServer ? field('API key', keyField(s.tts.hasKey, (v) => (keys.ttsKey = v), () => (keys.clearTtsKey = true))) : null,
      needsServer ? field('Model', text(s.tts.model, s.tts.engine === 'elevenlabs' ? 'eleven_multilingual_v2' : s.tts.engine === 'xai' ? 'grok-tts' : 'tts-1  ·  gpt-4o-mini-tts  ·  kokoro', (v) => (s.tts.model = v))) : null,
      s.tts.engine !== 'off' && s.tts.engine !== 'provider' ? field('Voice', h('div', {}, voiceInput, voiceList)) : null,
      s.tts.engine !== 'off' && s.tts.engine !== 'xai' ? field('Speed', h('div', { class: 'speed-row' }, speed, speedLabel)) : null,
      s.tts.engine !== 'off' ? h('div', { class: 'setting-row' }, h('div', {}, h('strong', {}, 'Read answers aloud automatically'), h('small', { class: 'muted' }, 'Each answer is read when it finishes.')), switchInput(s.tts.autoPlay, (v) => (s.tts.autoPlay = v))) : null
    );
  };
  renderStt();
  renderTts();
  void loadVoices();

  const persist = async () => {
    view = await audioApi.save(s, keys);
    Object.assign(s, structuredClone(view.settings));
    keys.sttKey = keys.ttsKey = '';
    keys.clearSttKey = keys.clearTtsKey = false;
    await refreshAudioSettings();
    renderStt();
    renderTts();
    void loadVoices();
  };
  const save = button({ variant: 'primary', label: 'Save', onClick: async () => { try { await persist(); toast('Audio settings saved.'); } catch (e) { toast(String(e), 'error'); } } });
  const test = button({
    variant: 'secondary',
    icon: 'speaker',
    label: 'Save and test voice',
    onClick: async () => {
      try {
        await persist();
        await speakText('Hello! This is how answers will sound.');
      } catch (e) {
        toast(String(e), 'error');
      }
    },
  });
  content.append(
    h('p', { class: 'muted intro' }, 'Choose how voice input is transcribed and how answers are read aloud, with any compatible service or one running on your own machine. Keys stay in the OS keychain.'),
    card(h('h3', {}, 'Speech to text'), h('p', { class: 'muted' }, 'Used by the microphone in the composer.'), sttBody),
    card(h('h3', {}, 'Text to speech'), h('p', { class: 'muted' }, 'Used by the speaker under each answer.'), ttsBody),
    h('div', { class: 'card-actions' }, test, save)
  );
}
