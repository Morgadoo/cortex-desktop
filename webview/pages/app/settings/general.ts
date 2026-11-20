/** Settings → General: chat defaults, diagnostics, keyboard shortcuts and the integrations. */
import { tauriInvoke } from '../../../shared/tauri';
import { api } from '../core/api';
import { clear, h } from '../ui/h';
import { icon } from '../ui/icons';
import { toast } from '../ui/overlay';
import { clockHour12, setClock } from '../core/clock';
import {
  autocompleteOn,
  finishNotificationsOn,
  setAutocomplete,
  finishSoundOn,
  followStreamOn,
  followUpsOn,
  setFinishNotifications,
  setFinishSound,
  setFollowStream,
  setFollowUps,
  setTemporaryByDefault,
  temporaryByDefault,
} from '../core/state';
import { card, switchInput } from '../ui/ui';
import { SHORTCUTS, bindingFor, comboOf, display, setBinding } from '../core/shortcuts';
import { setVimEnabled, vimEnabled } from '../ui/vim';
import { call, reported, shown } from '../core/call';
import { apiServerCard, discordCard, slackCard, telegramCard } from './integrations';
import { row } from './row';

export async function renderGeneral(content: HTMLElement) {
  const settings = await api.appSettings();
  const persist = () =>
    void reported(api.saveAppSettings(settings))
      .then(() => toast('Saved.'))
      .catch(shown);
  const shortcut = h('input', { class: 'input', value: settings.search_shortcut, placeholder: 'Ctrl+L' });
  shortcut.addEventListener('change', () => {
    settings.search_shortcut = shortcut.value.trim();
    persist();
  });
  content.append(
    card(
      row('Quick ask shortcut', 'Opens a search bar from anywhere.', shortcut),
      row(
        'Enable quick ask',
        '',
        switchInput(settings.search_shortcut_enabled, (v) => {
          settings.search_shortcut_enabled = v;
          persist();
        })
      ),
      row(
        'Settings shortcut',
        'Ctrl+, opens settings.',
        switchInput(settings.settings_hotkey_enabled, (v) => {
          settings.settings_hotkey_enabled = v;
          persist();
        })
      ),
      row(
        'Launch at startup',
        'Open the app when you sign in to your computer.',
        switchInput(settings.launch_at_startup, (v) => {
          settings.launch_at_startup = v;
          persist();
        })
      )
    ),
    card(
      row(
        'Notify when a reply is ready',
        'A desktop notification when an answer finishes (or fails) while Cortex is in the background. Scheduled tasks always notify.',
        switchInput(finishNotificationsOn(), (v) => setFinishNotifications(v))
      ),
      row(
        'Play a sound with it',
        'A short chime with the notification.',
        switchInput(finishSoundOn(), (v) => setFinishSound(v))
      ),
      row(
        'Follow answers as they stream',
        'Keep the newest text in view while an answer is written; off leaves the page where you put it.',
        switchInput(followStreamOn(), (v) => setFollowStream(v))
      ),
      row(
        'Suggest follow-ups',
        'After each answer, offer three next questions. Each takes one short extra request to the model.',
        switchInput(followUpsOn(), (v) => setFollowUps(v))
      ),
      (() => {
        // Cortex's own browser for links (Vibe's integrated browser); the choice lives in the app.
        const sw = switchInput(false, (v) => void call('set_browser_prefs', { prefs: { inApp: v } }).catch(shown));
        void tauriInvoke<{ inApp: boolean }>('browser_prefs')
          .then((p) => {
            const box = sw.querySelector('input') ?? (sw as HTMLInputElement);
            (box as HTMLInputElement).checked = p.inApp;
          })
          .catch(() => undefined);
        return row(
          'Open links in Cortex\u2019s browser',
          'Links from answers open in a Cortex window beside the chat; its Page menu goes back, reloads, copies the link or opens it in your usual browser. Off: links ask, then open in your usual browser.',
          sw
        );
      })(),
      (() => {
        const stored = clockHour12();
        const current = stored === undefined ? 'auto' : stored ? '12' : '24';
        const pick = h(
          'select',
          { class: 'input', 'aria-label': 'Clock' },
          ...[
            ['auto', 'Like my system'],
            ['24', '24-hour (14:30)'],
            ['12', '12-hour (2:30 PM)'],
          ].map(([v, label]) => h('option', { value: v, selected: v === current }, label))
        );
        pick.addEventListener('change', () => {
          setClock(pick.value as 'auto' | '12' | '24');
          toast('Saved. Times change as each page is shown again.');
        });
        return row('Clock', 'How times are shown on messages, tasks and the calendar.', pick);
      })(),
      row(
        'Vim keys in the message box',
        'Esc switches to Normal mode (h j k l, w b e, dd, cw, yy p, u…); i, a or o go back to typing. Enter sends in both modes.',
        switchInput(vimEnabled(), (v) => {
          setVimEnabled(v);
          toast(v ? 'Vim keys on: Esc for Normal mode.' : 'Vim keys off.');
        })
      ),
      row(
        'Autocomplete as you type',
        'When you pause while typing, suggest how the sentence goes on; Tab takes it, Esc dismisses it. Each pause is one short request to the model.',
        switchInput(autocompleteOn(), (v) => setAutocomplete(v))
      ),
      row(
        'New chats are temporary',
        'Chats are not saved to your history unless you turn this off for one (the ghost button on the new-chat page). Takes effect from the next new chat.',
        switchInput(temporaryByDefault(), (v) => setTemporaryByDefault(v))
      )
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
    void call<Params>('chat_set_defaults', { params: { temperature: number(temperature), maxTokens: number(maxTokens), topP: number(topP), instructions: '' } })
      .then((saved) => {
        temperature.value = saved.temperature?.toString() ?? '';
        maxTokens.value = saved.maxTokens?.toString() ?? '';
        topP.value = saved.topP?.toString() ?? '';
        toast('Saved. New replies use these unless a chat sets its own.');
      })
      .catch(shown);
  [temperature, maxTokens, topP].forEach((el) => el.addEventListener('change', persist));
  return card(
    h('h3', {}, 'Defaults for every chat'),
    h('p', { class: 'muted' }, "Empty follows each model. A chat's own settings (its menu → Chat settings) win over these."),
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
  const persist = () =>
    void call('chat_set_diagnostics_prefs', { prefs })
      .then(() => toast('Saved. Takes effect at the next start.'))
      .catch(shown);
  const exportBtn = h('button', { class: 'btn btn-secondary' }, icon('download', 14), 'Export diagnostics');
  exportBtn.addEventListener(
    'click',
    () =>
      void call<boolean>('chat_export_diagnostics')
        .then((saved) => saved && toast('Diagnostics saved.'))
        .catch(shown)
  );
  return card(
    h('h3', {}, 'Error reports and updates'),
    h('p', { class: 'muted' }, 'Cortex sends nothing about you or your use. Conversations, files and keys stay on this computer; requests go only to the providers and services you set up.'),
    row(
      'Error reports',
      view.reportingBuilt ? 'Send crash and error reports (no conversation content) to help fix problems. Off unless you turn it on.' : 'This build has no error reporting in it, so nothing is ever sent.',
      switchInput(prefs.errorReports && view.reportingBuilt, (v) => {
        prefs.errorReports = v;
        persist();
      })
    ),
    row(
      'Check for updates at start',
      'Asks the update server whether a newer version exists. Help → Check for updates works either way.',
      switchInput(prefs.updateChecks, (v) => {
        prefs.updateChecks = v;
        persist();
      })
    ),
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
        const reset = h(
          'button',
          {
            class: 'link-btn',
            title: 'Back to the default',
            on: {
              click: () => {
                setBinding(s.action, null);
                render();
              },
            },
          },
          'Reset'
        );
        return h('div', { class: 'setting-row' }, h('div', {}, h('strong', {}, s.label), h('small', { class: 'muted' }, `Default ${display(s.fallback)}`)), h('div', { class: 'row-actions' }, reset, btn));
      })
    );
  render();
  return card(h('h3', {}, 'Keyboard shortcuts'), h('p', { class: 'muted' }, 'Click a shortcut and press the new keys. Escape cancels; Backspace leaves it unbound.'), rows);
}
