/** The settings dialog: profile, providers, permissions, general and appearance. */
import { tauriInvoke } from '../../shared/tauri';
import { api, listen, type AppSettings, type Provider } from './api';
import { clear, confirmDialog, h, icon, modal, toast, clockHour12, setClock } from './dom';
import { historyButton } from './versions';
import { autocompleteOn, changed, finishNotificationsOn, setAutocomplete, finishSoundOn, followStreamOn, followUpsOn, setFinishNotifications, setFinishSound, setFollowStream, setFollowUps, setTemporaryByDefault, state, temporaryByDefault } from './state';
import { button, card, modalHeader, switchInput } from './ui';
import { audioApi, audioSettings, refreshAudioSettings, speakText, type AudioView } from './audio';
import { SHORTCUTS, bindingFor, comboOf, display, setBinding } from './shortcuts';
import { EMOJI_STYLES, emojiStatus, emojiStyle, onEmojiStatus, setEmojiStyle, type EmojiStatus, type EmojiStyle } from './fluent';
import { uploadsCard } from './uploads';
import { setMaterial, storedMaterial, type Material } from './appearance';
import { ACCENTS, BACKDROPS, BACKGROUNDS, backdropPreview, backdropVeil, setBackdrop, setBackdropImage, setBackdropVeil, storedBackdrop, TEXT_SIZES, ZOOMS, accentHex, applyAppearance, readability, setAccent, setBackground, setReadability, storedAccent, storedBackground, type TextSize } from './appearance';
import { setVoicePauseMs, voicePauseMs } from './dictation';
import { setWakePrefs, wakePrefs } from './wakeword';
import { setVimEnabled, vimEnabled } from './vim';
import { sharedLinksCard } from './share';
import { effectiveLanguage, setStoredLanguage, storedLanguage } from './i18n';
import { setOrbColor, storedOrbColor, type OrbColor } from './orbgl';

export type Section = 'profile' | 'providers' | 'audio' | 'permissions' | 'general' | 'appearance';

const SECTIONS: { id: Section; label: string; icon: string; group: string }[] = [
  { id: 'profile', label: 'Profile', icon: 'user', group: 'General settings' },
  { id: 'providers', label: 'Providers & models', icon: 'key', group: 'General settings' },
  { id: 'audio', label: 'Audio', icon: 'mic', group: 'General settings' },
  { id: 'permissions', label: 'Permissions', icon: 'shield', group: 'App settings' },
  { id: 'general', label: 'General', icon: 'monitor', group: 'App settings' },
  { id: 'appearance', label: 'Appearance', icon: 'palette', group: 'App settings' },
];

/** What the settings search finds: a setting's name, its section, and other words for it. */
const SETTINGS_INDEX: [string, Section, string][] = [
  ['Your profile', 'profile', 'name about instructions tone personality'],
  ['Your data', 'profile', 'export import backup delete history'],
  ['Uploaded files', 'profile', 'files attachments images uploads remove search full library'],
  ['Shared links', 'profile', 'share link network url stop sharing public page'],
  ['Encrypt data', 'profile', 'encryption at rest encrypt security privacy keychain disk'],
  ['Sync between devices', 'profile', 'sync devices encrypted syncthing dropbox icloud nextcloud onedrive passphrase other computer'],
  ['Import coding sessions', 'profile', 'claude code codex import sessions coding agents'],
  ['Storage and backups', 'profile', 'backup restore damaged files recently deleted trash'],
  ['Backup model', 'providers', 'fallback failover failure credit down'],
  ['Providers and API keys', 'providers', 'key openai anthropic mistral xai ollama llama.cpp lm studio model default base url'],
  ['Web search', 'providers', 'searxng brave tavily engine internet'],
  ['Semantic search', 'providers', 'embeddings libraries vector rag rerank reranker reranking vector database store qdrant minilm built-in'],
  ['ACP agents', 'providers', 'acp agent client protocol zed claude code codex gemini goose coding agents'],
  ['Activity log', 'profile', 'audit log export tool calls approvals history csv'],
  ['Built-in models', 'providers', 'local offline download qwen llama whisper on device runtime'],
  ['Document extraction', 'providers', 'tika docling extraction ocr pdf documents parser engine'],
  ['Images and voice', 'providers', 'image generation dall-e text to speech voice model local image server stable diffusion automatic1111 forge draw things'],
  ['Speech to text', 'audio', 'dictation microphone whisper transcription stt'],
  ['Text to speech', 'audio', 'read aloud voice tts elevenlabs'],
  ['Voice conversation', 'audio', 'pause silence turn hands-free call talk'],
  ['Vault', 'permissions', 'secrets tokens api keys passwords agents environment variables keychain'],
  ['Wake word', 'audio', 'hey cortex wake word hands-free always listening voice activation'],
  ['Dictation anywhere', 'audio', 'dictation hotkey shortcut system-wide global type speech any app whisper'],
  ['Files', 'permissions', 'edit files folders allow'],
  ['Terminal', 'permissions', 'shell commands run bash'],
  ['Your tools', 'permissions', 'custom tools python javascript script'],
  ['Functions', 'permissions', 'functions filters pipes inlet outlet redact block pii model from code'],
  ['Computer use', 'permissions', 'screen mouse keyboard screenshot click'],
  ['API tool servers (OpenAPI)', 'permissions', 'openapi swagger rest tools'],
  ['Privacy filter', 'permissions', 'redact personal data pii'],
  ['MCP servers', 'permissions', 'mcp connectors model context protocol'],
  ['Calendars and contacts', 'permissions', 'calendar ics ical contacts vcard vcf google proton outlook icloud schedule agenda'],
  ['Hooks', 'permissions', 'hooks before after tool call command script block turn end'],
  ['Quick ask shortcut', 'general', 'global hotkey quick ask bar'],
  ['Launch at startup', 'general', 'autostart login'],
  ['Notify when a reply is ready', 'general', 'notification background alert'],
  ['Suggest follow-ups', 'general', 'follow up questions suggestions'],
  ['Open links in Cortex\u2019s browser', 'general', 'browser links integrated external open in app web page'],
  ['Autocomplete as you type', 'general', 'autocomplete suggest complete typing tab continuation'],
  ['New chats are temporary', 'general', 'temporary incognito history'],
  ['Keyboard shortcuts', 'general', 'hotkeys keys rebind'],
  ['Slack', 'general', 'slack bot socket mode workspace channel message agent'],
  ['Discord', 'general', 'discord bot server gateway channel direct message agent'],
  ['Voice orb colour', 'audio', 'voice orb colour color ember accent glow shader'],
  ['Telegram', 'general', 'telegram phone mobile bot channel message agent away remote'],
  ['Local API', 'general', 'openai compatible server port token curl mcp anthropic website widget embed page chat bubble intranet snippet'],
  ['Defaults for every chat', 'general', 'temperature top-p top_p max tokens reply length model parameters'],
  ['Error reports and updates', 'general', 'privacy telemetry error reports crash sentry updates diagnostics'],
  ['Language', 'appearance', 'language translate português español français deutsch interface locale'],
  ['Vim keys', 'general', 'vim keybindings normal mode editor hjkl modal'],
  ['Clock', 'general', '12-hour 24-hour time format am pm'],
  ['Theme', 'appearance', 'dark light system mode'],
  ['Accent colour', 'appearance', 'color colour highlight'],
  ['Background and palette', 'appearance', 'background colours palette'],
  ['Readability', 'appearance', 'text size font zoom scale contrast motion animation accessibility'],
  ['Material', 'appearance', 'liquid glass apple translucent blur frosted material solid'],
  ['Emoji', 'appearance', 'emoji style microsoft fluent teams 3d flat system'],
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
    // Search: matching settings replace the section list; choosing one opens it there.
    const search = h('input', { class: 'input settings-search', type: 'search', placeholder: 'Search settings' });
    let query = '';
    let focusLabel: string | null = null;
    search.addEventListener('input', () => {
      query = search.value.trim().toLowerCase();
      renderNav();
    });
    search.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') (list.querySelector('.settings-hit') as HTMLButtonElement | null)?.click();
      if (e.key === 'Escape' && query) {
        e.stopPropagation();
        search.value = '';
        query = '';
        renderNav();
      }
    });
    // The profile line and the search box stay; only the list below them changes.
    const list = h('div', { class: 'settings-nav-list' });
    nav.append(h('div', { class: 'settings-user' }, h('span', { class: 'avatar' }, initials(name)), h('div', {}, h('strong', {}, name), h('small', { class: 'muted' }, 'Local profile'))), search, list);
    const renderNav = () => {
      clear(list);
      if (query) {
        const words = query.split(/\s+/);
        const hits = SETTINGS_INDEX.filter(([label, , keys]) => words.every((w) => `${label} ${keys}`.toLowerCase().includes(w)));
        list.append(h('div', { class: 'nav-heading' }, hits.length ? 'Settings' : 'Nothing found'));
        for (const [label, section] of hits) {
          const where = SECTIONS.find((x) => x.id === section)!;
          list.append(
            h(
              'button',
              { class: 'nav-item settings-hit', on: { click: () => { current = section; focusLabel = label; void renderBody(); } } },
              icon(where.icon, 16),
              h('span', { class: 'grow' }, label, h('small', { class: 'muted' }, ` · ${where.label}`))
            )
          );
        }
        return;
      }
      let group = '';
      for (const s of SECTIONS) {
        if (s.group !== group) {
          group = s.group;
          list.append(h('div', { class: 'nav-heading' }, group));
        }
        list.append(h('button', { class: `nav-item${s.id === current ? ' active' : ''}`, on: { click: () => { current = s.id; renderNav(); void renderBody(); } } }, icon(s.icon, 16), s.label));
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
      // From a search: bring the setting into view and flash its card.
      if (focusLabel) {
        const wanted = focusLabel.toLowerCase();
        focusLabel = null;
        const target = [...content.querySelectorAll<HTMLElement>('h3, .setting-row strong')].find((el) => (el.textContent ?? '').toLowerCase().startsWith(wanted.split(' (')[0]!));
        const box = target?.closest<HTMLElement>('.card, .setting-row') ?? target;
        if (box) {
          box.scrollIntoView({ block: 'center' });
          box.classList.add('flash');
          setTimeout(() => box.classList.remove('flash'), 1600);
        }
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
  const dataCard = card();
  /** Storage, backups, anything damaged, and the conversations deleted in the last 30 days. */
  const renderData = async () => {
    const [health, trash] = await Promise.all([api.storageHealth(), api.trash()]);
    const size = health.bytes > 1_048_576 ? `${(health.bytes / 1_048_576).toFixed(1)} MB` : `${Math.max(1, Math.round(health.bytes / 1024))} KB`;
    const refresh = () => { window.dispatchEvent(new Event('chat:conversations-changed')); void renderData(); };
    clear(
      dataCard,
      h('h3', {}, 'Storage and backups'),
      h(
        'p',
        { class: 'muted' },
        `${health.conversations.toLocaleString()} conversations, ${size} on this device. `,
        health.newestBackup
          ? `A copy of each conversation is kept before its first change of the day, for ${health.keepDays} days (${health.backupDays} day${health.backupDays === 1 ? '' : 's'} so far, newest ${health.newestBackup}).`
          : `A copy of each conversation is kept before its first change of the day, for ${health.keepDays} days.`
      ),
      health.damaged.length
        ? h(
            'div',
            { class: 'data-warning' },
            h('strong', {}, `${health.damaged.length} conversation file${health.damaged.length === 1 ? '' : 's'} cannot be read`),
            health.damaged.map((d) =>
              h(
                'div',
                { class: 'data-row' },
                h('span', { class: 'grow mono' }, d.id),
                d.backup
                  ? button({ variant: 'secondary', size: 'sm', label: `Restore the ${d.backup} backup`, onClick: async () => { try { await api.recoverConversation(d.id); toast('Restored from the backup.'); refresh(); } catch (e) { toast(String(e), 'error'); } } })
                  : h('span', { class: 'muted' }, 'No backup yet; the file is kept as it is.')
              )
            )
          )
        : null,
      h('h4', {}, 'Recently deleted'),
      trash.length
        ? h(
            'div',
            { class: 'data-list' },
            trash.map((t) =>
              h(
                'div',
                { class: 'data-row' },
                h('span', { class: 'grow' }, t.title),
                h('span', { class: 'muted' }, `${t.daysLeft} day${t.daysLeft === 1 ? '' : 's'} left`),
                button({ variant: 'secondary', size: 'sm', label: 'Restore', onClick: async () => { try { await api.restoreConversation(t.id); toast('Conversation restored.'); refresh(); } catch (e) { toast(String(e), 'error'); } } }),
                button({ variant: 'subtle', size: 'sm', label: 'Delete now', onClick: async () => { await api.deleteForever(t.id); void renderData(); } })
              )
            ),
            h('div', { class: 'card-actions' }, button({ variant: 'danger', size: 'sm', label: 'Empty', onClick: async () => { await api.deleteForever(null); void renderData(); } }))
          )
        : h('p', { class: 'muted' }, `Nothing. Deleted conversations wait here for ${health.keepDays} days.`)
    );
  };
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
    uploadsCard(),
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
        }),
        button({ variant: 'secondary', icon: 'code', label: 'Import coding sessions', title: 'Claude Code and Codex sessions on this computer become Code chats', onClick: () => importCodingSessions() })
      )
    ),
    dataCard,
    encryptionCard(),
    syncCard(() => void renderData()),
    sharedLinksCard(),
    activityLogCard()
  );
  void renderData();
}

/**
 * Sync between devices (Osaurus): conversations, agents, knowledge, memories, projects, canvases
 * and skills, encrypted with a passphrase, through a folder a sync service mirrors.
 */
