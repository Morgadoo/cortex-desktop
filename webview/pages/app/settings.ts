/** The settings dialog: profile, providers, permissions, general and appearance. */
import { tauriInvoke } from '../../shared/tauri';
import { api, type AppSettings, type Provider } from './api';
import { clear, confirmDialog, h, icon, modal, toast } from './dom';
import { historyButton } from './versions';
import { changed, finishNotificationsOn, finishSoundOn, followStreamOn, followUpsOn, setFinishNotifications, setFinishSound, setFollowStream, setFollowUps, setTemporaryByDefault, state, temporaryByDefault } from './state';
import { button, card, modalHeader, switchInput } from './ui';
import { audioApi, refreshAudioSettings, speakText, type AudioView } from './audio';
import { SHORTCUTS, bindingFor, comboOf, display, setBinding } from './shortcuts';
import { ACCENTS, BACKGROUNDS, TEXT_SIZES, ZOOMS, accentHex, applyAppearance, readability, setAccent, setBackground, setReadability, storedAccent, storedBackground, type TextSize } from './appearance';
import { setVoicePauseMs, voicePauseMs } from './dictation';

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
  ['Storage and backups', 'profile', 'backup restore damaged files recently deleted trash'],
  ['Backup model', 'providers', 'fallback failover failure credit down'],
  ['Providers and API keys', 'providers', 'key openai anthropic mistral xai ollama llama.cpp lm studio model default base url'],
  ['Web search', 'providers', 'searxng brave tavily engine internet'],
  ['Semantic search', 'providers', 'embeddings libraries vector rag rerank reranker reranking'],
  ['Images and voice', 'providers', 'image generation dall-e text to speech voice model'],
  ['Speech to text', 'audio', 'dictation microphone whisper transcription stt'],
  ['Text to speech', 'audio', 'read aloud voice tts elevenlabs'],
  ['Voice conversation', 'audio', 'pause silence turn hands-free call talk'],
  ['Files', 'permissions', 'edit files folders allow'],
  ['Terminal', 'permissions', 'shell commands run bash'],
  ['Your tools', 'permissions', 'custom tools python javascript script'],
  ['Computer use', 'permissions', 'screen mouse keyboard screenshot click'],
  ['API tool servers (OpenAPI)', 'permissions', 'openapi swagger rest tools'],
  ['Privacy filter', 'permissions', 'redact personal data pii'],
  ['MCP servers', 'permissions', 'mcp connectors model context protocol'],
  ['Quick ask shortcut', 'general', 'global hotkey quick ask bar'],
  ['Launch at startup', 'general', 'autostart login'],
  ['Notify when a reply is ready', 'general', 'notification background alert'],
  ['Suggest follow-ups', 'general', 'follow up questions suggestions'],
  ['New chats are temporary', 'general', 'temporary incognito history'],
  ['Keyboard shortcuts', 'general', 'hotkeys keys rebind'],
  ['Local API', 'general', 'openai compatible server port token curl mcp anthropic'],
  ['Defaults for every chat', 'general', 'temperature top-p top_p max tokens reply length model parameters'],
  ['Error reports and updates', 'general', 'privacy telemetry error reports crash sentry updates diagnostics'],
  ['Theme', 'appearance', 'dark light system mode'],
  ['Accent colour', 'appearance', 'color colour highlight'],
  ['Background and palette', 'appearance', 'background colours palette'],
  ['Readability', 'appearance', 'text size font zoom scale contrast motion animation accessibility'],
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
    ),
    dataCard
  );
  void renderData();
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
  content.append(await mediaCard());
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
}

