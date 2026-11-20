/** The settings dialog: its sections and search, and reloading the models. Each section is a file here. */
import { api } from '../core/api';
import { clear, h } from '../ui/h';
import { icon } from '../ui/icons';
import { modal } from '../ui/overlay';
import { changed, state } from '../core/state';
import { modalHeader } from '../ui/ui';
import { renderAudio } from './audio';
import { renderAppearance } from './appearance';
import { renderGeneral } from './general';
import { renderPermissions } from './permissions';
import { renderProviders } from './providers';
import { renderProfile } from './profile';

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
              {
                class: 'nav-item settings-hit',
                on: {
                  click: () => {
                    current = section;
                    focusLabel = label;
                    void renderBody();
                  },
                },
              },
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
        list.append(
          h(
            'button',
            {
              class: `nav-item${s.id === current ? ' active' : ''}`,
              on: {
                click: () => {
                  current = s.id;
                  renderNav();
                  void renderBody();
                },
              },
            },
            icon(s.icon, 16),
            s.label
          )
        );
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
  return (
    name
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((w) => w[0]!.toUpperCase())
      .join('') || 'U'
  );
}