function syncCard(onSynced: () => void): HTMLElement {
  type View = { folder: string | null; everyMinutes: number; lastSync: string | null; lastReport: string; hasPassphrase: boolean };
  let view: View = { folder: null, everyMinutes: 0, lastSync: null, lastReport: '', hasPassphrase: false };
  const box = card();
  const render = () => {
    let folder = view.folder ?? '';
    const folderLabel = h('span', { class: folder ? 'grow mono root-path' : 'grow muted' }, folder || 'No folder chosen');
    const choose = button({
      variant: 'secondary',
      size: 'sm',
      icon: 'folder',
      label: folder ? 'Change' : 'Choose folder',
      onClick: async () => {
        const dir = await api.pickDirectory();
        if (dir) {
          folder = dir;
          folderLabel.textContent = dir;
          folderLabel.className = 'grow mono root-path';
        }
      },
    });
    const passphrase = h('input', { class: 'input', type: 'password', autocomplete: 'new-password', placeholder: view.hasPassphrase ? 'Saved in the keychain; type to change' : 'At least 8 characters, the same on every device', 'aria-label': 'Sync passphrase' });
    const every = h(
      'select',
      { class: 'input', 'aria-label': 'How often' },
      ...[[0, 'Only when I press Sync now'], [5, 'Every 5 minutes'], [15, 'Every 15 minutes'], [60, 'Every hour']].map(([v, label]) => h('option', { value: String(v), selected: view.everyMinutes === v }, String(label)))
    );
    const status = h('p', { class: 'muted small' }, view.lastSync ? `Last sync ${new Date(view.lastSync).toLocaleString(undefined, { hour12: clockHour12() })}: ${view.lastReport}` : view.folder ? 'Not synced yet.' : '');
    const save = async () => {
      if (!folder) return toast('Choose the folder your sync service mirrors.', 'error');
      try {
        view = await tauriInvoke<View>('chat_save_sync_settings', { folder, everyMinutes: Number(every.value), passphrase: passphrase.value || null });
        toast('Sync is set up.');
        await syncNow();
      } catch (e) {
        toast(String(e), 'error');
      }
    };
    const syncNow = async () => {
      clear(status, h('span', { class: 'dots' }), ' Syncing…');
      try {
        const summary = await tauriInvoke<string>('chat_sync_now');
        toast(summary);
        window.dispatchEvent(new Event('chat:conversations-changed'));
        onSynced();
      } catch (e) {
        toast(String(e), 'error');
      }
      view = await tauriInvoke<View>('chat_sync_settings').catch(() => view);
      render();
    };
    clear(
      box,
      h('h3', {}, 'Sync between devices'),
      h('p', { class: 'muted' }, 'Keep conversations, agents, knowledge, memories, projects, canvases and skills the same on your computers through a folder that Syncthing, Nextcloud, Dropbox, iCloud Drive or OneDrive mirrors. Everything is encrypted with your passphrase before it is written there; settings, API keys and the vault stay on each device.'),
      h('label', { class: 'field-label' }, 'Folder'),
      h('div', { class: 'root-row' }, icon('folder', 16), folderLabel, choose),
      h('label', { class: 'field-label' }, 'Passphrase'),
      passphrase,
      h('label', { class: 'field-label' }, 'Sync automatically'),
      every,
      status,
      h(
        'div',
        { class: 'card-actions' },
        view.folder
          ? button({
              variant: 'secondary',
              label: 'Turn off',
              onClick: async () => {
                if (!(await confirmDialog('Turn off sync?', 'This device stops syncing and forgets the passphrase. Nothing is deleted here or in the folder.'))) return;
                view = await tauriInvoke<View>('chat_save_sync_settings', { folder: null, everyMinutes: 0, passphrase: null });
                render();
              },
            })
          : null,
        view.folder ? button({ variant: 'secondary', icon: 'reset', label: 'Sync now', onClick: () => void syncNow() }) : null,
        button({ variant: 'primary', label: 'Save', onClick: () => void save() })
      )
    );
  };
  render();
  void tauriInvoke<View>('chat_sync_settings').then((v) => {
    view = v;
    render();
  }).catch(() => undefined);
  return box;
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
  content.append(await backupModelCard());
  content.append(await reviewModelCard());
  content.append(await autoModelCard());
  content.append(await localModelsCard());
  content.append(await mediaCard());
  content.append(await extractionCard());
  content.append(await acpCard());
  content.append(await searchCard());
  content.append(await embeddingCard());
}

interface SearchSettings {
  engine: 'auto' | 'searxng' | 'brave' | 'tavily';
  baseUrl: string;
  hasKey: boolean;
}

/** Which engine `web_search` uses: keyless by default, or SearXNG, Brave or Tavily. */
async function searchCard(): Promise<HTMLElement> {
  let current = await tauriInvoke<SearchSettings>('chat_search_settings');
  const ENGINES: [SearchSettings['engine'], string, string][] = [
    ['auto', 'Keyless (default)', 'Bing results, with DuckDuckGo as a fallback. No account needed.'],
    ['searxng', 'SearXNG', 'Your own or a trusted instance; private and free. JSON output must be enabled on it.'],
    ['brave', 'Brave Search', 'Independent index; needs a Brave Search API key.'],
    ['tavily', 'Tavily', 'Made for AI agents; needs a Tavily API key.'],
  ];
  const engine = h('select', { class: 'input' }, ENGINES.map(([v, label]) => h('option', { value: v, selected: v === current.engine }, label)));
  const hint = h('small', { class: 'muted' });
  const baseUrl = h('input', { class: 'input', value: current.baseUrl, placeholder: 'https://searx.example.org' });
  const key = h('input', { class: 'input', type: 'password', placeholder: current.hasKey ? 'Saved in the keychain' : 'API key' });
  const urlField = h('div', { class: 'field' }, h('label', { class: 'field-label' }, 'Instance address'), baseUrl);
  const keyField = h('div', { class: 'field' }, h('label', { class: 'field-label' }, 'API key'), key);
  const sync = () => {
    const v = engine.value as SearchSettings['engine'];
    hint.textContent = ENGINES.find((e) => e[0] === v)?.[2] ?? '';
    urlField.hidden = v !== 'searxng';
    keyField.hidden = v !== 'brave' && v !== 'tavily';
  };
  engine.addEventListener('change', sync);
  sync();
  const save = h('button', { class: 'btn btn-primary' }, 'Save');
  save.addEventListener('click', async () => {
    try {
      current = await tauriInvoke<SearchSettings>('chat_save_search_settings', { settings: { engine: engine.value, baseUrl: baseUrl.value, hasKey: current.hasKey }, key: key.value.trim() || null });
      key.value = '';
      key.placeholder = current.hasKey ? 'Saved in the keychain' : 'API key';
      toast('Saved. If the engine fails, the keyless search answers instead.');
    } catch (e) {
      toast(String(e), 'error');
    }
  });
  return card(
    h('h3', {}, 'Web search'),
    h('p', { class: 'muted' }, 'The engine the assistant searches the web with.'),
    h('label', { class: 'field-label' }, 'Engine'),
    engine,
    hint,
    urlField,
    keyField,
    h('div', { class: 'card-actions' }, save)
  );
}

interface EmbeddingSettings {
  enabled: boolean;
  baseUrl: string;
  model: string;
  hasKey: boolean;
  rerankUrl: string;
  rerankModel: string;
  hasRerankKey: boolean;
  vectorStore: string;
  qdrantUrl: string;
  hasQdrantKey: boolean;
}

/** Semantic search for libraries: an OpenAI-compatible embedding model, fused with keyword search. */
async function embeddingCard(): Promise<HTMLElement> {
  let current = await tauriInvoke<EmbeddingSettings>('chat_embedding_settings');
  const PRESETS: [string, string, string][] = [
    ['Built-in (MiniLM)', 'builtin', 'minilm-l6'],
    ['Ollama (this computer)', 'http://localhost:11434/v1', 'nomic-embed-text'],
    ['LM Studio (this computer)', 'http://localhost:1234/v1', 'text-embedding-nomic-embed-text-v1.5'],
    ['OpenAI', 'https://api.openai.com/v1', 'text-embedding-3-small'],
    ['Mistral', 'https://api.mistral.ai/v1', 'mistral-embed'],
  ];
  const baseUrl = h('input', { class: 'input', value: current.baseUrl, placeholder: 'http://localhost:11434/v1, or builtin' });
  const model = h('input', { class: 'input', value: current.model, placeholder: 'nomic-embed-text' });
  const key = h('input', { class: 'input', type: 'password', placeholder: current.hasKey ? 'Saved in the keychain' : 'Not needed for local servers' });
  const presets = h(
    'div',
    { class: 'preset-row' },
    PRESETS.map(([label, url, m]) => h('button', { class: 'chip-btn', on: { click: () => { baseUrl.value = url; model.value = m; } } }, label))
  );
  // Reranking: a model that orders the found passages by how well they answer the question.
  const rerankUrl = h('input', { class: 'input', value: current.rerankUrl ?? '', placeholder: 'Off. For example http://localhost:8080/v1' });
  const rerankModel = h('input', { class: 'input', value: current.rerankModel ?? '', placeholder: 'bge-reranker-v2-m3' });
  const rerankKey = h('input', { class: 'input', type: 'password', placeholder: current.hasRerankKey ? 'Saved in the keychain' : 'Not needed for local servers' });
  const RERANKERS: [string, string, string][] = [
    ['llama.cpp (this computer)', 'http://localhost:8080/v1', 'bge-reranker-v2-m3'],
    ['Jina', 'https://api.jina.ai/v1', 'jina-reranker-v2-base-multilingual'],
    ['Cohere', 'https://api.cohere.com/v2', 'rerank-v3.5'],
    ['Off', '', ''],
  ];
  const rerankPresets = h(
    'div',
    { class: 'preset-row' },
    RERANKERS.map(([label, url, m]) => h('button', { class: 'chip-btn', on: { click: () => { rerankUrl.value = url; rerankModel.value = m; } } }, label))
  );
  // Where vectors are searched: here, or a Qdrant server.
  const store = h('select', { class: 'input' }, h('option', { value: 'local' }, 'This computer (no setup)'), h('option', { value: 'qdrant' }, 'Qdrant'));
  store.value = current.vectorStore === 'qdrant' ? 'qdrant' : 'local';
  const qdrantUrl = h('input', { class: 'input', value: current.qdrantUrl ?? '', placeholder: 'http://localhost:6333' });
  const qdrantKey = h('input', { class: 'input', type: 'password', placeholder: current.hasQdrantKey ? 'Saved in the keychain' : 'Only for Qdrant Cloud or a server with a key' });
  const qdrantStatus = h('small', { class: 'muted' });
  const checkQdrant = h('button', { class: 'btn btn-secondary' }, 'Check server');
  const qdrantFields = h('div', { class: 'qdrant-fields' }, h('label', { class: 'field-label' }, 'Qdrant server'), qdrantUrl, h('label', { class: 'field-label' }, 'Qdrant API key'), qdrantKey, h('div', { class: 'card-actions' }, qdrantStatus, checkQdrant));
  const showStore = () => { qdrantFields.hidden = store.value !== 'qdrant'; };
  store.addEventListener('change', showStore);
  showStore();
  let enabled = current.enabled;
  const status = h('small', { class: 'muted' });
  const save = h('button', { class: 'btn btn-primary' }, 'Save');
  const indexNow = h('button', { class: 'btn btn-secondary' }, 'Embed documents now');
  const persist = async () => {
    current = await tauriInvoke<EmbeddingSettings>('chat_save_embedding_settings', {
      settings: { enabled, baseUrl: baseUrl.value, model: model.value, hasKey: current.hasKey, rerankUrl: rerankUrl.value, rerankModel: rerankModel.value, hasRerankKey: current.hasRerankKey, vectorStore: store.value, qdrantUrl: qdrantUrl.value, hasQdrantKey: current.hasQdrantKey },
      key: key.value.trim() || null,
      clearKey: false,
      rerankKey: rerankKey.value.trim() || null,
      qdrantKey: qdrantKey.value.trim() || null,
    });
    key.value = '';
    rerankKey.value = '';
    qdrantKey.value = '';
    qdrantKey.placeholder = current.hasQdrantKey ? 'Saved in the keychain' : 'Only for Qdrant Cloud or a server with a key';
    rerankKey.placeholder = current.hasRerankKey ? 'Saved in the keychain' : 'Not needed for local servers';
    key.placeholder = current.hasKey ? 'Saved in the keychain' : 'Not needed for local servers';
  };
  save.addEventListener('click', async () => {
    try {
      await persist();
      toast('Saved.');
    } catch (e) {
      toast(String(e), 'error');
    }
  });
  checkQdrant.addEventListener('click', async () => {
    qdrantStatus.textContent = 'Checking…';
    try {
      await persist();
      qdrantStatus.textContent = `Connected: ${await tauriInvoke<string>('chat_check_vector_store')}.`;
    } catch (e) {
      qdrantStatus.textContent = String(e);
    }
  });
  indexNow.addEventListener('click', async () => {
    indexNow.setAttribute('disabled', '');
    status.textContent = 'Embedding library documents…';
    try {
      await persist();
      const n = await tauriInvoke<number>('chat_index_libraries');
      status.textContent = n ? `Embedded ${n} passages. Library searches now match by meaning too.` : 'Every document is already embedded.';
    } catch (e) {
      status.textContent = String(e);
    } finally {
      indexNow.removeAttribute('disabled');
    }
  });
  return card(
    h('h3', {}, 'Semantic search'),
    h('p', { class: 'muted' }, 'Library searches always match words. With an embedding model they also match meaning ("cancel the contract" finds the termination clause), and both rankings are merged. Use the built-in model (download it under Built-in models) or any OpenAI-compatible embeddings server, including Ollama on this computer.'),
    row('Use semantic search', 'Documents are embedded once and kept with the library.', switchInput(enabled, (v) => { enabled = v; })),
    presets,
    h('label', { class: 'field-label' }, 'Server base URL'),
    baseUrl,
    h('label', { class: 'field-label' }, 'Embedding model'),
    model,
    h('label', { class: 'field-label' }, 'API key'),
    key,
    h('h4', { class: 'subhead' }, 'Vector store'),
    h('p', { class: 'muted' }, 'Where passage vectors are searched. This computer needs nothing more; Qdrant (on your machine or Qdrant Cloud) suits very large libraries. Vectors stay cached here either way, so switching never embeds again.'),
    store,
    qdrantFields,
    h('h4', { class: 'subhead' }, 'Reranking (optional)'),
    h('p', { class: 'muted' }, 'A reranking model reads the question beside each passage found and puts the ones that answer it first. Any server with the Cohere or Jina rerank API works (llama.cpp, vLLM, Infinity, TEI, Jina, Cohere); it works with or without semantic search.'),
    rerankPresets,
    h('label', { class: 'field-label' }, 'Reranker base URL'),
    rerankUrl,
    h('label', { class: 'field-label' }, 'Reranking model'),
    rerankModel,
    h('label', { class: 'field-label' }, 'Reranker API key'),
    rerankKey,
    status,
    h('div', { class: 'card-actions' }, indexNow, save)
  );
}

interface MediaModels {
  imageModel: string | null;
  speechModel: string | null;
  voiceModel: string | null;
  voiceVendors: string[];
  imageVendors: string[];
  speechVendors: string[];
  imageServer?: string | null;
  videoModel?: string | null;
  videoVendors?: string[];
}