/** Semantic search for libraries: an OpenAI-compatible embedding model, fused with keyword search. */
async function embeddingCard(): Promise<HTMLElement> {
  let current = await tauriInvoke<EmbeddingSettings>('chat_embedding_settings');
  const PRESETS: [string, string, string][] = [
    ['Ollama (this computer)', 'http://localhost:11434/v1', 'nomic-embed-text'],
    ['LM Studio (this computer)', 'http://localhost:1234/v1', 'text-embedding-nomic-embed-text-v1.5'],
    ['OpenAI', 'https://api.openai.com/v1', 'text-embedding-3-small'],
    ['Mistral', 'https://api.mistral.ai/v1', 'mistral-embed'],
  ];
  const baseUrl = h('input', { class: 'input', value: current.baseUrl, placeholder: 'http://localhost:11434/v1' });
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
  let enabled = current.enabled;
  const status = h('small', { class: 'muted' });
  const save = h('button', { class: 'btn btn-primary' }, 'Save');
  const indexNow = h('button', { class: 'btn btn-secondary' }, 'Embed documents now');
  const persist = async () => {
    current = await tauriInvoke<EmbeddingSettings>('chat_save_embedding_settings', {
      settings: { enabled, baseUrl: baseUrl.value, model: model.value, hasKey: current.hasKey, rerankUrl: rerankUrl.value, rerankModel: rerankModel.value, hasRerankKey: current.hasRerankKey },
      key: key.value.trim() || null,
      clearKey: false,
      rerankKey: rerankKey.value.trim() || null,
    });
    key.value = '';
    rerankKey.value = '';
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
    h('p', { class: 'muted' }, 'Library searches always match words. With an embedding model they also match meaning ("cancel the contract" finds the termination clause), and both rankings are merged. Any OpenAI-compatible embeddings server works, including Ollama on this computer.'),
    row('Use semantic search', 'Documents are embedded once and kept with the library.', switchInput(enabled, (v) => { enabled = v; })),
    presets,
    h('label', { class: 'field-label' }, 'Server base URL'),
    baseUrl,
    h('label', { class: 'field-label' }, 'Embedding model'),
    model,
    h('label', { class: 'field-label' }, 'API key'),
    key,
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
    await computerCard(),
    await customToolsCard(),
    mcpCard(),
    await openApiCard()
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

/** Computer use: the assistant may see the screen and use the mouse and keyboard. */
async function computerCard(): Promise<HTMLElement> {
  const current = await tauriInvoke<{ enabled: boolean }>('chat_computer_settings');
  return card(
    h('h3', {}, 'Computer use'),
    row(
      'Allow computer use',
      'Lets the assistant take screenshots and click, type and scroll on this computer. Screenshots are sent to your model. Each action follows the composer’s mode: Ask confirms every click, Auto checks each one against your request. Needs xdotool and ImageMagick on Linux, or cliclick on macOS.',
      switchInput(current.enabled, (v) => void tauriInvoke('chat_save_computer_settings', { settings: { enabled: v } }).then(() => toast(v ? 'Computer use on.' : 'Computer use off.')))
    )
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
      row('Notify when a reply is ready', 'A desktop notification when an answer finishes (or fails) while Lumo is in the background. Scheduled tasks always notify.', switchInput(finishNotificationsOn(), (v) => setFinishNotifications(v))),
      row('Play a sound with it', 'A short chime with the notification.', switchInput(finishSoundOn(), (v) => setFinishSound(v))),
      row('Follow answers as they stream', 'Keep the newest text in view while an answer is written; off leaves the page where you put it.', switchInput(followStreamOn(), (v) => setFollowStream(v))),
      row('Suggest follow-ups', 'After each answer, offer three next questions. Each takes one short extra request to the model.', switchInput(followUpsOn(), (v) => setFollowUps(v))),
      row('New chats are temporary', 'Chats are not saved to your history unless you turn this off for one (the ghost button on the new-chat page). Takes effect from the next new chat.', switchInput(temporaryByDefault(), (v) => setTemporaryByDefault(v)))
    ),
    await chatDefaultsCard(),
    await apiServerCard(),
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
    h('p', { class: 'muted' }, 'Lumo sends nothing about you or your use. Conversations, files and keys stay on this computer; requests go only to the providers and services you set up.'),
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

interface ApiServerView {
  settings: { enabled: boolean; port: number };
  status: { running: boolean; address: string | null; error: string | null };
  token: string | null;
}

/** The local OpenAI-compatible API: other apps on this computer use the models set up here. */
async function apiServerCard(): Promise<HTMLElement> {
  const body = h('div', {});
  const render = (view: ApiServerView) => {
    const port = h('input', { class: 'input small', type: 'number', min: '1024', max: '65535', value: String(view.settings.port), style: { width: '110px' } });
    const save = async (enabled: boolean, regenerateToken = false) => {
      try {
        render(await tauriInvoke<ApiServerView>('chat_save_api_server', { settings: { enabled, port: Number(port.value) || 8484 }, regenerateToken }));
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
            ? h('pre', { class: 'api-example' }, `curl ${s.address}/chat/completions \\\n  -H "Authorization: Bearer ${view.token.slice(0, 9)}…" \\\n  -H "Content-Type: application/json" \\\n  -d '{"model": "${state.model ?? 'vendor/model'}", "messages": [{"role": "user", "content": "Hello"}]}'\n\n# As an MCP server, e.g. for Claude Code:\nclaude mcp add --transport http lumo ${s.address.replace(/\/v1$/, '')}/mcp \\\n  --header "Authorization: Bearer ${view.token.slice(0, 9)}…"`)
            : null,
          h('div', { class: 'card-actions' }, logButton(), h('button', { class: 'btn btn-secondary', on: { click: () => void save(true, true) } }, 'New API key')),
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
                        h('td', {}, new Date(e.at).toLocaleTimeString()),
                        h('td', {}, e.endpoint),
                        h('td', { class: 'mono' }, e.model || '—'),
                        h('td', { class: e.status >= 400 ? 'error-text' : '' }, String(e.status)),
                        h('td', {}, e.ms >= 1000 ? `${(e.ms / 1000).toFixed(1)} s` : `${e.ms} ms`)
                      )
                    )
                  )
                )
              : h('p', { class: 'muted' }, 'No requests since Lumo started.')
          )
        );
      load();
      return h('div', { class: 'dialog-small wide' }, modalHeader('Recent requests', close, { flush: true, actions: [h('button', { class: 'btn btn-ghost btn-sm', on: { click: load } }, 'Refresh')] }), table);
    });
  });
  return button;
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
    )
  );
}
