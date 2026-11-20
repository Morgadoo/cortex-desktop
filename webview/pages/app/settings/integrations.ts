/** Settings → General's integrations: the local API server and its widget, and the Telegram, Slack and Discord bots. */
import { clear, h } from '../ui/h';
import { icon } from '../ui/icons';
import { modal, toast } from '../ui/overlay';
import { clockHour12 } from '../core/clock';
import { state } from '../core/state';
import { button, card, modalHeader, switchInput } from '../ui/ui';
import { row } from './row';
import { call, invoke } from '../core/call';

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
export async function telegramCard(): Promise<HTMLElement> {
  const body = h('div', {});
  let s = await invoke<TelegramSettings>('chat_telegram');
  const update = async (patch: Partial<{ enabled: boolean; agentId: string | null; allowChanges: boolean; openPairing: boolean; unpair: number }>) => {
    try {
      s = await call<TelegramSettings>('chat_save_telegram', {
        enabled: patch.enabled ?? s.enabled,
        agentId: patch.agentId !== undefined ? patch.agentId : s.agentId,
        allowChanges: patch.allowChanges ?? s.allowChanges,
        openPairing: patch.openPairing ?? false,
        unpair: patch.unpair ?? null,
      });
      draw();
    } catch {
      // Already shown.
    }
  };
  const draw = () => {
    const token = h('input', { class: 'input mono', type: 'password', placeholder: s.hasToken ? 'Saved in the keychain; paste a new one to change' : '123456789:AA… from @BotFather', autocomplete: 'off' });
    const saveToken = button({
      variant: 'secondary',
      label: s.hasToken ? 'Replace' : 'Connect',
      onClick: async () => {
        try {
          s = await call<TelegramSettings>('chat_set_telegram_token', { token: token.value });
          toast(s.botName ? `Connected to @${s.botName}.` : 'Token removed.');
          draw();
        } catch {
          // Already shown.
        }
      },
    });
    const agent = h(
      'select',
      { class: 'input', 'aria-label': 'Who answers' },
      h('option', { value: '' }, 'The assistant'),
      state.agents.map((a) => h('option', { value: a.id, selected: a.id === s.agentId }, a.name))
    );
    agent.addEventListener('change', () => void update({ agentId: agent.value || null }));
    clear(
      body,
      h('label', { class: 'field-label' }, 'Bot token'),
      h('div', { class: 'row-inline' }, h('div', { class: 'grow' }, token), saveToken),
      h('small', { class: 'muted' }, s.botName ? `Bot: @${s.botName}` : 'In Telegram, message @BotFather, send /newbot, and paste the token it gives you.'),
      s.hasToken
        ? [
            h(
              'div',
              { class: 'setting-row' },
              h('div', {}, h('strong', {}, 'Answer messages'), h('small', { class: 'muted' }, 'While Cortex runs, messages to your bot are answered here and the answer sent back.')),
              switchInput(s.enabled, (v) => void update({ enabled: v }))
            ),
            h('label', { class: 'field-label' }, 'Who answers'),
            agent,
            h(
              'div',
              { class: 'setting-row' },
              h(
                'div',
                {},
                h('strong', {}, 'Allow changes'),
                h('small', { class: 'muted' }, 'Off: it reads and searches, but anything that would change something (send, write, delete) is refused, since you cannot approve it from Telegram.')
              ),
              switchInput(s.allowChanges, (v) => void update({ allowChanges: v }))
            ),
            h('label', { class: 'field-label' }, 'Paired accounts'),
            s.paired.length
              ? h(
                  'div',
                  { class: 'hook-list' },
                  s.paired.map((id, i) =>
                    h('div', { class: 'hook-row' }, icon('user', 15), h('span', { class: 'grow' }, s.pairedNames[i] ?? String(id)), h('button', { class: 'link-btn', on: { click: () => void update({ unpair: id }) } }, 'Unpair'))
                  )
                )
              : h('p', { class: 'muted small' }, 'Nobody yet. Only paired accounts are answered.'),
            s.pairingCode
              ? h('p', {}, 'Send this to your bot from your phone: ', h('code', { class: 'pair-code' }, `/pair ${s.pairingCode}`))
              : button({ variant: 'secondary', icon: 'plus', label: 'Pair my phone', onClick: () => void update({ openPairing: true }) }),
          ]
        : null
    );
  };
  draw();
  return card(
    h('h3', {}, 'Telegram'),
    h('p', { class: 'muted' }, 'Ask your assistant or one of your agents from your phone. Cortex fetches the messages from Telegram itself, so nothing reaches this computer from outside; each Telegram chat becomes a chat here.'),
    body
  );
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
export async function slackCard(): Promise<HTMLElement> {
  const body = h('div', {});
  let s = await invoke<SlackSettings>('chat_slack');
  const update = async (patch: Partial<{ enabled: boolean; agentId: string | null; allowChanges: boolean; openPairing: boolean; unpair: string }>) => {
    try {
      s = await call<SlackSettings>('chat_save_slack', {
        enabled: patch.enabled ?? s.enabled,
        agentId: patch.agentId !== undefined ? patch.agentId : s.agentId,
        allowChanges: patch.allowChanges ?? s.allowChanges,
        openPairing: patch.openPairing ?? false,
        unpair: patch.unpair ?? null,
      });
      draw();
    } catch {
      // Already shown.
    }
  };
  const draw = () => {
    const appToken = h('input', { class: 'input mono', type: 'password', placeholder: s.hasTokens ? 'Saved; paste to change' : 'xapp-… (App-Level Token, connections:write)', autocomplete: 'off' });
    const botToken = h('input', { class: 'input mono', type: 'password', placeholder: s.hasTokens ? 'Saved; paste to change' : 'xoxb-… (Bot User OAuth Token)', autocomplete: 'off' });
    const connect = button({
      variant: 'secondary',
      label: s.hasTokens ? 'Replace' : 'Connect',
      onClick: async () => {
        try {
          s = await call<SlackSettings>('chat_set_slack_tokens', { appToken: appToken.value, botToken: botToken.value });
          toast(s.team ? `Connected to ${s.team}.` : 'Tokens removed.');
          draw();
        } catch {
          // Already shown.
        }
      },
    });
    const agent = h(
      'select',
      { class: 'input', 'aria-label': 'Who answers' },
      h('option', { value: '' }, 'The assistant'),
      state.agents.map((a) => h('option', { value: a.id, selected: a.id === s.agentId }, a.name))
    );
    agent.addEventListener('change', () => void update({ agentId: agent.value || null }));
    clear(
      body,
      h('label', { class: 'field-label' }, 'App-level token'),
      appToken,
      h('label', { class: 'field-label' }, 'Bot token'),
      h('div', { class: 'row-inline' }, h('div', { class: 'grow' }, botToken), connect),
      h(
        'small',
        { class: 'muted' },
        s.team
          ? `Workspace: ${s.team}`
          : 'At api.slack.com/apps: create an app, turn on Socket Mode, add the bot scopes chat:write, im:history and app_mentions:read, subscribe to message.im and app_mention, install it, then paste both tokens.'
      ),
      s.hasTokens
        ? [
            h(
              'div',
              { class: 'setting-row' },
              h('div', {}, h('strong', {}, 'Answer messages'), h('small', { class: 'muted' }, 'Direct messages to the app, and mentions of it in channels (answered in a thread), while Cortex runs.')),
              switchInput(s.enabled, (v) => void update({ enabled: v }))
            ),
            h('label', { class: 'field-label' }, 'Who answers'),
            agent,
            h(
              'div',
              { class: 'setting-row' },
              h('div', {}, h('strong', {}, 'Allow changes'), h('small', { class: 'muted' }, 'Off: it reads and searches; anything that would change something is refused.')),
              switchInput(s.allowChanges, (v) => void update({ allowChanges: v }))
            ),
            h('label', { class: 'field-label' }, 'Paired members'),
            s.paired.length
              ? h(
                  'div',
                  { class: 'hook-list' },
                  s.paired.map((id) => h('div', { class: 'hook-row' }, icon('user', 15), h('span', { class: 'grow mono' }, id), h('button', { class: 'link-btn', on: { click: () => void update({ unpair: id }) } }, 'Unpair')))
                )
              : h('p', { class: 'muted small' }, 'Nobody yet. Only paired members are answered.'),
            s.pairingCode
              ? h('p', {}, 'Send this to the app in Slack: ', h('code', { class: 'pair-code' }, `pair ${s.pairingCode}`))
              : button({ variant: 'secondary', icon: 'plus', label: 'Pair my Slack account', onClick: () => void update({ openPairing: true }) }),
          ]
        : null
    );
  };
  draw();
  return card(
    h('h3', {}, 'Slack'),
    h('p', { class: 'muted' }, 'Ask your assistant or an agent from Slack. The app connects with Socket Mode, so Slack needs no address for this computer; each Slack conversation becomes a chat here.'),
    body
  );
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
export async function discordCard(): Promise<HTMLElement> {
  const body = h('div', {});
  let s = await invoke<DiscordSettings>('chat_discord');
  const update = async (patch: Partial<{ enabled: boolean; agentId: string | null; allowChanges: boolean; openPairing: boolean; unpair: string }>) => {
    try {
      s = await call<DiscordSettings>('chat_save_discord', {
        enabled: patch.enabled ?? s.enabled,
        agentId: patch.agentId !== undefined ? patch.agentId : s.agentId,
        allowChanges: patch.allowChanges ?? s.allowChanges,
        openPairing: patch.openPairing ?? false,
        unpair: patch.unpair ?? null,
      });
      draw();
    } catch {
      // Already shown.
    }
  };
  const draw = () => {
    const token = h('input', { class: 'input mono', type: 'password', placeholder: s.hasToken ? 'Saved; paste to change' : 'Bot token (Developer Portal → Bot → Reset Token)', autocomplete: 'off' });
    const connect = button({
      variant: 'secondary',
      label: s.hasToken ? 'Replace' : 'Connect',
      onClick: async () => {
        try {
          s = await call<DiscordSettings>('chat_set_discord_token', { token: token.value });
          toast(s.botName ? `Connected as ${s.botName}.` : 'Token removed.');
          draw();
        } catch {
          // Already shown.
        }
      },
    });
    const agent = h(
      'select',
      { class: 'input', 'aria-label': 'Who answers' },
      h('option', { value: '' }, 'The assistant'),
      state.agents.map((a) => h('option', { value: a.id, selected: a.id === s.agentId }, a.name))
    );
    agent.addEventListener('change', () => void update({ agentId: agent.value || null }));
    clear(
      body,
      h('label', { class: 'field-label' }, 'Bot token'),
      h('div', { class: 'row-inline' }, h('div', { class: 'grow' }, token), connect),
      h('small', { class: 'muted' }, s.botName ? `Bot: ${s.botName}` : 'At discord.com/developers: create an application, add a bot and copy its token; invite it to your server with the bot scope and the Send Messages permission.'),
      s.hasToken
        ? [
            h(
              'div',
              { class: 'setting-row' },
              h('div', {}, h('strong', {}, 'Answer messages'), h('small', { class: 'muted' }, 'Direct messages to the bot, and messages that mention it in a server, while Cortex runs.')),
              switchInput(s.enabled, (v) => void update({ enabled: v }))
            ),
            h('label', { class: 'field-label' }, 'Who answers'),
            agent,
            h(
              'div',
              { class: 'setting-row' },
              h('div', {}, h('strong', {}, 'Allow changes'), h('small', { class: 'muted' }, 'Off: it reads and searches; anything that would change something is refused.')),
              switchInput(s.allowChanges, (v) => void update({ allowChanges: v }))
            ),
            h('label', { class: 'field-label' }, 'Paired users'),
            s.paired.length
              ? h(
                  'div',
                  { class: 'hook-list' },
                  s.paired.map((id) => h('div', { class: 'hook-row' }, icon('user', 15), h('span', { class: 'grow mono' }, id), h('button', { class: 'link-btn', on: { click: () => void update({ unpair: id }) } }, 'Unpair')))
                )
              : h('p', { class: 'muted small' }, 'Nobody yet. Only paired users are answered.'),
            s.pairingCode
              ? h('p', {}, 'Send this to the bot in Discord: ', h('code', { class: 'pair-code' }, `pair ${s.pairingCode}`))
              : button({ variant: 'secondary', icon: 'plus', label: 'Pair my Discord account', onClick: () => void update({ openPairing: true }) }),
          ]
        : null
    );
  };
  draw();
  return card(
    h('h3', {}, 'Discord'),
    h('p', { class: 'muted' }, 'Ask your assistant or an agent from Discord. The bot connects out to Discord, so this computer needs no public address; each Discord channel or direct message becomes a chat here.'),
    body
  );
}

export async function apiServerCard(): Promise<HTMLElement> {
  const body = h('div', {});
  const render = (view: ApiServerView) => {
    const port = h('input', { class: 'input small', type: 'number', min: '1024', max: '65535', value: String(view.settings.port), style: { width: '110px' } });
    const save = async (enabled: boolean, regenerateToken = false, widget: WidgetSettings = view.settings.widget, regenerateWidgetToken = false) => {
      try {
        render(await call<ApiServerView>('chat_save_api_server', { settings: { enabled, port: Number(port.value) || 8484, widget }, regenerateToken, regenerateWidgetToken }));
      } catch {
        // Already shown.
      }
    };
    port.addEventListener('change', () => void save(view.settings.enabled));
    const copyable = (label: string, value: string) =>
      h('div', { class: 'copy-row' }, h('span', { class: 'muted' }, label), h('code', {}, value), h('button', { class: 'link-btn', on: { click: () => void navigator.clipboard.writeText(value).then(() => toast('Copied.')) } }, 'Copy'));
    const s = view.status;
    const details = view.settings.enabled
      ? [
          s.running && s.address ? copyable('Base URL', s.address) : h('p', { class: 'error-text' }, s.error ?? 'Not running.'),
          view.token ? copyable('API key', view.token) : null,
          s.running && s.address && view.token
            ? h(
                'pre',
                { class: 'api-example' },
                `curl ${s.address}/chat/completions \\\n  -H "Authorization: Bearer ${view.token.slice(0, 9)}…" \\\n  -H "Content-Type: application/json" \\\n  -d '{"model": "${state.model ?? 'vendor/model'}", "messages": [{"role": "user", "content": "Hello"}]}'\n\n# As an MCP server, e.g. for Claude Code:\nclaude mcp add --transport http cortex ${s.address.replace(/\/v1$/, '')}/mcp \\\n  --header "Authorization: Bearer ${view.token.slice(0, 9)}…"`
              )
            : null,
          h('div', { class: 'card-actions' }, logButton(), h('button', { class: 'btn btn-secondary', on: { click: () => void save(true, true) } }, 'New API key')),
          widgetSection(view, (widget, regenerate) => void save(true, false, widget, regenerate)),
        ]
      : [];
    clear(
      body,
      row(
        'Local API',
        'Lets scripts, editors and other apps on this computer use your models and agents (as agent/<id>) through an OpenAI-compatible API (/v1/chat/completions, /v1/models) or an Anthropic-style one (/v1/messages, key in x-api-key), serves an MCP server at /mcp (search your libraries, ask a model or agent, run tasks from an editor or coding agent), and starts scheduled tasks like webhooks (GET /v1/tasks, POST /v1/tasks/{id}/run with an optional input). Only this computer can reach it, and every request needs the API key.',
        switchInput(view.settings.enabled, (v) => void save(v))
      ),
      row('Port', '', port),
      details
    );
  };
  render(await invoke<ApiServerView>('chat_api_server'));
  return card(body);
}

/**
 * The website widget (Onyx's): a chat bubble for web pages, answered by one model or agent
 * through the local API, with its own key and a list of pages allowed to use it.
 */
function widgetSection(view: ApiServerView, save: (widget: WidgetSettings, regenerate: boolean) => void): HTMLElement {
  const w = view.settings.widget ?? { enabled: false, model: '', origins: [] };
  const who = h(
    'select',
    { class: 'input', 'aria-label': 'Who answers' },
    h('option', { value: '' }, 'The default model'),
    state.agents.map((a) => h('option', { value: `agent/${a.id}`, selected: w.model === `agent/${a.id}` }, a.name))
  );
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
    row(
      'Chat bubble for web pages',
      'Pages you list get a chat bubble answered by the model or agent you choose. It has its own key, which can do nothing else, and pages cannot change its instructions. Pages opened on this computer reach it: an intranet page, your own site, documentation.',
      switchInput(w.enabled, (v) => save({ ...current(), enabled: v }, false))
    ),
    w.enabled
      ? [
          h('label', { class: 'field-label' }, 'Who answers'),
          who,
          h('label', { class: 'field-label' }, 'Pages allowed (origins, one per line; null for files opened from disk)'),
          origins,
          h(
            'div',
            { class: 'card-actions' },
            h('button', { class: 'btn btn-secondary', on: { click: () => save(current(), true) } }, 'New widget key'),
            h(
              'button',
              {
                class: 'btn btn-primary',
                on: {
                  click: () => {
                    save(current(), false);
                    toast('Saved.');
                  },
                },
              },
              'Save'
            )
          ),
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
        void invoke<Entry[]>('chat_api_server_log').then((entries) =>
          clear(
            table,
            entries.length
              ? h(
                  'table',
                  { class: 'data-table' },
                  h(
                    'thead',
                    {},
                    h(
                      'tr',
                      {},
                      ['Time', 'Endpoint', 'Model', 'Status', 'Took'].map((t) => h('th', {}, t))
                    )
                  ),
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