/** ACP agents (Agent Client Protocol): coding agents that answer as models, with their own tools. */
async function acpCard(): Promise<HTMLElement> {
  type Acp = { id: string; name: string; command: string; args: string[] };
  let all = await tauriInvoke<Acp[]>('chat_acp_agents', { agents: null }).catch(() => [] as Acp[]);
  const list = h('div', { class: 'hook-list' });
  const own = () => all.filter((a) => !['claude-code', 'codex', 'gemini'].includes(a.id) || a.command.includes('/'));
  const save = async (next: Acp[]) => {
    try {
      all = await tauriInvoke<Acp[]>('chat_acp_agents', { agents: next });
      render();
      await reloadModels();
    } catch (e) {
      toast(String(e), 'error');
    }
  };
  const render = () =>
    clear(
      list,
      all.length
        ? all.map((a) =>
            h(
              'div',
              { class: 'hook-row' },
              h('div', { class: 'hook-text' }, h('strong', {}, a.name), h('code', {}, [a.command, ...a.args].join(' '))),
              own().some((o) => o.id === a.id) ? h('button', { class: 'icon-btn small', title: 'Remove', on: { click: () => void save(own().filter((o) => o.id !== a.id)) } }, icon('trash', 15)) : h('small', { class: 'muted' }, 'found on this computer')
            )
          )
        : h('p', { class: 'muted small' }, 'None found. Install claude-code-acp, codex-acp or the Gemini CLI, or add an agent below.')
    );
  render();
  const name = h('input', { class: 'input', placeholder: 'Name, e.g. Goose' });
  const command = h('input', { class: 'input mono', placeholder: 'Command, e.g. goose acp' });
  const add = button({ variant: 'secondary', icon: 'plus', label: 'Add', onClick: () => {
    const parts = command.value.trim().split(/\s+/).filter(Boolean);
    if (!name.value.trim() || !parts.length) return toast('Give the agent a name and its command.', 'error');
    const id = name.value.trim().toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'agent';
    void save([...own().filter((o) => o.id !== id), { id, name: name.value.trim(), command: parts[0]!, args: parts.slice(1) }]);
    name.value = '';
    command.value = '';
  } });
  return card(
    h('h3', {}, 'ACP agents'),
    h('p', { class: 'muted' }, 'Coding agents that speak the Agent Client Protocol answer as models in the model menu: they work in the chat’s folder with their own tools, stream their answer and thoughts, and ask you before acting.'),
    list,
    h('div', { class: 'hook-add' }, name, command, add)
  );
}

/** Which engine reads documents: the built-in readers, Apache Tika or Docling (Open WebUI's extraction engines). */
async function extractionCard(): Promise<HTMLElement> {
  type Extraction = { engine: string; url: string };
  const now = await tauriInvoke<Extraction>('chat_extraction', { change: null }).catch(() => ({ engine: 'builtin', url: '' }));
  const engine = h('select', { class: 'input select-sm' }, [['builtin', 'Built-in readers'], ['tika', 'Apache Tika'], ['docling', 'Docling']].map(([v, l]) => h('option', { value: v, selected: (now.engine || 'builtin') === v }, l)));
  const url = h('input', { class: 'input', value: now.url, placeholder: 'http://localhost:9998 (Tika) or http://localhost:5001 (Docling)' });
  const sync = () => { url.disabled = engine.value === 'builtin'; };
  engine.addEventListener('change', () => {
    if (!url.value) url.value = engine.value === 'tika' ? 'http://localhost:9998' : engine.value === 'docling' ? 'http://localhost:5001' : '';
    sync();
  });
  sync();
  const save = button({ variant: 'primary', label: 'Save', onClick: async () => {
    try {
      await tauriInvoke('chat_extraction', { change: { engine: engine.value, url: url.value.trim() } });
      toast(engine.value === 'builtin' ? 'Documents are read by the built-in readers.' : `Documents are read by ${engine.value === 'tika' ? 'Tika' : 'Docling'}, with the built-in readers as a fallback.`);
    } catch (e) {
      toast(String(e), 'error');
    }
  } });
  return card(
    h('h3', {}, 'Document extraction'),
    h('p', { class: 'muted' }, 'How text is taken from files you add to libraries or attach in chat. Tika (with OCR) and Docling (layouts and tables, as Markdown) run as servers on your machine; if one fails, the built-in readers take over.'),
    h('div', { class: 'row-inline' }, engine, url),
    h('div', { class: 'card-actions' }, save)
  );
}

interface LocalModel {
  id: string;
  name: string;
  kind: 'chat' | 'speech' | 'embedding';
  sizeMb: number;
  note: string;
  installed: boolean;
  downloading: boolean;
}

/**
 * Built-in models (Osaurus's and LM Studio's local models): chat and speech models that run in
 * Cortex itself, downloaded once from Hugging Face, then offline.
 */
async function localModelsCard(): Promise<HTMLElement> {
  const body = h('div', { class: 'local-models' });
  const progress = new Map<string, { done: number; total: number }>();
  const size = (mb: number) => (mb >= 1000 ? `${(mb / 1000).toFixed(1)} GB` : `${mb} MB`);
  const draw = async () => {
    const list = await tauriInvoke<LocalModel[]>('chat_local_models').catch(() => []);
    const row = (m: LocalModel) => {
      const p = progress.get(m.id);
      const pct = p && p.total ? Math.floor((p.done / p.total) * 100) : 0;
      const action = m.installed
        ? button({ variant: 'secondary', label: 'Delete', onClick: async () => {
            if (!(await confirmDialog(`Delete ${m.name}?`, `${size(m.sizeMb)} are freed. You can download it again.`, 'Delete'))) return;
            await tauriInvoke('chat_remove_local_model', { id: m.id }).catch((e) => toast(String(e), 'error'));
            void draw();
            void reloadModels().catch(() => undefined);
          } })
        : m.downloading || p
          ? button({ variant: 'secondary', label: 'Cancel', onClick: () => void tauriInvoke('chat_cancel_local_model', { id: m.id }) })
          : button({ variant: 'primary', label: 'Download', onClick: async () => {
              progress.set(m.id, { done: 0, total: m.sizeMb * 1_000_000 });
              void draw();
              try {
                await tauriInvoke('chat_download_local_model', { id: m.id });
                toast(`${m.name} is ready. ${m.kind === 'chat' ? 'Pick it in the model menu, under Built-in.' : m.kind === 'embedding' ? 'Choose Built-in under Semantic search.' : 'Choose Built-in under Settings → Audio → Speech-to-text.'}`);
              } catch (e) {
                toast(String(e), 'error');
              }
              progress.delete(m.id);
              void draw();
              void reloadModels().catch(() => undefined);
            } });
      return h(
        'div',
        { class: 'local-model', dataset: { model: m.id } },
        h('div', { class: 'local-model-text' }, h('strong', {}, m.name), h('small', { class: 'muted' }, `${size(m.sizeMb)} · ${m.note}`), p ? h('div', { class: 'local-progress' }, h('div', { class: 'local-progress-bar', style: { width: `${pct}%` } }), h('span', { class: 'muted small' }, `${pct}%`)) : null),
        m.installed ? h('span', { class: 'chip-ok' }, icon('check', 13), 'Ready') : null,
        action
      );
    };
    clear(
      body,
      h('h4', {}, 'Chat'),
      list.filter((m) => m.kind === 'chat').map(row),
      h('h4', {}, 'Speech-to-text (Whisper)'),
      list.filter((m) => m.kind === 'speech').map(row),
      h('h4', {}, 'Embeddings (semantic search)'),
      list.filter((m) => m.kind === 'embedding').map(row)
    );
  };
  void listen<{ id: string; done: number; total: number }>('local-model-progress', (p) => {
    progress.set(p.id, { done: p.done, total: p.total });
    const bar = body.querySelector<HTMLElement>(`[data-model="${p.id}"] .local-progress-bar`);
    const label = body.querySelector<HTMLElement>(`[data-model="${p.id}"] .local-progress span`);
    const pct = p.total ? Math.floor((p.done / p.total) * 100) : 0;
    if (bar && label) {
      bar.style.width = `${pct}%`;
      label.textContent = `${pct}%`;
    } else void draw();
  });
  await draw();
  return card(
    h('h3', {}, 'Built-in models'),
    h('p', { class: 'muted' }, 'Models that run inside Cortex, on this computer: nothing to install, no account, and they work offline once downloaded (from Hugging Face). Chat models appear in the model menu under Built-in; Whisper models transcribe the microphone and dictation when Settings → Audio uses Built-in.'),
    body
  );
}

/** Which models make images and transcribe voice input. */
async function mediaCard(): Promise<HTMLElement> {
  const media = await tauriInvoke<MediaModels>('chat_media_models');
  const image = h('input', { class: 'input', value: media.imageModel ?? '', placeholder: 'xai/grok-imagine-image, openai/gpt-image-1' });
  const speech = h('input', { class: 'input', value: media.speechModel ?? '', placeholder: 'openai/gpt-4o-mini-transcribe, groq/whisper-large-v3-turbo' });
  const voice = h('input', { class: 'input', value: media.voiceModel ?? '', placeholder: 'xai/grok-tts, openai/gpt-4o-mini-tts' });
  const server = h('input', { class: 'input', value: media.imageServer ?? '', placeholder: 'http://127.0.0.1:7860' });
  const video = h('input', { class: 'input', value: media.videoModel ?? '', placeholder: 'xai/grok-imagine-video' });
  const serverSave = button({ variant: 'secondary', label: 'Connect', onClick: async () => {
    try {
      const saved = await tauriInvoke<MediaModels>('chat_set_image_server', { url: server.value.trim() || null });
      image.value = saved.imageModel ?? '';
      toast(saved.imageServer ? 'The local image server answered.' : 'Local image server removed.');
    } catch (e) {
      toast(String(e), 'error');
    }
  } });
  const save = h('button', { class: 'btn btn-primary' }, 'Save');
  save.addEventListener('click', async () => {
    try {
      const saved = await tauriInvoke<MediaModels>('chat_set_media_models', { imageModel: image.value.trim() || null, speechModel: speech.value.trim() || null, voiceModel: voice.value.trim() || null, videoModel: video.value.trim() || null });
      image.value = saved.imageModel ?? '';
      speech.value = saved.speechModel ?? '';
      voice.value = saved.voiceModel ?? '';
      video.value = saved.videoModel ?? '';
      toast(saved.imageModel || saved.speechModel ? 'Saved.' : 'Saved. No provider set up can serve these yet.');
    } catch (e) {
      toast(String(e), 'error');
    }
  });
  const note = (vendors: string[], what: string) =>
    h('small', { class: 'muted' }, vendors.length ? `Available from: ${vendors.join(', ')}. Leave empty for the default.` : `Set up a provider that offers ${what} to use this.`);
  return card(
    h('h3', {}, 'Images, video and voice'),
    h('p', { class: 'muted' }, 'The assistant creates images with the image model and clips with the video model; the microphone in the composer transcribes with the speech model; the speaker under an answer reads it with the read-aloud model.'),
    h('label', { class: 'field-label' }, 'Image model'),
    image,
    note(media.imageVendors, 'image generation (OpenAI, xAI or Together)'),
    h('label', { class: 'field-label', title: 'Stable Diffusion WebUI (started with --api), Forge, SD.Next or Draw Things: images made on your own computer' }, 'Local image server'),
    h('div', { class: 'row-inline' }, server, serverSave),
    h('small', { class: 'muted' }, 'Its models show as sdwebui/<checkpoint>; with no other image model, it is used by itself.'),
    h('label', { class: 'field-label' }, 'Video model'),
    video,
    note(media.videoVendors ?? [], 'video generation (xAI)'),
    h('label', { class: 'field-label' }, 'Speech-to-text model'),
    speech,
    note(media.speechVendors, 'speech-to-text (OpenAI or Groq)'),
    h('label', { class: 'field-label' }, 'Read-aloud model'),
    voice,
    note(media.voiceVendors, 'text-to-speech (OpenAI or xAI)'),
    h('div', { class: 'card-actions' }, save)
  );
}

/** The activity log (OpenBot's audit log): every tool call and approval, exported on demand. */
function activityLogCard(): HTMLElement {
  const summary = h('p', { class: 'muted' }, 'Loading…');
  const range = h('select', { class: 'input', 'aria-label': 'Period' }, h('option', { value: '30' }, 'Last 30 days'), h('option', { value: '7' }, 'Last 7 days'), h('option', { value: '0' }, 'Everything kept'));
  const load = () =>
    void tauriInvoke<{ count: number; since: string | null; approvals: number; denied: number; errors: number }>('chat_audit_summary')
      .then((s) => {
        summary.textContent = s.count
          ? `${s.count} entries since ${new Date(s.since!).toLocaleDateString()}: ${s.count - s.approvals} tool calls (${s.errors} failed) and ${s.approvals} approval decisions (${s.denied} declined).`
          : 'Nothing yet. Tool calls and approvals are recorded here as they happen.';
      })
      .catch((e) => (summary.textContent = String(e)));
  const exportAs = (format: 'csv' | 'json') => async () => {
    try {
      if (await tauriInvoke<boolean>('chat_export_audit', { format, days: Number(range.value) || null })) toast('Activity log saved.');
    } catch (e) {
      toast(String(e), 'error');
    }
  };
  const clearBtn = button({ variant: 'secondary', label: 'Clear', onClick: async () => {
    if (!(await confirmDialog('Clear the activity log?', 'Every recorded tool call and approval is removed from this computer.', 'Clear'))) return;
    await tauriInvoke('chat_clear_audit').catch((e) => toast(String(e), 'error'));
    load();
  } });
  load();
  return card(
    h('h3', {}, 'Activity log'),
    h('p', { class: 'muted' }, 'Every tool the assistant used and every approval you gave or declined: when, in which chat, for which agent, and how it ended. Kept on this computer for a year, encrypted when encryption is on; secrets from the vault are masked.'),
    summary,
    h('div', { class: 'card-actions' }, range, button({ variant: 'secondary', icon: 'download', label: 'Export CSV', onClick: exportAs('csv') }), button({ variant: 'secondary', icon: 'download', label: 'Export JSON', onClick: exportAs('json') }), clearBtn)
  );
}

/** The model a reply switches to, once, when its own fails before answering. */
async function backupModelCard(): Promise<HTMLElement> {
  const current = await tauriInvoke<string | null>('chat_backup_model').catch(() => null);
  const select = h(
    'select',
    { class: 'input' },
    h('option', { value: '' }, 'None: show the error'),
    state.models.map((m) => h('option', { value: m.reference, selected: m.reference === current }, `${m.providerLabel} · ${m.name}`))
  );
  select.addEventListener('change', () =>
    void tauriInvoke('chat_set_backup_model', { reference: select.value || null }).then(() => toast('Saved.')).catch((e) => toast(String(e), 'error'))
  );
  return card(
    h('h3', {}, 'Backup model'),
    h('p', { class: 'muted' }, 'When a reply\'s model fails before answering (a provider down, an account out of credit, a key refused), the reply continues once with this model and says so. An agent\'s own backup model comes first.'),
    select
  );
}

/** The model that reviews what the assistant writes for itself, before the user decides. */
async function reviewModelCard(): Promise<HTMLElement> {
  const current = await tauriInvoke<string | null>('chat_review_model').catch(() => null);
  const select = h(
    'select',
    { class: 'input' },
    h('option', { value: '' }, 'The chat\'s own model'),
    state.models.map((m) => h('option', { value: m.reference, selected: m.reference === current }, `${m.providerLabel} · ${m.name}`))
  );
  select.addEventListener('change', () =>
    void tauriInvoke('chat_set_review_model', { reference: select.value || null }).then(() => toast('Saved.')).catch((e) => toast(String(e), 'error'))
  );
  return card(
    h('h3', {}, 'Reviewer model'),
    h('p', { class: 'muted' }, 'Before you approve a tool the assistant wrote or a change it proposes, another model reviews it and its verdict is on the card. A different model from the chat\'s is a more independent reviewer.'),
    select
  );
}

/** Auto (Onyx's model router): a fast and a strong model, picked per message. */
async function autoModelCard(): Promise<HTMLElement> {
  const [fast, strong] = await tauriInvoke<[string | null, string | null]>('chat_router_models').catch(() => [null, null] as [null, null]);
  const choices = state.models.filter((m) => m.reference !== 'auto/router');
  const pick = (current: string | null) =>
    h('select', { class: 'input' }, h('option', { value: '' }, 'Not set'), choices.map((m) => h('option', { value: m.reference, selected: m.reference === current }, `${m.providerLabel} · ${m.name}`)));
  const fastSel = pick(fast);
  const strongSel = pick(strong);
  const save = () =>
    void tauriInvoke('chat_set_router_models', { fast: fastSel.value || null, strong: strongSel.value || null })
      .then(async () => {
        await reloadModels();
        toast(fastSel.value && strongSel.value ? 'Auto is in the model picker.' : 'Saved. Choose both for Auto to appear.');
      })
      .catch((e) => toast(String(e), 'error'));
  fastSel.addEventListener('change', save);
  strongSel.addEventListener('change', save);
  return card(
    h('h3', {}, 'Auto model'),
    h('p', { class: 'muted' }, 'With both set, "Auto" in the model picker sends each message to one of them: the strong model for code, analysis and design, long or attached input, images or several questions at once, and the fast one for everything else. The chat says which it chose and why.'),
    row('Fast model', 'Quick questions, small talk, short rewrites.', fastSel),
    row('Strong model', 'Code, analysis, planning, long documents.', strongSel)
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
                h('small', { class: 'muted', style: 'display:block' }, [s.description, s.everywhere ? 'Every chat and agent' : users(s.name).length ? `Agents: ${users(s.name).join(', ')}` : 'No agent yet: pick it in an agent’s settings'].filter(Boolean).join(' · '))
              ),
              h('button', { class: 'link-btn', on: { click: () => edit(s) } }, 'Edit'),
              h('button', {
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
              }, icon('trash', 14))
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
        row('Every chat and agent', 'Off: only the agents you pick can use it.', switchInput(everywhere, (v) => { everywhere = v; })),
        error,
        h('div', { class: 'dialog-actions' }, button({ variant: 'secondary', label: 'Cancel', onClick: close }), button({ variant: 'primary', label: 'Save', onClick: () => void save() }))
      );
    });
  render();
  return card(
    h('h3', {}, 'Vault'),
    h('p', { class: 'muted' }, 'API tokens and passwords for agents, kept in the OS keychain. An agent you allow gets them as environment variables in its shell commands, code and your own tools; the model only sees the names, and any value that shows up in a tool’s output is masked.'),
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
    const rows = [
      ...settings.calendars.map((c, i) => ({ c, kind: 'calendar' as const, i })),
      ...settings.contacts.map((c, i) => ({ c, kind: 'contacts' as const, i })),
    ];
    clear(
      list,
      rows.length
        ? rows.map(({ c, kind, i }) =>
            h(
              'div',
              { class: 'hook-row' },
              icon(kind === 'calendar' ? 'calendar' : 'user', 16),
              h('div', { class: 'hook-text' }, h('strong', {}, c.name), h('code', {}, c.location.replace(/(https?:\/\/[^/]+\/).{12,}/, '$1…'))),
              h('button', {
                class: 'icon-btn small',
                title: 'Remove',
                on: { click: () => void save(kind === 'calendar' ? { ...settings, calendars: settings.calendars.filter((_, j) => j !== i) } : { ...settings, contacts: settings.contacts.filter((_, j) => j !== i) }) },
              }, icon('trash', 15))
            )
          )
        : h('p', { class: 'muted small' }, 'None yet.')
    );
  };
  const kind = h('select', { class: 'input select-sm' }, h('option', { value: 'calendar' }, 'Calendar'), h('option', { value: 'contacts' }, 'Contacts'));
  const name = h('input', { class: 'input', placeholder: 'Name, e.g. Work' });
  const location = h('input', { class: 'input', placeholder: 'Private ICS link (https:// or webcal://) or a file' });
  const browse = button({ variant: 'subtle', size: 'sm', label: 'File…', onClick: async () => {
    const path = await tauriInvoke<string | null>('chat_pick_calendar_file', { contacts: kind.value === 'contacts' }).catch(() => null);
    if (path) {
      location.value = path;
      if (!name.value) name.value = path.split(/[\\/]/).pop()!.replace(/\.[^.]+$/, '');
    }
  } });
  const add = button({ variant: 'secondary', icon: 'plus', label: 'Add', onClick: () => {
    const entry = { name: name.value.trim() || (kind.value === 'calendar' ? 'Calendar' : 'Contacts'), location: location.value.trim() };
    if (!entry.location) return toast('Paste a link or choose a file.', 'error');
    void save(kind.value === 'calendar' ? { ...settings, calendars: [...settings.calendars, entry] } : { ...settings, contacts: [...settings.contacts, entry] }).then(() => {
      name.value = '';
      location.value = '';
    });
  } });
  render();
  return card(
    h('h3', {}, 'Calendars and contacts'),
    h('p', { class: 'muted' }, 'Let the assistant read your schedule and look people up. Add a calendar\u2019s private ICS link (Google, Proton, Outlook and iCloud all offer one) or an .ics file, and a .vcf contacts export. It only reads them; links are fetched when it asks.'),
    list,
    h('div', { class: 'calendar-add' }, kind, name, location, browse, add)
  );
}

interface HookDef {
  event: 'before_tool' | 'after_tool' | 'turn_end';
  matcher: string;
  command: string;
  enabled: boolean;
  /** The plugin it came with. */
  source?: string;
}

const HOOK_EVENTS: [HookDef['event'], string][] = [
  ['before_tool', 'Before a tool call'],
  ['after_tool', 'After a tool call'],
  ['turn_end', 'When a reply finishes'],
];

/**
 * Hooks (Vibe's and Claude Code's): commands of your own at points of a reply. A before-call hook
 * that exits with code 2 blocks the call and tells the model why.
 */
async function hooksCard(): Promise<HTMLElement> {
  let hooks = await tauriInvoke<HookDef[]>('chat_get_hooks').catch(() => [] as HookDef[]);
  const list = h('div', { class: 'hook-list' });
  const save = async () => {
    try {
      await tauriInvoke('chat_save_hooks', { hooks });
    } catch (e) {
      toast(String(e), 'error');
    }
  };
  const render = () => {
    clear(
      list,
      hooks.length
        ? hooks.map((hook, i) => {
            const toggle = switchInput(hook.enabled, (v) => { hook.enabled = v; void save(); });
            const remove = h('button', { class: 'icon-btn small', title: 'Remove', on: { click: () => { hooks.splice(i, 1); void save(); render(); } } }, icon('trash', 15));
            return h(
              'div',
              { class: 'hook-row' },
              h('div', { class: 'hook-text' }, h('strong', {}, HOOK_EVENTS.find(([e]) => e === hook.event)?.[1] ?? hook.event, hook.matcher && hook.event !== 'turn_end' ? h('small', { class: 'muted' }, ` · ${hook.matcher}`) : null, hook.source ? h('span', { class: 'badge', title: 'Came with this plugin; importing it again replaces these hooks' }, hook.source) : null), h('code', {}, hook.command)),
              toggle,
              remove
            );
          })
        : h('p', { class: 'muted small' }, 'No hooks yet.')
    );
  };
  const event = h('select', { class: 'input select-sm' }, HOOK_EVENTS.map(([value, label]) => h('option', { value }, label)));
  const matcher = h('input', { class: 'input', placeholder: 'Tools (empty for all), e.g. run_code, mcp-github' });
  const command = h('input', { class: 'input mono', placeholder: 'Command, e.g. ./check.sh (the event comes as JSON on its input)' });
  event.addEventListener('change', () => { matcher.disabled = event.value === 'turn_end'; });
  const add = button({ variant: 'secondary', icon: 'plus', label: 'Add hook', onClick: () => {
    if (!command.value.trim()) return toast('Write the command to run.', 'error');
    hooks = [...hooks, { event: event.value as HookDef['event'], matcher: event.value === 'turn_end' ? '' : matcher.value.trim(), command: command.value.trim(), enabled: true }];
    command.value = '';
    matcher.value = '';
    void save();
    render();
  } });
  render();
  return card(
    h('h3', {}, 'Hooks'),
    h('p', { class: 'muted' }, 'Your own commands at points of a reply: to check or log tool calls, format code after edits, or notify you when a reply is done. They get the event as JSON on their input and run in the chat\u2019s folder. A before-call hook that exits with code 2 blocks the call, and what it printed tells the assistant why; what an after-call hook prints is added to the result.'),
    list,
    h('div', { class: 'hook-add' }, event, matcher, command, add)
  );
}

interface CustomTool {
  name: string;
  description: string;
  parameters: unknown;
  language: 'python' | 'javascript' | 'bash';
  code: string;
  enabled: boolean;
}

/** Your own tools: a script and its input schema, run in the code sandbox on each call. */
async function customToolsCard(): Promise<HTMLElement> {
  const list = h('div', {});
  const render = (tools: CustomTool[]) =>
    clear(
      list,
      tools.length
        ? tools.map((t) =>
            h(
              'div',
              { class: 'setting-row' },
              h('div', {}, h('strong', {}, t.name), h('small', { class: 'muted' }, `${t.language} · ${t.description}`)),
              h('div', { class: 'row-actions' }, h('button', { class: 'link-btn', on: { click: () => void editTool(t, render) } }, 'Edit'), switchInput(t.enabled, (v) => void tauriInvoke<CustomTool[]>('chat_save_custom_tool', { tool: { ...t, enabled: v } }).then(render)))
            )
          )
        : h('p', { class: 'muted' }, 'No tools of your own yet.')
    );
  render(await tauriInvoke<CustomTool[]>('chat_custom_tools'));
  const add = h('button', { class: 'btn btn-primary' }, icon('plus', 15), 'New tool');
  add.addEventListener('click', () => void editTool(null, render));
  return card(
    h('h3', {}, 'Your tools'),
    h('p', { class: 'muted' }, 'Write a tool in Python, JavaScript or Bash: the assistant calls it with arguments that match its input schema (in the variable `args`) and reads what it prints. It runs in the code sandbox, and each call follows the approval mode.'),
    list,
    h('div', { class: 'card-actions' }, add)
  );
}

function editTool(existing: CustomTool | null, onSaved: (tools: CustomTool[]) => void): Promise<void> {
  return modal((close) => {
    const name = h('input', { class: 'input', value: existing?.name ?? '', placeholder: 'convert_units', disabled: Boolean(existing) });
    const description = h('input', { class: 'input', value: existing?.description ?? '', placeholder: 'What it does and when to use it' });
    const language = h('select', { class: 'input' }, (['python', 'javascript', 'bash'] as const).map((l) => h('option', { value: l, selected: (existing?.language ?? 'python') === l }, l === 'python' ? 'Python' : l === 'javascript' ? 'JavaScript' : 'Bash')));
    const schema = h('textarea', { class: 'input textarea mono', rows: 6 });
    schema.value = JSON.stringify(existing?.parameters ?? { type: 'object', properties: { km: { type: 'number', description: 'Distance in kilometres' } }, required: ['km'] }, null, 2);
    const code = h('textarea', { class: 'input textarea mono', rows: 8 });
    code.value = existing?.code ?? "print(round(args['km'] * 0.621371, 2), 'miles')";
    const sample = h('input', { class: 'input mono', value: '{"km": 10}' });
    const output = h('pre', { class: 'run-output' });
    const error = h('p', { class: 'error-text' });
    const current = (): CustomTool | null => {
      try {
        return { name: name.value.trim(), description: description.value.trim(), parameters: JSON.parse(schema.value), language: language.value as CustomTool['language'], code: code.value, enabled: existing?.enabled ?? true };
      } catch {
        error.textContent = 'The input schema is not valid JSON.';
        return null;
      }
    };
    const save = async (andClose: boolean) => {
      const tool = current();
      if (!tool) return false;
      try {
        onSaved(await tauriInvoke<CustomTool[]>('chat_save_custom_tool', { tool }));
        error.textContent = '';
        if (andClose) close();
        return true;
      } catch (e) {
        error.textContent = String(e);
        return false;
      }
    };
    const test = async () => {
      let args: unknown;
      try {
        args = JSON.parse(sample.value || '{}');
      } catch {
        output.textContent = 'The sample input is not valid JSON.';
        return;
      }
      if (!(await save(false))) return;
      output.textContent = 'Running…';
      if (!output.isConnected) sample.after(output);
      try {
        output.textContent = await tauriInvoke<string>('chat_test_custom_tool', { name: name.value.trim(), args });
        output.classList.remove('failed');
      } catch (e) {
        output.textContent = String(e);
        output.classList.add('failed');
      }
    };
    const remove = existing
      ? h('button', { class: 'btn btn-danger', on: { click: async () => { if (await confirmDialog('Delete tool?', `“${existing.name}” will no longer be offered to the assistant.`)) { onSaved(await tauriInvoke<CustomTool[]>('chat_delete_custom_tool', { name: existing.name })); close(); } } } }, 'Delete')
      : null;
    return h(
      'div',
      { class: 'dialog-small wide' },
      modalHeader(existing ? `Edit ${existing.name}` : 'New tool', close, { flush: true }),
      h('label', { class: 'field-label' }, 'Name'),
      name,
      h('label', { class: 'field-label' }, 'Description'),
      description,
      h('label', { class: 'field-label' }, 'Language'),
      language,
      h('label', { class: 'field-label' }, 'Input schema (JSON)'),
      schema,
      h('label', { class: 'field-label' }, 'Code — the arguments are in `args` (`$ARGS` as JSON in Bash); print the result'),
      code,
      h('label', { class: 'field-label' }, 'Try it with'),
      h('div', { class: 'freq-row' }, h('div', { class: 'grow' }, sample), h('button', { class: 'btn btn-secondary', on: { click: () => void test() } }, icon('play', 14), 'Test')),
      error,
      h(
        'div',
        { class: 'dialog-actions' },
        remove,
        existing
          ? historyButton('tools', existing.name, (v) => {
              description.value = typeof v.description === 'string' ? v.description : '';
              if (typeof v.language === 'string') language.value = v.language;
              schema.value = JSON.stringify(v.parameters ?? {}, null, 2);
              code.value = typeof v.code === 'string' ? v.code : '';
            })
          : null,
        h('span', { class: 'spacer' }),
        h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'),
        h('button', { class: 'btn btn-primary', on: { click: () => void save(true) } }, 'Save')
      )
    );
  });
}

interface CortexFunction {
  id: string;
  name: string;
  kind: 'filter' | 'pipe';
  description: string;
  language: 'python' | 'javascript' | 'bash';
  code: string;
  enabled: boolean;
}

const FILTER_SAMPLE = `import re
# Runs on each message before it is sent (stage "inlet") and on each answer (stage "outlet").
# Print the new text; print nothing to keep it; print {"block": "why"} to refuse it.
text = args['text']
if args['stage'] == 'inlet':
    text = re.sub(r'[\\w.+-]+@[\\w-]+\\.[\\w.]+', '[email]', text)
print(text)`;

const PIPE_SAMPLE = `# A model made of code: the conversation is in args['messages'] ({role, content}).
# Print the answer.
last = args['messages'][-1]['content']
print(f"You said {len(last.split())} words. The longest was: {max(last.split(), key=len)}")`;

/** Functions (Open WebUI's): filters that shape messages and answers, pipes that act as models. */
async function functionsCard(): Promise<HTMLElement> {
  const list = h('div', {});
  const render = (all: CortexFunction[]) =>
    clear(
      list,
      all.length
        ? all.map((f) =>
            h(
              'div',
              { class: 'setting-row' },
              h('div', {}, h('strong', {}, f.name), h('small', { class: 'muted' }, `${f.kind === 'pipe' ? 'Pipe (a model)' : 'Filter'} · ${f.language}${f.description ? ` · ${f.description}` : ''}`)),
              h('div', { class: 'row-actions' }, h('button', { class: 'link-btn', on: { click: () => void editFunction(f, render) } }, 'Edit'), switchInput(f.enabled, (v) => void tauriInvoke<CortexFunction[]>('chat_save_function', { function: { ...f, enabled: v } }).then((x) => { render(x); void reloadModels(); })))
            )
          )
        : h('p', { class: 'muted' }, 'No functions yet.')
    );
  render(await tauriInvoke<CortexFunction[]>('chat_functions').catch(() => []));
  const addFilter = button({ variant: 'secondary', icon: 'plus', label: 'New filter', onClick: () => void editFunction({ id: '', name: '', kind: 'filter', description: '', language: 'python', code: FILTER_SAMPLE, enabled: true }, render, true) });
  const addPipe = button({ variant: 'secondary', icon: 'plus', label: 'New pipe', onClick: () => void editFunction({ id: '', name: '', kind: 'pipe', description: '', language: 'python', code: PIPE_SAMPLE, enabled: true }, render, true) });
  return card(
    h('h3', {}, 'Functions'),
    h('p', { class: 'muted' }, 'Code that shapes your chats. A filter sees every message before it is sent and every answer before it is kept: it can take personal data out, add context, or refuse a message. A pipe is a model made of code, in the model menu under Your functions: it can call any API or program. Both run in the code sandbox.'),
    list,
    h('div', { class: 'card-actions' }, addFilter, addPipe)
  );
}

function editFunction(f: CortexFunction, onSaved: (all: CortexFunction[]) => void, isNew = false): Promise<void> {
  return modal((close) => {
    const id = h('input', { class: 'input', value: f.id, placeholder: f.kind === 'pipe' ? 'word-counter' : 'redact-emails', disabled: !isNew });
    const name = h('input', { class: 'input', value: f.name, placeholder: f.kind === 'pipe' ? 'Word counter' : 'Redact emails' });
    const description = h('input', { class: 'input', value: f.description, placeholder: 'What it does' });
    const language = h('select', { class: 'input' }, (['python', 'javascript', 'bash'] as const).map((l) => h('option', { value: l, selected: f.language === l }, l === 'python' ? 'Python' : l === 'javascript' ? 'JavaScript' : 'Bash')));
    const code = h('textarea', { class: 'input textarea mono', rows: 12 });
    code.value = f.code;
    const sample = h('input', { class: 'input', value: f.kind === 'pipe' ? 'How long is this sentence really?' : 'Write to ana.silva@example.com about the invoice' });
    const output = h('pre', { class: 'run-output' });
    const error = h('p', { class: 'error-text' });
    const current = (): CortexFunction => ({ id: id.value.trim(), name: name.value.trim(), kind: f.kind, description: description.value.trim(), language: language.value as CortexFunction['language'], code: code.value, enabled: f.enabled });
    const save = async () => {
      try {
        onSaved(await tauriInvoke<CortexFunction[]>('chat_save_function', { function: current() }));
        void reloadModels();
        close();
      } catch (e) {
        error.textContent = String(e);
      }
    };
    const test = async () => {
      output.textContent = 'Running…';
      if (!output.isConnected) sample.closest('.freq-row')?.after(output);
      try {
        output.textContent = await tauriInvoke<string>('chat_test_function', { function: current(), sample: sample.value });
        output.classList.remove('failed');
      } catch (e) {
        output.textContent = String(e);
        output.classList.add('failed');
      }
    };
    const remove = !isNew
      ? h('button', { class: 'btn btn-danger', on: { click: async () => { if (await confirmDialog('Delete function?', `“${f.name}” will stop running.`, 'Delete')) { onSaved(await tauriInvoke<CortexFunction[]>('chat_delete_function', { id: f.id })); void reloadModels(); close(); } } } }, 'Delete')
      : null;
    return h(
      'div',
      { class: 'dialog-small wide' },
      modalHeader(isNew ? (f.kind === 'pipe' ? 'New pipe' : 'New filter') : `Edit ${f.name}`, close, { flush: true }),
      h('div', { class: 'row-inline' }, h('div', { class: 'grow' }, h('label', { class: 'field-label' }, 'Id'), id), h('div', { class: 'grow' }, h('label', { class: 'field-label' }, 'Name'), name)),
      h('label', { class: 'field-label' }, 'Description'),
      description,
      h('label', { class: 'field-label' }, 'Language'),
      language,
      h('label', { class: 'field-label' }, f.kind === 'pipe' ? 'Code — the conversation is in `args.messages`; print the answer' : 'Code — `args.stage` is inlet or outlet, `args.text` the text; print the new text, nothing, or {"block": "why"}'),
      code,
      h('label', { class: 'field-label' }, 'Try it with'),
      h('div', { class: 'freq-row' }, h('div', { class: 'grow' }, sample), h('button', { class: 'btn btn-secondary', on: { click: () => void test() } }, icon('play', 14), 'Test')),
      error,
      h('div', { class: 'dialog-actions' }, remove, h('span', { class: 'spacer' }), h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'), h('button', { class: 'btn btn-primary', on: { click: () => void save() } }, 'Save'))
    );
  });
}

/** Computer use: the assistant may see the screen and use the mouse and keyboard. */
/**
 * The agent workbench (workbench.rs): code from the assistant and agents runs in a container with
 * Python, Node.js, media tools and Blender, seeing only its workspace and the agent's own home.
 */
async function workbenchCard(): Promise<HTMLElement> {
  type Status =
    | { state: 'unavailable'; reason: string }
    | { state: 'missing' }
    | { state: 'building' }
    | { state: 'ready'; gpu: boolean }
    | { state: 'failed'; error: string };
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
    clear(
      status,
      h('p', { class: s.state === 'failed' ? 'error-text' : 'muted' }, line),
      s.state === 'failed' ? h('pre', { class: 'mono workbench-log' }, s.error) : null
    );
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
    h('p', { class: 'muted' }, 'Code from the assistant and your agents runs in a container of its own: Python with its data libraries, Node.js, ffmpeg, ImageMagick and Blender for 3D. It sees only the chat’s workspace and the agent’s own home, never your files; the network is on only for agents you allow. Needs Podman.'),
    row(
      'Run code in the workbench',
      'When it is built. Off, code runs in the basic sandbox with this computer’s programs.',
      switchInput(info.enabled, (v) => {
        info.enabled = v;
        void tauriInvoke('chat_workbench_set_enabled', { enabled: v }).then(() => toast(v ? 'Code runs in the workbench.' : 'Code runs in the basic sandbox.')).catch((e) => toast(String(e), 'error'));
      })
    ),
    status,
    actions
  );
}

async function computerCard(): Promise<HTMLElement> {
  type Settings = { enabled: boolean; blockedApps: string[]; onlyApps: string[] };
  let current = await tauriInvoke<Settings>('chat_computer_settings');
  const save = (next: Partial<Settings>, said: string) => {
    current = { ...current, ...next };
    void tauriInvoke('chat_save_computer_settings', { settings: current }).then(() => toast(said)).catch((e) => toast(String(e), 'error'));
  };
  const lines = (s: string) => s.split('\n').map((l) => l.trim()).filter(Boolean);
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
async function openApiCard(): Promise<HTMLElement> {
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
                h('button', { class: 'link-btn', title: 'Fetch the spec again', on: { click: () => void tauriInvoke<OpenApiServer[]>('chat_refresh_openapi_server', { id: s.id }).then((v) => { render(v); toast('Refreshed.'); }).catch((e) => toast(String(e), 'error')) } }, 'Refresh'),
                h('button', { class: 'link-btn', on: { click: () => void tauriInvoke<OpenApiServer[]>('chat_remove_openapi_server', { id: s.id }).then(render).catch((e) => toast(String(e), 'error')) } }, 'Remove'),
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
    ),
    card(
      row('Notify when a reply is ready', 'A desktop notification when an answer finishes (or fails) while Cortex is in the background. Scheduled tasks always notify.', switchInput(finishNotificationsOn(), (v) => setFinishNotifications(v))),
      row('Play a sound with it', 'A short chime with the notification.', switchInput(finishSoundOn(), (v) => setFinishSound(v))),
      row('Follow answers as they stream', 'Keep the newest text in view while an answer is written; off leaves the page where you put it.', switchInput(followStreamOn(), (v) => setFollowStream(v))),
      row('Suggest follow-ups', 'After each answer, offer three next questions. Each takes one short extra request to the model.', switchInput(followUpsOn(), (v) => setFollowUps(v))),
      (() => {
        // Cortex's own browser for links (Vibe's integrated browser); the choice lives in the app.
        const sw = switchInput(false, (v) => void tauriInvoke('set_browser_prefs', { prefs: { inApp: v } }).catch((e) => toast(String(e), 'error')));
        void tauriInvoke<{ inApp: boolean }>('browser_prefs').then((p) => {
          const box = sw.querySelector('input') ?? (sw as HTMLInputElement);
          (box as HTMLInputElement).checked = p.inApp;
        }).catch(() => undefined);
        return row('Open links in Cortex\u2019s browser', 'Links from answers open in a Cortex window beside the chat; its Page menu goes back, reloads, copies the link or opens it in your usual browser. Off: links ask, then open in your usual browser.', sw);
      })(),
      (() => {
        const stored = clockHour12();
        const current = stored === undefined ? 'auto' : stored ? '12' : '24';
        const pick = h(
          'select',
          { class: 'input', 'aria-label': 'Clock' },
          ...[['auto', 'Like my system'], ['24', '24-hour (14:30)'], ['12', '12-hour (2:30 PM)']].map(([v, label]) => h('option', { value: v, selected: v === current }, label))
        );
        pick.addEventListener('change', () => {
          setClock(pick.value as 'auto' | '12' | '24');
          toast('Saved. Times change as each page is shown again.');
        });
        return row('Clock', 'How times are shown on messages, tasks and the calendar.', pick);
      })(),
      row('Vim keys in the message box', 'Esc switches to Normal mode (h j k l, w b e, dd, cw, yy p, u…); i, a or o go back to typing. Enter sends in both modes.', switchInput(vimEnabled(), (v) => { setVimEnabled(v); toast(v ? 'Vim keys on: Esc for Normal mode.' : 'Vim keys off.'); })),
      row('Autocomplete as you type', 'When you pause while typing, suggest how the sentence goes on; Tab takes it, Esc dismisses it. Each pause is one short request to the model.', switchInput(autocompleteOn(), (v) => setAutocomplete(v))),
      row('New chats are temporary', 'Chats are not saved to your history unless you turn this off for one (the ghost button on the new-chat page). Takes effect from the next new chat.', switchInput(temporaryByDefault(), (v) => setTemporaryByDefault(v)))
    ),
    await chatDefaultsCard(),
    await apiServerCard(),
    await telegramCard(),
    await slackCard(),
    await discordCard(),
    await diagnosticsCard(),
    shortcutsCard()
  );
}

/** Model settings every chat starts from; a chat's own settings win. */
async function chatDefaultsCard(): Promise<HTMLElement> {
  type Params = { temperature: number | null; maxTokens: number | null; topP?: number | null; instructions: string };
  const current = await tauriInvoke<Params>('chat_get_defaults');
  const field = (value: number | null | undefined, attrs: Record<string, string>) => h('input', { class: 'input select-sm', type: 'number', placeholder: 'Model default', value: value ?? '', ...attrs });
  const temperature = field(current.temperature, { min: '0', max: '2', step: '0.1' });
  const maxTokens = field(current.maxTokens, { min: '16', step: '1' });
  const topP = field(current.topP, { min: '0.05', max: '1', step: '0.05' });
  const number = (el: HTMLInputElement) => (el.value.trim() === '' || Number.isNaN(Number(el.value)) ? null : Number(el.value));
  const persist = () =>
    void tauriInvoke<Params>('chat_set_defaults', { params: { temperature: number(temperature), maxTokens: number(maxTokens), topP: number(topP), instructions: '' } })
      .then((saved) => {
        temperature.value = saved.temperature?.toString() ?? '';
        maxTokens.value = saved.maxTokens?.toString() ?? '';
        topP.value = saved.topP?.toString() ?? '';
        toast('Saved. New replies use these unless a chat sets its own.');
      })
      .catch((e) => toast(String(e), 'error'));
  [temperature, maxTokens, topP].forEach((el) => el.addEventListener('change', persist));
  return card(
    h('h3', {}, 'Defaults for every chat'),
    h('p', { class: 'muted' }, 'Empty follows each model. A chat\'s own settings (its menu → Chat settings) win over these.'),
    row('Temperature', '0 is focused and repeatable, 1 balanced, up to 2 inventive.', temperature),
    row('Longest reply (tokens)', '', maxTokens),
    row('Top-p', 'Lower keeps to the likeliest words.', topP)
  );
}

/** What the app sends about itself, and a diagnostics file for bug reports that holds no content. */
async function diagnosticsCard(): Promise<HTMLElement> {
  type Prefs = { errorReports: boolean; updateChecks: boolean };
  const view = await tauriInvoke<{ prefs: Prefs; reportingBuilt: boolean }>('chat_diagnostics_prefs');
  const prefs = view.prefs;
  const persist = () => void tauriInvoke('chat_set_diagnostics_prefs', { prefs }).then(() => toast('Saved. Takes effect at the next start.')).catch((e) => toast(String(e), 'error'));
  const exportBtn = h('button', { class: 'btn btn-secondary' }, icon('download', 14), 'Export diagnostics');
  exportBtn.addEventListener('click', () => void tauriInvoke<boolean>('chat_export_diagnostics').then((saved) => saved && toast('Diagnostics saved.')).catch((e) => toast(String(e), 'error')));
  return card(
    h('h3', {}, 'Error reports and updates'),
    h('p', { class: 'muted' }, 'Cortex sends nothing about you or your use. Conversations, files and keys stay on this computer; requests go only to the providers and services you set up.'),
    row(
      'Error reports',
      view.reportingBuilt ? 'Send crash and error reports (no conversation content) to help fix problems. Off unless you turn it on.' : 'This build has no error reporting in it, so nothing is ever sent.',
      switchInput(prefs.errorReports && view.reportingBuilt, (v) => { prefs.errorReports = v; persist(); })
    ),
    row('Check for updates at start', 'Asks the update server whether a newer version exists. Help → Check for updates works either way.', switchInput(prefs.updateChecks, (v) => { prefs.updateChecks = v; persist(); })),
    row('Diagnostics for a bug report', 'A file with the app version, platform, settings and counts, without anything said, typed or stored.', exportBtn)
  );
}

/** Every shortcut, rebindable: click, press the new keys; Escape cancels, Backspace clears. */
function shortcutsCard(): HTMLElement {
  const rows = h('div', {});
  const render = () =>
    clear(
      rows,
      SHORTCUTS.map((s) => {
        const btn = h('button', { class: 'shortcut-key' }, display(bindingFor(s.action)));
        btn.addEventListener('click', () => {
          btn.classList.add('shortcut-recording');
          btn.textContent = 'Press keys…';
          const onKey = (e: KeyboardEvent) => {
            e.preventDefault();
            e.stopPropagation();
            if (e.key === 'Escape') return done();
            if (e.key === 'Backspace') {
              setBinding(s.action, '');
              return done();
            }
            const combo = comboOf(e);
            // A shortcut needs Ctrl (or Cmd) or Alt, so typing is never taken over.
            if (!combo || !/^(Mod|Alt)\+/.test(combo)) return;
            setBinding(s.action, combo);
            done();
          };
          const done = () => {
            document.removeEventListener('keydown', onKey, true);
            render();
          };
          document.addEventListener('keydown', onKey, true);
        });
        const reset = h('button', { class: 'link-btn', title: 'Back to the default', on: { click: () => { setBinding(s.action, null); render(); } } }, 'Reset');
        return h('div', { class: 'setting-row' }, h('div', {}, h('strong', {}, s.label), h('small', { class: 'muted' }, `Default ${display(s.fallback)}`)), h('div', { class: 'row-actions' }, reset, btn));
      })
    );
  render();
  return card(h('h3', {}, 'Keyboard shortcuts'), h('p', { class: 'muted' }, 'Click a shortcut and press the new keys. Escape cancels; Backspace leaves it unbound.'), rows);
}

interface WidgetSettings {
  enabled: boolean;
  model: string;
  origins: string[];
}

interface ApiServerView {
  settings: { enabled: boolean; port: number; widget: WidgetSettings };
  status: { running: boolean; address: string | null; error: string | null };
  token: string | null;
  widgetToken: string | null;
}

/** The local OpenAI-compatible API: other apps on this computer use the models set up here. */
interface TelegramSettings {
  enabled: boolean;
  agentId: string | null;
  paired: number[];
  pairedNames: string[];
  pairingCode: string | null;
  allowChanges: boolean;
  botName?: string | null;
  hasToken: boolean;
}

/** Talk to your assistant or an agent from Telegram on your phone, through a bot of your own. */
async function telegramCard(): Promise<HTMLElement> {
  const body = h('div', {});
  let s = await tauriInvoke<TelegramSettings>('chat_telegram');
  const update = async (patch: Partial<{ enabled: boolean; agentId: string | null; allowChanges: boolean; openPairing: boolean; unpair: number }>) => {
    try {
      s = await tauriInvoke<TelegramSettings>('chat_update_telegram', {
        enabled: patch.enabled ?? s.enabled,
        agentId: patch.agentId !== undefined ? patch.agentId : s.agentId,
        allowChanges: patch.allowChanges ?? s.allowChanges,
        openPairing: patch.openPairing ?? false,
        unpair: patch.unpair ?? null,
      });
      draw();
    } catch (e) {
      toast(String(e), 'error');
    }
  };
  const draw = () => {
    const token = h('input', { class: 'input mono', type: 'password', placeholder: s.hasToken ? 'Saved in the keychain; paste a new one to change' : '123456789:AA… from @BotFather', autocomplete: 'off' });
    const saveToken = button({ variant: 'secondary', label: s.hasToken ? 'Replace' : 'Connect', onClick: async () => {
      try {
        s = await tauriInvoke<TelegramSettings>('chat_set_telegram_token', { token: token.value });
        toast(s.botName ? `Connected to @${s.botName}.` : 'Token removed.');
        draw();
      } catch (e) {
        toast(String(e), 'error');
      }
    } });
    const agent = h('select', { class: 'input', 'aria-label': 'Who answers' }, h('option', { value: '' }, 'The assistant'), state.agents.map((a) => h('option', { value: a.id, selected: a.id === s.agentId }, a.name)));
    agent.addEventListener('change', () => void update({ agentId: agent.value || null }));
    clear(
      body,
      h('label', { class: 'field-label' }, 'Bot token'),
      h('div', { class: 'row-inline' }, h('div', { class: 'grow' }, token), saveToken),
      h('small', { class: 'muted' }, s.botName ? `Bot: @${s.botName}` : 'In Telegram, message @BotFather, send /newbot, and paste the token it gives you.'),
      s.hasToken
        ? [
            h('div', { class: 'setting-row' }, h('div', {}, h('strong', {}, 'Answer messages'), h('small', { class: 'muted' }, 'While Cortex runs, messages to your bot are answered here and the answer sent back.')), switchInput(s.enabled, (v) => void update({ enabled: v }))),
            h('label', { class: 'field-label' }, 'Who answers'),
            agent,
            h('div', { class: 'setting-row' }, h('div', {}, h('strong', {}, 'Allow changes'), h('small', { class: 'muted' }, 'Off: it reads and searches, but anything that would change something (send, write, delete) is refused, since you cannot approve it from Telegram.')), switchInput(s.allowChanges, (v) => void update({ allowChanges: v }))),
            h('label', { class: 'field-label' }, 'Paired accounts'),
            s.paired.length
              ? h('div', { class: 'hook-list' }, s.paired.map((id, i) => h('div', { class: 'hook-row' }, icon('user', 15), h('span', { class: 'grow' }, s.pairedNames[i] ?? String(id)), h('button', { class: 'link-btn', on: { click: () => void update({ unpair: id }) } }, 'Unpair'))))
              : h('p', { class: 'muted small' }, 'Nobody yet. Only paired accounts are answered.'),
            s.pairingCode
              ? h('p', {}, 'Send this to your bot from your phone: ', h('code', { class: 'pair-code' }, `/pair ${s.pairingCode}`))
              : button({ variant: 'secondary', icon: 'plus', label: 'Pair my phone', onClick: () => void update({ openPairing: true }) }),
          ]
        : null
    );
  };
  draw();
  return card(h('h3', {}, 'Telegram'), h('p', { class: 'muted' }, 'Ask your assistant or one of your agents from your phone. Cortex fetches the messages from Telegram itself, so nothing reaches this computer from outside; each Telegram chat becomes a chat here.'), body);
}

interface SlackSettings {
  enabled: boolean;
  agentId: string | null;
  paired: string[];
  pairingCode: string | null;
  allowChanges: boolean;
  team?: string | null;
  hasTokens: boolean;
}

/** Talk to your assistant or an agent from Slack, through an app of your own in Socket Mode. */
async function slackCard(): Promise<HTMLElement> {
  const body = h('div', {});
  let s = await tauriInvoke<SlackSettings>('chat_slack');
  const update = async (patch: Partial<{ enabled: boolean; agentId: string | null; allowChanges: boolean; openPairing: boolean; unpair: string }>) => {
    try {
      s = await tauriInvoke<SlackSettings>('chat_update_slack', { enabled: patch.enabled ?? s.enabled, agentId: patch.agentId !== undefined ? patch.agentId : s.agentId, allowChanges: patch.allowChanges ?? s.allowChanges, openPairing: patch.openPairing ?? false, unpair: patch.unpair ?? null });
      draw();
    } catch (e) {
      toast(String(e), 'error');
    }
  };
  const draw = () => {
    const appToken = h('input', { class: 'input mono', type: 'password', placeholder: s.hasTokens ? 'Saved; paste to change' : 'xapp-… (App-Level Token, connections:write)', autocomplete: 'off' });
    const botToken = h('input', { class: 'input mono', type: 'password', placeholder: s.hasTokens ? 'Saved; paste to change' : 'xoxb-… (Bot User OAuth Token)', autocomplete: 'off' });
    const connect = button({ variant: 'secondary', label: s.hasTokens ? 'Replace' : 'Connect', onClick: async () => {
      try {
        s = await tauriInvoke<SlackSettings>('chat_set_slack_tokens', { appToken: appToken.value, botToken: botToken.value });
        toast(s.team ? `Connected to ${s.team}.` : 'Tokens removed.');
        draw();
      } catch (e) {
        toast(String(e), 'error');
      }
    } });
    const agent = h('select', { class: 'input', 'aria-label': 'Who answers' }, h('option', { value: '' }, 'The assistant'), state.agents.map((a) => h('option', { value: a.id, selected: a.id === s.agentId }, a.name)));
    agent.addEventListener('change', () => void update({ agentId: agent.value || null }));
    clear(
      body,
      h('label', { class: 'field-label' }, 'App-level token'),
      appToken,
      h('label', { class: 'field-label' }, 'Bot token'),
      h('div', { class: 'row-inline' }, h('div', { class: 'grow' }, botToken), connect),
      h('small', { class: 'muted' }, s.team ? `Workspace: ${s.team}` : 'At api.slack.com/apps: create an app, turn on Socket Mode, add the bot scopes chat:write, im:history and app_mentions:read, subscribe to message.im and app_mention, install it, then paste both tokens.'),
      s.hasTokens
        ? [
            h('div', { class: 'setting-row' }, h('div', {}, h('strong', {}, 'Answer messages'), h('small', { class: 'muted' }, 'Direct messages to the app, and mentions of it in channels (answered in a thread), while Cortex runs.')), switchInput(s.enabled, (v) => void update({ enabled: v }))),
            h('label', { class: 'field-label' }, 'Who answers'),
            agent,
            h('div', { class: 'setting-row' }, h('div', {}, h('strong', {}, 'Allow changes'), h('small', { class: 'muted' }, 'Off: it reads and searches; anything that would change something is refused.')), switchInput(s.allowChanges, (v) => void update({ allowChanges: v }))),
            h('label', { class: 'field-label' }, 'Paired members'),
            s.paired.length ? h('div', { class: 'hook-list' }, s.paired.map((id) => h('div', { class: 'hook-row' }, icon('user', 15), h('span', { class: 'grow mono' }, id), h('button', { class: 'link-btn', on: { click: () => void update({ unpair: id }) } }, 'Unpair')))) : h('p', { class: 'muted small' }, 'Nobody yet. Only paired members are answered.'),
            s.pairingCode ? h('p', {}, 'Send this to the app in Slack: ', h('code', { class: 'pair-code' }, `pair ${s.pairingCode}`)) : button({ variant: 'secondary', icon: 'plus', label: 'Pair my Slack account', onClick: () => void update({ openPairing: true }) }),
          ]
        : null
    );
  };
  draw();
  return card(h('h3', {}, 'Slack'), h('p', { class: 'muted' }, 'Ask your assistant or an agent from Slack. The app connects with Socket Mode, so Slack needs no address for this computer; each Slack conversation becomes a chat here.'), body);
}

interface DiscordSettings {
  enabled: boolean;
  agentId: string | null;
  paired: string[];
  pairingCode: string | null;
  allowChanges: boolean;
  botName?: string | null;
  hasToken: boolean;
}

/** Talk to your assistant or an agent from Discord, through a bot of your own. */
async function discordCard(): Promise<HTMLElement> {
  const body = h('div', {});
  let s = await tauriInvoke<DiscordSettings>('chat_discord');
  const update = async (patch: Partial<{ enabled: boolean; agentId: string | null; allowChanges: boolean; openPairing: boolean; unpair: string }>) => {
    try {
      s = await tauriInvoke<DiscordSettings>('chat_update_discord', { enabled: patch.enabled ?? s.enabled, agentId: patch.agentId !== undefined ? patch.agentId : s.agentId, allowChanges: patch.allowChanges ?? s.allowChanges, openPairing: patch.openPairing ?? false, unpair: patch.unpair ?? null });
      draw();
    } catch (e) {
      toast(String(e), 'error');
    }
  };
  const draw = () => {
    const token = h('input', { class: 'input mono', type: 'password', placeholder: s.hasToken ? 'Saved; paste to change' : 'Bot token (Developer Portal → Bot → Reset Token)', autocomplete: 'off' });
    const connect = button({ variant: 'secondary', label: s.hasToken ? 'Replace' : 'Connect', onClick: async () => {
      try {
        s = await tauriInvoke<DiscordSettings>('chat_set_discord_token', { token: token.value });
        toast(s.botName ? `Connected as ${s.botName}.` : 'Token removed.');
        draw();
      } catch (e) {
        toast(String(e), 'error');
      }
    } });
    const agent = h('select', { class: 'input', 'aria-label': 'Who answers' }, h('option', { value: '' }, 'The assistant'), state.agents.map((a) => h('option', { value: a.id, selected: a.id === s.agentId }, a.name)));
    agent.addEventListener('change', () => void update({ agentId: agent.value || null }));
    clear(
      body,
      h('label', { class: 'field-label' }, 'Bot token'),
      h('div', { class: 'row-inline' }, h('div', { class: 'grow' }, token), connect),
      h('small', { class: 'muted' }, s.botName ? `Bot: ${s.botName}` : 'At discord.com/developers: create an application, add a bot and copy its token; invite it to your server with the bot scope and the Send Messages permission.'),
      s.hasToken
        ? [
            h('div', { class: 'setting-row' }, h('div', {}, h('strong', {}, 'Answer messages'), h('small', { class: 'muted' }, 'Direct messages to the bot, and messages that mention it in a server, while Cortex runs.')), switchInput(s.enabled, (v) => void update({ enabled: v }))),
            h('label', { class: 'field-label' }, 'Who answers'),
            agent,
            h('div', { class: 'setting-row' }, h('div', {}, h('strong', {}, 'Allow changes'), h('small', { class: 'muted' }, 'Off: it reads and searches; anything that would change something is refused.')), switchInput(s.allowChanges, (v) => void update({ allowChanges: v }))),
            h('label', { class: 'field-label' }, 'Paired users'),
            s.paired.length ? h('div', { class: 'hook-list' }, s.paired.map((id) => h('div', { class: 'hook-row' }, icon('user', 15), h('span', { class: 'grow mono' }, id), h('button', { class: 'link-btn', on: { click: () => void update({ unpair: id }) } }, 'Unpair')))) : h('p', { class: 'muted small' }, 'Nobody yet. Only paired users are answered.'),
            s.pairingCode ? h('p', {}, 'Send this to the bot in Discord: ', h('code', { class: 'pair-code' }, `pair ${s.pairingCode}`)) : button({ variant: 'secondary', icon: 'plus', label: 'Pair my Discord account', onClick: () => void update({ openPairing: true }) }),
          ]
        : null
    );
  };
  draw();
  return card(h('h3', {}, 'Discord'), h('p', { class: 'muted' }, 'Ask your assistant or an agent from Discord. The bot connects out to Discord, so this computer needs no public address; each Discord channel or direct message becomes a chat here.'), body);
}

async function apiServerCard(): Promise<HTMLElement> {
  const body = h('div', {});
  const render = (view: ApiServerView) => {
    const port = h('input', { class: 'input small', type: 'number', min: '1024', max: '65535', value: String(view.settings.port), style: { width: '110px' } });
    const save = async (enabled: boolean, regenerateToken = false, widget: WidgetSettings = view.settings.widget, regenerateWidgetToken = false) => {
      try {
        render(await tauriInvoke<ApiServerView>('chat_save_api_server', { settings: { enabled, port: Number(port.value) || 8484, widget }, regenerateToken, regenerateWidgetToken }));
      } catch (e) {
        toast(String(e), 'error');
      }
    };
    port.addEventListener('change', () => void save(view.settings.enabled));
    const copyable = (label: string, value: string) =>
      h(
        'div',
        { class: 'copy-row' },
        h('span', { class: 'muted' }, label),
        h('code', {}, value),
        h('button', { class: 'link-btn', on: { click: () => void navigator.clipboard.writeText(value).then(() => toast('Copied.')) } }, 'Copy')
      );
    const s = view.status;
    const details = view.settings.enabled
      ? [
          s.running && s.address ? copyable('Base URL', s.address) : h('p', { class: 'error-text' }, s.error ?? 'Not running.'),
          view.token ? copyable('API key', view.token) : null,
          s.running && s.address && view.token
            ? h('pre', { class: 'api-example' }, `curl ${s.address}/chat/completions \\\n  -H "Authorization: Bearer ${view.token.slice(0, 9)}…" \\\n  -H "Content-Type: application/json" \\\n  -d '{"model": "${state.model ?? 'vendor/model'}", "messages": [{"role": "user", "content": "Hello"}]}'\n\n# As an MCP server, e.g. for Claude Code:\nclaude mcp add --transport http cortex ${s.address.replace(/\/v1$/, '')}/mcp \\\n  --header "Authorization: Bearer ${view.token.slice(0, 9)}…"`)
            : null,
          h('div', { class: 'card-actions' }, logButton(), h('button', { class: 'btn btn-secondary', on: { click: () => void save(true, true) } }, 'New API key')),
          widgetSection(view, (widget, regenerate) => void save(true, false, widget, regenerate)),
        ]
      : [];
    clear(
      body,
      row('Local API', 'Lets scripts, editors and other apps on this computer use your models and agents (as agent/<id>) through an OpenAI-compatible API (/v1/chat/completions, /v1/models) or an Anthropic-style one (/v1/messages, key in x-api-key), serves an MCP server at /mcp (search your libraries, ask a model or agent, run tasks from an editor or coding agent), and starts scheduled tasks like webhooks (GET /v1/tasks, POST /v1/tasks/{id}/run with an optional input). Only this computer can reach it, and every request needs the API key.', switchInput(view.settings.enabled, (v) => void save(v))),
      row('Port', '', port),
      details
    );
  };
  render(await tauriInvoke<ApiServerView>('chat_api_server'));
  return card(body);
}

/**
 * The website widget (Onyx's): a chat bubble for web pages, answered by one model or agent
 * through the local API, with its own key and a list of pages allowed to use it.
 */
function widgetSection(view: ApiServerView, save: (widget: WidgetSettings, regenerate: boolean) => void): HTMLElement {
  const w = view.settings.widget ?? { enabled: false, model: '', origins: [] };
  const who = h('select', { class: 'input', 'aria-label': 'Who answers' }, h('option', { value: '' }, 'The default model'), state.agents.map((a) => h('option', { value: `agent/${a.id}`, selected: w.model === `agent/${a.id}` }, a.name)));
  if (w.model && !w.model.startsWith('agent/')) who.append(h('option', { value: w.model, selected: true }, w.model));
  const origins = h('textarea', { class: 'input mono', rows: 3, placeholder: 'https://intranet.example.com\nhttp://localhost:3000' }, w.origins.join('\n'));
  const current = (): WidgetSettings => ({ enabled: w.enabled, model: who.value, origins: origins.value.split(/\s+/).filter(Boolean) });
  who.addEventListener('change', () => save(current(), false));
  const base = view.status.address?.replace(/\/v1$/, '') ?? 'http://127.0.0.1:8484';
  const snippet = view.widgetToken ? `<script src="${base}/widget.js" data-key="${view.widgetToken}" data-title="Ask us" defer></script>` : '';
  return h(
    'div',
    { class: 'widget-settings' },
    h('h4', { class: 'subhead' }, 'Website widget'),
    row('Chat bubble for web pages', 'Pages you list get a chat bubble answered by the model or agent you choose. It has its own key, which can do nothing else, and pages cannot change its instructions. Pages opened on this computer reach it: an intranet page, your own site, documentation.', switchInput(w.enabled, (v) => save({ ...current(), enabled: v }, false))),
    w.enabled
      ? [
          h('label', { class: 'field-label' }, 'Who answers'),
          who,
          h('label', { class: 'field-label' }, 'Pages allowed (origins, one per line; null for files opened from disk)'),
          origins,
          h('div', { class: 'card-actions' }, h('button', { class: 'btn btn-secondary', on: { click: () => save(current(), true) } }, 'New widget key'), h('button', { class: 'btn btn-primary', on: { click: () => { save(current(), false); toast('Saved.'); } } }, 'Save')),
          snippet ? h('label', { class: 'field-label' }, 'Add to the page') : null,
          snippet ? h('pre', { class: 'api-example' }, snippet) : null,
          snippet ? h('div', { class: 'card-actions' }, h('button', { class: 'btn btn-secondary', on: { click: () => void navigator.clipboard.writeText(snippet).then(() => toast('Copied.')) } }, 'Copy snippet')) : null,
        ]
      : null
  );
}

/** The local API's latest requests: when, which endpoint and model, the status and how long. */
function logButton(): HTMLElement {
  type Entry = { at: string; endpoint: string; model: string; status: number; ms: number };
  const button = h('button', { class: 'btn btn-ghost' }, icon('list', 14), 'Recent requests');
  button.addEventListener('click', () => {
    void modal((close) => {
      const table = h('div', { class: 'api-log' }, h('p', { class: 'muted' }, 'Loading…'));
      const load = () =>
        void tauriInvoke<Entry[]>('chat_api_server_log').then((entries) =>
          clear(
            table,
            entries.length
              ? h(
                  'table',
                  { class: 'data-table' },
                  h('thead', {}, h('tr', {}, ['Time', 'Endpoint', 'Model', 'Status', 'Took'].map((t) => h('th', {}, t)))),
                  h(
                    'tbody',
                    {},
                    entries.map((e) =>
                      h(
                        'tr',
                        {},
                        h('td', {}, new Date(e.at).toLocaleTimeString(undefined, { hour12: clockHour12() })),
                        h('td', {}, e.endpoint),
                        h('td', { class: 'mono' }, e.model || '—'),
                        h('td', { class: e.status >= 400 ? 'error-text' : '' }, String(e.status)),
                        h('td', {}, e.ms >= 1000 ? `${(e.ms / 1000).toFixed(1)} s` : `${e.ms} ms`)
                      )
                    )
                  )
                )
              : h('p', { class: 'muted' }, 'No requests since Cortex started.')
          )
        );
      load();
      return h('div', { class: 'dialog-small wide' }, modalHeader('Recent requests', close, { flush: true, actions: [h('button', { class: 'btn btn-ghost btn-sm', on: { click: load } }, 'Refresh')] }), table);
    });
  });
  return button;
}

/**
 * The interface language (LibreChat's multilingual UI). Portuguese, Spanish, French and German
 * ship complete; the others are translated by the chosen model once, then kept.
 */
function languageCard(): HTMLElement {
  const box = card(h('h3', {}, 'Language'));
  void (async () => {
    const languages = await tauriInvoke<[string, string][]>('chat_ui_languages').catch(() => [] as [string, string][]);
    const stored = storedLanguage();
    const current = effectiveLanguage(languages.map(([c]) => c));
    const select = h(
      'select',
      { class: 'input', 'aria-label': 'Interface language' },
      h('option', { value: 'auto', selected: stored === 'auto' }, 'Automatic (the system’s language)'),
      h('option', { value: '', selected: stored === '' }, 'English'),
      ...languages.map(([code, name]) => h('option', { value: code, selected: stored === code, class: 'no-translate' }, name))
    );
    select.addEventListener('change', () => {
      setStoredLanguage(select.value);
      location.reload();
    });
    const extra = h('div', {});
    if (current) {
      const cov = await tauriInvoke<{ known: number; total: number }>('chat_ui_coverage', { code: current }).catch(() => null);
      if (cov && cov.known < cov.total) {
        const missing = cov.total - cov.known;
        const status = h('p', { class: 'muted small' }, `${cov.known} of ${cov.total} interface texts are translated.`);
        const go = button({
          variant: 'secondary',
          icon: 'sparkle',
          label: 'Translate the rest with your model',
          title: `${missing} texts, in a few requests to ${state.model ?? 'your default model'}`,
          onClick: async () => {
            if (!state.model) return toast('Choose a model first.', 'error');
            clear(status, h('span', { class: 'dots' }), ` Translating ${missing} texts…`);
            try {
              const after = await tauriInvoke<{ known: number; total: number }>('chat_translate_ui', { model: state.model, code: current });
              toast(`${after.known} of ${after.total} translated. Reloading…`);
              setTimeout(() => location.reload(), 900);
            } catch (e) {
              clear(status, String(e));
            }
          },
        });
        extra.append(status, h('div', { class: 'card-actions start' }, go));
      }
    }
    box.append(
      row('Interface language', 'Menus, buttons and settings. Answers follow the language you write in. Only the app’s own text is translated, never your chats.', select),
      extra
    );
  })();
  return box;
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
  content.append(languageCard());
  content.append(card(h('h3', {}, 'Theme'), h('div', { class: 'theme-options' }, option('light', 'Light', 'sun'), option('dark', 'Dark', 'moon'), option('system', 'System', 'monitor'))));

  // Readability: chat text size, the zoom of the whole interface, contrast and motion.
  const r = readability();
  const sizeSelect = h('select', { class: 'input select-sm' }, TEXT_SIZES.map((t) => h('option', { value: t.id, selected: t.id === r.textSize }, `${t.label} (${t.px}px)`)));
  sizeSelect.addEventListener('change', () => setReadability({ textSize: sizeSelect.value as TextSize }));
  const zoomSelect = h('select', { class: 'input select-sm' }, ZOOMS.map((z) => h('option', { value: String(z), selected: Math.abs(z - r.zoom) < 0.001 }, `${Math.round(z * 100)}%`)));
  zoomSelect.addEventListener('change', () => setReadability({ zoom: Number(zoomSelect.value) }));
  content.append(
    card(
      h('h3', {}, 'Readability'),
      row('Chat text size', 'Messages and answers; the rest of the interface keeps its size.', sizeSelect),
      row('Interface zoom', 'Everything, larger or smaller. Ctrl+= and Ctrl+- step it, Ctrl+0 resets.', zoomSelect),
      row('High contrast', 'Stronger text, borders and focus rings, and underlined links.', switchInput(r.highContrast, (v) => setReadability({ highContrast: v }))),
      row('Reduce motion', 'Turns off animations and smooth scrolling, also when the system does not ask for it.', switchInput(r.reduceMotion, (v) => setReadability({ reduceMotion: v })))
    )
  );

  // Material: solid surfaces, or Liquid Glass (Apple's 2025 design language).
  const materials: [Material, string, string][] = [['glass', 'Liquid Glass', 'Translucent, lightly lit glass'], ['solid', 'Solid', 'Opaque surfaces']];
  const materialRow = h('div', { class: 'theme-options' }, materials.map(([id, label, hint]) =>
    h('button', { class: `theme-option material-${id}${storedMaterial() === id ? ' active' : ''}`, title: hint, on: { click: () => { setMaterial(id); rerender(); } } }, h('span', { class: `material-swatch ${id}` }), label)
  ));
  content.append(card(h('h3', {}, 'Material'), h('p', { class: 'muted' }, 'Liquid Glass gives the message box, menus, dialogs and question cards a light glass finish: what is behind them shows through, blurred, under a thin bright rim. Solid keeps them opaque.'), materialRow));

  // Emoji: Microsoft's Fluent set (as in Teams), downloaded once, or the system's.
  const emojiSelect = h('select', { class: 'input select-sm' }, EMOJI_STYLES.map((s) => h('option', { value: s.id, selected: s.id === emojiStyle() }, s.label)));
  const emojiNote = h('small', { class: 'muted' });
  const showEmoji = (s: EmojiStatus) => {
    const style = emojiSelect.value as EmojiStyle;
    clear(
      emojiNote,
      style === 'system'
        ? 'Emoji look as your system draws them.'
        : s.state === 'ready'
          ? h('span', { class: 'emoji-sample' }, '😀 👍 🎉 ❤️ 🚀 🇵🇹')
          : s.state === 'downloading'
            ? 'Downloading Microsoft’s emoji (about 16 MB, once)…'
            : s.state === 'failed'
              ? h('span', {}, `Could not download them: ${s.error ?? 'unknown error'} `, h('button', { class: 'btn-link', on: { click: () => void setEmojiStyle(style) } }, 'Try again'))
              : ''
    );
  };
  const unwatch = onEmojiStatus(showEmoji);
  emojiSelect.addEventListener('change', () => void setEmojiStyle(emojiSelect.value as EmojiStyle).then(() => emojiStatus(emojiSelect.value as EmojiStyle).then(showEmoji)));
  void emojiStatus(emojiStyle()).then(showEmoji).catch(() => undefined);
  const emojiCard = card(
    h('h3', {}, 'Emoji'),
    row('Emoji style', 'Microsoft’s Fluent emoji, the set Teams uses, in 3D or flat, or the ones your system has. Copying a message still copies the emoji themselves.', emojiSelect),
    emojiNote
  );
  // Stop listening once the settings page is gone.
  new MutationObserver((_, obs) => { if (!emojiCard.isConnected) { unwatch(); obs.disconnect(); } }).observe(document.body, { childList: true, subtree: true });
  content.append(emojiCard);

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
      h('div', { class: 'card-actions start' }, button({ variant: 'subtle', size: 'sm', icon: 'reset', label: 'Reset appearance', onClick: () => { setAccent('theme'); setBackground('default'); setBackdrop('none'); rerender(); } }))
    )
  );

  // Backdrop: a picture behind the conversation.
  const backdrop = storedBackdrop();
  const pick = () => {
    const input = h('input', { type: 'file', accept: 'image/*', class: 'hidden' });
    document.body.append(input);
    input.addEventListener('change', () => {
      const file = input.files?.[0];
      input.remove();
      if (file) void setBackdropImage(file).then(rerender).catch((e) => toast(String(e instanceof Error ? e.message : e), 'error'));
    });
    input.click();
  };
  const backdropTiles = BACKDROPS.map((b) => {
    const preview = backdropPreview(b.id);
    const empty = b.id === 'custom' && !preview;
    return h(
      'button',
      {
        type: 'button',
        class: `bg-tile${backdrop === b.id ? ' active' : ''}`,
        'aria-pressed': String(backdrop === b.id),
        on: { click: () => { if (b.id === 'custom') pick(); else { setBackdrop(b.id); rerender(); } } },
      },
      // The backdrop in miniature in a sketch of the window: beside the sidebar (it fills only the
      // main area, as in the app), under the glass message box and a reply bubble.
      h(
        'span',
        { class: 'bg-thumb backdrop-thumb', style: { background: empty ? 'var(--bg-input)' : 'var(--bg)' } },
        empty
          ? icon('upload', 18)
          : [
              preview ? h('span', { class: 'backdrop-thumb-main', style: { background: preview } }) : null,
              h('span', { class: 'bg-thumb-side', style: { background: 'var(--bg-sidebar)' } }),
              h('span', { class: 'bg-thumb-card backdrop-thumb-glass' }),
              h('span', { class: 'bg-thumb-bubble', style: { background: 'var(--bg-bubble)' } }),
            ],
        b.id === 'custom' && !empty ? h('span', { class: 'backdrop-thumb-change' }, icon('upload', 14)) : null
      ),
      h('span', { class: 'bg-label' }, b.label, backdrop === b.id ? icon('check', 14) : null)
    );
  });
  const veil = h('input', { type: 'range', min: '0', max: '0.95', step: '0.05', value: String(backdropVeil()) });
  veil.addEventListener('input', () => setBackdropVeil(Number(veil.value)));
  content.append(
    card(
      h('h3', {}, 'Backdrop'),
      h('p', { class: 'muted' }, 'A picture behind your conversations. Your own picture stays on this device; the veil keeps the text readable on it.'),
      h('div', { class: 'bg-tiles' }, backdropTiles),
      backdrop === 'custom' ? row('Veil', 'How much the page colour covers your picture.', veil) : null
    )
  );
}


/** The Whisper models downloaded, to choose one for voice input. */
function builtinSpeechSelect(current: string, onChange: (v: string) => void): HTMLSelectElement {
  const sel = h('select', { class: 'input' }, h('option', { value: '' }, 'The first one downloaded'));
  sel.addEventListener('change', () => onChange(sel.value));
  void tauriInvoke<LocalModel[]>('chat_local_models').then((list) => {
    for (const m of list.filter((x) => x.kind === 'speech')) sel.append(h('option', { value: m.id, selected: m.id === current, disabled: !m.installed }, m.installed ? m.name : `${m.name} (not downloaded)`));
  }).catch(() => undefined);
  return sel;
}

const STT_ENGINES: [string, string][] = [
  ['provider', 'Your chat providers'],
  ['builtin', 'Built-in (on this computer)'],
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
      field('Engine', select(STT_ENGINES, s.stt.engine, (v) => { s.stt.engine = v as typeof s.stt.engine; if (v === 'builtin' && !s.stt.model.startsWith('whisper-')) s.stt.model = ''; renderStt(); })),
      s.stt.engine === 'builtin'
        ? field('Model', builtinSpeechSelect(s.stt.model, (v) => (s.stt.model = v)), 'Whisper runs on this computer; download models under Providers → Built-in models.')
        : null,
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
    h('div', { class: 'card-actions' }, test, save),
    card(
      h('h3', {}, 'Voice conversation'),
      row(
        'Pause before your turn ends',
        'How long you can stop speaking before your words are sent. Longer suits thinking aloud; shorter feels snappier. Saved straight away.',
        select(
          [['800', '0.8 seconds'], ['1300', '1.3 seconds (default)'], ['2000', '2 seconds'], ['3000', '3 seconds'], ['4500', '4.5 seconds']],
          String(voicePauseMs()),
          (v) => { setVoicePauseMs(Number(v)); toast('Saved.'); }
        )
      )
    ),
    wakeWordCard(),
    await dictationCard()
  );
}

/** The wake word (Osaurus): "Hey Cortex" from anywhere, while Cortex runs. */
function wakeWordCard(): HTMLElement {
  let prefs = wakePrefs();
  const changed = (next: Partial<typeof prefs>) => {
    prefs = { ...prefs, ...next };
    setWakePrefs(prefs);
    window.dispatchEvent(new Event('cortex:wake-word-changed'));
  };
  const phrase = h('input', { class: 'input', value: prefs.phrase, placeholder: 'Hey Cortex', 'aria-label': 'Wake phrase' });
  phrase.addEventListener('change', () => {
    changed({ phrase: phrase.value.trim() || 'Hey Cortex' });
    toast('Saved.');
  });
  const local = audioSettings?.stt.engine === 'builtin' || (audioSettings?.stt.engine === 'openai' && /localhost|127\.0\.0\.1|\[::1\]/.test(audioSettings.stt.baseUrl ?? ''));
  return card(
    h('h3', {}, 'Wake word'),
    h('p', { class: 'muted' }, 'Say the wake word and a question (“Hey Cortex, what’s on my calendar today?”) and Cortex comes forward and asks it in a new chat; say it alone to start a voice conversation. Works while Cortex is open or in the tray.'),
    row('Listen for the wake word', local ? 'Short phrases go to your local speech-to-text, so nothing leaves this computer.' : 'Every short phrase said near the microphone goes to the speech-to-text above to be checked. Use the Built-in engine (or a Whisper server of your own) to keep it on this computer.', switchInput(prefs.enabled, (v) => {
      changed({ enabled: v });
      toast(v ? `Listening for “${prefs.phrase}”.` : 'Wake word off.');
    })),
    row('Wake phrase', 'A greeting and a name work best; “Hi” or “OK” in place of “Hey” is understood too.', phrase),
    row('Voice orb colour', 'The glowing glass orb of a voice conversation: your accent colour, or the warm ember.', (() => {
      const sel = h('select', { class: 'input', 'aria-label': 'Voice orb colour' }, h('option', { value: 'accent', selected: storedOrbColor() === 'accent' }, 'Accent colour'), h('option', { value: 'ember', selected: storedOrbColor() === 'ember' }, 'Ember'));
      sel.addEventListener('change', () => setOrbColor(sel.value as OrbColor));
      return sel;
    })())
  );
}

/** Dictation anywhere: a global shortcut that types what you say into any app. */
async function dictationCard(): Promise<HTMLElement> {
  type Prefs = { enabled: boolean; shortcut: string; insert: 'type' | 'paste' | 'copy'; stopOnPause: boolean };
  let prefs: Prefs = { enabled: false, shortcut: 'Ctrl+Alt+D', insert: 'type', stopOnPause: false };
  try {
    prefs = await tauriInvoke<Prefs>('dictation_prefs');
  } catch {
    /* defaults */
  }
  const save = async (next: Partial<Prefs>) => {
    const before = prefs;
    prefs = { ...prefs, ...next };
    try {
      await tauriInvoke('set_dictation_prefs', { prefs });
      toast(prefs.enabled ? `Dictation anywhere is on: press ${prefs.shortcut} in any app.` : 'Saved.');
    } catch (e) {
      prefs = before;
      toast(String(e), 'error');
    }
  };
  const shortcut = h('input', { class: 'input', value: prefs.shortcut, placeholder: 'Ctrl+Alt+D', 'aria-label': 'Dictation shortcut' });
  shortcut.addEventListener('change', () => void save({ shortcut: shortcut.value.trim() || 'Ctrl+Alt+D' }));
  const how = h(
    'select',
    { class: 'input', 'aria-label': 'How the text goes in', on: { change: (e: Event) => void save({ insert: (e.target as HTMLSelectElement).value as Prefs['insert'] }) } },
    h('option', { value: 'type' }, 'Type it into the app'),
    h('option', { value: 'paste' }, 'Paste it (uses the clipboard)'),
    h('option', { value: 'copy' }, 'Copy it, I paste myself')
  );
  how.value = prefs.insert;
  return card(
    h('h3', {}, 'Dictation anywhere'),
    h('p', { class: 'muted' }, 'Press a shortcut in any app, speak, and press it again: what you said is transcribed with the speech-to-text above and typed where your cursor is. A small pill shows while Cortex listens; it never takes focus.'),
    row('Dictation anywhere', 'Registers the shortcut below for the whole system while Cortex runs.', switchInput(prefs.enabled, (v) => void save({ enabled: v }))),
    row('Shortcut', 'Like Ctrl+Alt+D or Alt+Shift+Space.', shortcut),
    row('How the text goes in', 'Typing works on X11 and Wayland (xdotool, wtype or ydotool); macOS and Windows paste it. Terminals paste with Ctrl+Shift+V, so typing suits them better.', how),
    row('Finish when I pause', 'Also stop after about two seconds of silence, not only on the second press.', switchInput(prefs.stopOnPause, (v) => void save({ stopOnPause: v })))
  );
}

interface CodingSession {
  path: string;
  source: 'claude-code' | 'codex';
  folder: string;
  title: string;
  messages: number;
  updatedAt: string | null;
  imported: boolean;
}

/**
 * Sessions from other coding agents (Vibe Desktop imports them too): Claude Code and Codex
 * sessions on this computer, picked and brought in as Code chats in their project folders.
 */
function importCodingSessions() {
  void modal((close) => {
    const list = h('div', { class: 'hook-list import-list' }, h('div', { class: 'status-line' }, h('span', { class: 'dots' }), ' Looking for sessions…'));
    const picked = new Set<string>();
    const go = button({
      variant: 'primary',
      label: 'Import',
      onClick: async () => {
        if (!picked.size) return toast('Pick the sessions to import.', 'error');
        try {
          const ids = await tauriInvoke<string[]>('chat_import_coding_sessions', { paths: [...picked] });
          toast(`Imported ${ids.length} session${ids.length === 1 ? '' : 's'} as Code chats.`);
          window.dispatchEvent(new Event('chat:conversations-changed'));
          close();
        } catch (e) {
          toast(String(e), 'error');
        }
      },
    });
    void tauriInvoke<CodingSession[]>('chat_find_coding_sessions')
      .then((found) =>
        clear(
          list,
          found.length
            ? found.map((s) => {
                const box = h('input', { type: 'checkbox', checked: false });
                box.addEventListener('change', () => (box.checked ? picked.add(s.path) : picked.delete(s.path)));
                const when = s.updatedAt ? new Date(s.updatedAt).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' }) : '';
                return h(
                  'label',
                  { class: 'hook-row import-row' },
                  box,
                  h(
                    'div',
                    { class: 'grow share-row' },
                    h('div', {}, h('strong', { class: 'no-translate' }, s.title || 'Untitled')),
                    h('div', { class: 'muted small mono no-translate share-url' }, s.folder || '(no folder)'),
                    h('div', { class: 'muted small' }, [s.source === 'codex' ? 'Codex' : 'Claude Code', `${s.messages} messages`, when, s.imported ? 'imported before: updates it' : ''].filter(Boolean).join(' · '))
                  )
                );
              })
            : h('p', { class: 'muted' }, 'No Claude Code or Codex sessions were found on this computer (~/.claude/projects, ~/.codex/sessions).')
        )
      )
      .catch((e) => clear(list, h('p', { class: 'error-text' }, String(e))));
    return h(
      'div',
      { class: 'dialog-small wide' },
      modalHeader('Import coding sessions', close, { flush: true }),
      h('p', { class: 'muted' }, 'Sessions from Claude Code and Codex become Code chats in their project folder: what was said is kept, and each tool step is a short line. Importing one again updates it.'),
      list,
      h('div', { class: 'dialog-actions' }, button({ variant: 'secondary', label: 'Cancel', onClick: close }), go)
    );
  });
}

/** Encryption at rest: conversations and everything Cortex keeps, sealed with a keychain key. */
function encryptionCard(): HTMLElement {
  type Status = { enabled: boolean; locked: boolean; sealed: number; plain: number };
  const box = card();
  const render = (s: Status | null) => {
    const busy = h('p', { class: 'muted small' });
    const sw = switchInput(!!s?.enabled, async (v) => {
      if (!v && !(await confirmDialog('Decrypt your data?', 'Every conversation, knowledge page and the rest are written back in the clear on this computer, and the key is removed from the keychain.', 'Decrypt'))) return void render(s);
      clear(busy, h('span', { class: 'dots' }), v ? ' Encrypting…' : ' Decrypting…');
      try {
        const n = await tauriInvoke<number>('chat_set_encryption', { on: v });
        toast(v ? `Encrypted ${n} files. New ones are encrypted as they are saved.` : `Decrypted ${n} files.`);
      } catch (e) {
        toast(String(e), 'error');
      }
      render(await tauriInvoke<Status>('chat_encryption_status').catch(() => null));
    });
    clear(
      box,
      h('h3', {}, 'Encrypt data on this computer'),
      h('p', { class: 'muted' }, 'Seal conversations, knowledge, memories, agents, tasks, canvases, libraries, the search index and backups with XChaCha20-Poly1305, under a key kept in the OS keychain. Anyone who copies Cortex’s folder or reads a backup of the disk sees only ciphertext; exports you make are written in the clear. Code sandboxes and skills stay readable for the programs that run them.'),
      row('Encrypt data', s?.locked ? 'On, but the key could not be read from the keychain: unlock it and restart Cortex.' : s?.enabled ? `${s.sealed} files encrypted${s.plain ? `, ${s.plain} not yet` : ''}.` : 'Off: files are plain JSON on this computer.', sw),
      busy
    );
  };
  render(null);
  void tauriInvoke<Status>('chat_encryption_status').then(render).catch(() => undefined);
  return box;
}
