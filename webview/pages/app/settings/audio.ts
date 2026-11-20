/** Settings → Audio: speech-to-text and text-to-speech engines, voices, the wake word and dictation. */
import { tauriInvoke } from '../../../shared/tauri';
import { clear, h } from '../ui/h';
import { toast } from '../ui/overlay';
import { button, card, switchInput } from '../ui/ui';
import { audioApi, audioSettings, refreshAudioSettings, speakText, type AudioView } from '../voice/audio';
import { setVoicePauseMs, voicePauseMs } from '../voice/dictation';
import { setWakePrefs, wakePrefs } from '../voice/wakeword';
import { setOrbColor, storedOrbColor, type OrbColor } from '../theme/orbgl';
import { row } from './row';
import type { LocalModel } from './models';
import { call, reported } from '../core/call';

/** The Whisper models downloaded, to choose one for voice input. */
function builtinSpeechSelect(current: string, onChange: (v: string) => void): HTMLSelectElement {
  const sel = h('select', { class: 'input' }, h('option', { value: '' }, 'The first one downloaded'));
  sel.addEventListener('change', () => onChange(sel.value));
  void tauriInvoke<LocalModel[]>('chat_local_models')
    .then((list) => {
      for (const m of list.filter((x) => x.kind === 'speech')) sel.append(h('option', { value: m.id, selected: m.id === current, disabled: !m.installed }, m.installed ? m.name : `${m.name} (not downloaded)`));
    })
    .catch(() => undefined);
  return sel;
}

interface HexModel {
  id: string;
  name: string;
  installed: boolean;
  verified: boolean;
}
interface HexStatus {
  running: boolean;
  version: string | null;
  models: HexModel[];
  problem: string | null;
}

/** Hex's model picker, with whether Hex is running; filled in once Hex answers. */
function hexSpeechField(current: string, onChange: (v: string) => void): HTMLElement {
  const sel = h('select', { class: 'input', 'aria-label': 'Hex model' }, h('option', { value: '' }, 'The best one downloaded in Hex'));
  sel.addEventListener('change', () => onChange(sel.value));
  const status = h('small', { class: 'field-hint' }, 'Looking for Hex…');
  void tauriInvoke<HexStatus>('chat_hex_status')
    .then((hex) => {
      for (const m of hex.models) {
        const ready = m.installed && m.verified;
        sel.append(h('option', { value: m.id, selected: m.id === current, disabled: !ready }, ready ? m.name || m.id : `${m.name || m.id} (not downloaded)`));
      }
      status.textContent = hex.running
        ? 'Hex is running. When it is closed, the built-in Whisper takes over.'
        : hex.problem
          ? `${hex.problem} Until it is, the built-in Whisper is used.`
          : 'Hex is not running. Until it is, the built-in Whisper is used.';
    })
    .catch(() => (status.textContent = 'Hex could not be asked.'));
  return h('div', { class: 'field' }, h('label', { class: 'field-label' }, 'Model'), sel, status);
}

const STT_ENGINES: [string, string][] = [
  ['provider', 'Your chat providers'],
  ['builtin', 'Built-in (on this computer)'],
  ['hex', 'Hex (on this computer)'],
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
export async function renderAudio(content: HTMLElement) {
  let view: AudioView = await audioApi.get();
  const s = structuredClone(view.settings);
  const select = (options: [string, string][], value: string, onChange: (v: string) => void) => {
    const el = h(
      'select',
      { class: 'input' },
      options.map(([v, l]) => h('option', { value: v, selected: v === value }, l))
    );
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
    const clearBtn = has
      ? button({
          variant: 'subtle',
          size: 'sm',
          label: 'Remove key',
          onClick: () => {
            onClear();
            input.placeholder = 'Removed on save';
          },
        })
      : null;
    return h('div', { class: 'key-row' }, input, clearBtn);
  };
  const field = (label: string, control: HTMLElement, hint?: string) => h('div', { class: 'field' }, h('label', { class: 'field-label' }, label), control, hint ? h('small', { class: 'field-hint' }, hint) : null);

  const sttBody = h('div', { class: 'audio-fields' });
  const ttsBody = h('div', { class: 'audio-fields' });
  const renderStt = () =>
    clear(
      sttBody,
      field(
        'Engine',
        select(STT_ENGINES, s.stt.engine, (v) => {
          s.stt.engine = v as typeof s.stt.engine;
          if ((v === 'builtin' && !s.stt.model.startsWith('whisper-')) || v === 'hex') s.stt.model = '';
          renderStt();
        })
      ),
      s.stt.engine === 'hex' ? h('p', { class: 'muted small' }, 'Hex is a fast speech-to-text app (Parakeet and Whisper models) that runs on this computer. Install it from anoma.ly/hex and download a model there.') : null,
      s.stt.engine === 'hex' ? hexSpeechField(s.stt.model, (v) => (s.stt.model = v)) : null,
      s.stt.engine === 'builtin'
        ? field(
            'Model',
            builtinSpeechSelect(s.stt.model, (v) => (s.stt.model = v)),
            'Whisper runs on this computer; download models under Providers → Built-in models.'
          )
        : null,
      s.stt.engine === 'provider'
        ? h('p', { class: 'muted small' }, view.providerSpeechModel ? `Uses ${view.providerSpeechModel}. Change it under Providers & models.` : 'None of your providers offers speech-to-text (OpenAI or Groq). Choose another engine.')
        : null,
      s.stt.engine === 'openai'
        ? field(
            'API base URL',
            text(s.stt.baseUrl, 'https://api.openai.com/v1  ·  https://api.groq.com/openai/v1  ·  http://localhost:8000/v1', (v) => (s.stt.baseUrl = v)),
            'Any server with /audio/transcriptions: OpenAI, Groq, Speaches, LocalAI, faster-whisper-server.'
          )
        : null,
      s.stt.engine === 'openai' || s.stt.engine === 'elevenlabs'
        ? field(
            'API key',
            keyField(
              s.stt.hasKey,
              (v) => (keys.sttKey = v),
              () => (keys.clearSttKey = true)
            )
          )
        : null,
      s.stt.engine === 'openai' || s.stt.engine === 'elevenlabs'
        ? field(
            'Model',
            text(s.stt.model, s.stt.engine === 'elevenlabs' ? 'scribe_v1' : 'whisper-1  ·  gpt-4o-mini-transcribe  ·  whisper-large-v3-turbo', (v) => (s.stt.model = v))
          )
        : null,
      s.stt.engine !== 'off' && s.stt.engine !== 'provider'
        ? field(
            'Language',
            text(s.stt.language, 'Detect automatically, or a code like en, pt', (v) => (s.stt.language = v))
          )
        : null,
      s.stt.engine !== 'off'
        ? h(
            'div',
            { class: 'setting-row' },
            h('div', {}, h('strong', {}, 'Send after dictation'), h('small', { class: 'muted' }, 'Send the message as soon as the transcript arrives.')),
            switchInput(s.stt.autoSend, (v) => (s.stt.autoSend = v))
          )
        : null
    );
  const voiceList = h('datalist', { id: 'tts-voices' });
  const loadVoices = async () => {
    const voices =
      s.tts.engine === 'browser' && 'speechSynthesis' in window
        ? speechSynthesis.getVoices().map((v) => [v.name, `${v.name} (${v.lang})`] as [string, string])
        : await audioApi.voices(s.tts.engine, s.tts.baseUrl).catch(() => [] as [string, string][]);
    clear(
      voiceList,
      voices.map(([id, name]) => h('option', { value: id }, name))
    );
  };
  const renderTts = () => {
    const needsServer = ['openai', 'xai', 'elevenlabs'].includes(s.tts.engine);
    const voiceInput = h('input', {
      class: 'input',
      value: s.tts.voice,
      placeholder: s.tts.engine === 'elevenlabs' ? 'Voice id (pick from the list once the key is saved)' : s.tts.engine === 'xai' ? 'eve' : 'alloy',
      list: 'tts-voices',
      autocomplete: 'off',
    });
    voiceInput.addEventListener('input', () => (s.tts.voice = voiceInput.value));
    const speed = h('input', { type: 'range', min: '0.5', max: '2', step: '0.05', value: String(s.tts.speed), class: 'speed-range' });
    const speedLabel = h('span', { class: 'muted' }, `${s.tts.speed.toFixed(2)}×`);
    speed.addEventListener('input', () => {
      s.tts.speed = Number(speed.value);
      speedLabel.textContent = `${s.tts.speed.toFixed(2)}×`;
    });
    clear(
      ttsBody,
      field(
        'Engine',
        select(TTS_ENGINES, s.tts.engine, (v) => {
          s.tts.engine = v as typeof s.tts.engine;
          renderTts();
          void loadVoices();
        })
      ),
      s.tts.engine === 'provider'
        ? h('p', { class: 'muted small' }, view.providerVoiceModel ? `Uses ${view.providerVoiceModel}. Change it under Providers & models.` : 'None of your providers offers text-to-speech (OpenAI or xAI). Choose another engine.')
        : null,
      s.tts.engine === 'browser' ? h('p', { class: 'muted small' }, 'Uses the voices installed on this computer. Nothing leaves the device.') : null,
      needsServer
        ? field(
            'API base URL',
            text(s.tts.baseUrl, s.tts.engine === 'elevenlabs' ? 'https://api.elevenlabs.io' : s.tts.engine === 'xai' ? 'https://api.x.ai' : 'https://api.openai.com/v1  ·  http://localhost:8880/v1', (v) => (s.tts.baseUrl = v)),
            s.tts.engine === 'openai' ? 'Any server with /audio/speech: OpenAI, Kokoro-FastAPI, LocalAI, Speaches.' : 'Leave empty for the service default.'
          )
        : null,
      needsServer
        ? field(
            'API key',
            keyField(
              s.tts.hasKey,
              (v) => (keys.ttsKey = v),
              () => (keys.clearTtsKey = true)
            )
          )
        : null,
      needsServer
        ? field(
            'Model',
            text(s.tts.model, s.tts.engine === 'elevenlabs' ? 'eleven_multilingual_v2' : s.tts.engine === 'xai' ? 'grok-tts' : 'tts-1  ·  gpt-4o-mini-tts  ·  kokoro', (v) => (s.tts.model = v))
          )
        : null,
      s.tts.engine !== 'off' && s.tts.engine !== 'provider' ? field('Voice', h('div', {}, voiceInput, voiceList)) : null,
      s.tts.engine !== 'off' && s.tts.engine !== 'xai' ? field('Speed', h('div', { class: 'speed-row' }, speed, speedLabel)) : null,
      s.tts.engine !== 'off'
        ? h(
            'div',
            { class: 'setting-row' },
            h('div', {}, h('strong', {}, 'Read answers aloud automatically'), h('small', { class: 'muted' }, 'Each answer is read when it finishes.')),
            switchInput(s.tts.autoPlay, (v) => (s.tts.autoPlay = v))
          )
        : null
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
  const save = button({
    variant: 'primary',
    label: 'Save',
    onClick: async () => {
      try {
        await reported(persist());
        toast('Audio settings saved.');
      } catch {
        // Already shown.
      }
    },
  });
  const test = button({
    variant: 'secondary',
    icon: 'speaker',
    label: 'Save and test voice',
    onClick: async () => {
      try {
        await reported(persist());
        await reported(speakText('Hello! This is how answers will sound.'));
      } catch {
        // Already shown.
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
          [
            ['800', '0.8 seconds'],
            ['1300', '1.3 seconds (default)'],
            ['2000', '2 seconds'],
            ['3000', '3 seconds'],
            ['4500', '4.5 seconds'],
          ],
          String(voicePauseMs()),
          (v) => {
            setVoicePauseMs(Number(v));
            toast('Saved.');
          }
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
  const local = audioSettings?.stt.engine === 'builtin' || audioSettings?.stt.engine === 'hex' || (audioSettings?.stt.engine === 'openai' && /localhost|127\.0\.0\.1|\[::1\]/.test(audioSettings.stt.baseUrl ?? ''));
  return card(
    h('h3', {}, 'Wake word'),
    h(
      'p',
      { class: 'muted' },
      'Say the wake word and a question (“Hey Cortex, what’s on my calendar today?”) and Cortex comes forward and asks it in a new chat; say it alone to start a voice conversation. Works while Cortex is open or in the tray.'
    ),
    row(
      'Listen for the wake word',
      local
        ? 'Short phrases go to your local speech-to-text, so nothing leaves this computer.'
        : 'Every short phrase said near the microphone goes to the speech-to-text above to be checked. Use the Built-in engine (or a Whisper server of your own) to keep it on this computer.',
      switchInput(prefs.enabled, (v) => {
        changed({ enabled: v });
        toast(v ? `Listening for “${prefs.phrase}”.` : 'Wake word off.');
      })
    ),
    row('Wake phrase', 'A greeting and a name work best; “Hi” or “OK” in place of “Hey” is understood too.', phrase),
    row(
      'Voice orb colour',
      'The glowing glass orb of a voice conversation: your accent colour, or the warm ember.',
      (() => {
        const sel = h(
          'select',
          { class: 'input', 'aria-label': 'Voice orb colour' },
          h('option', { value: 'accent', selected: storedOrbColor() === 'accent' }, 'Accent colour'),
          h('option', { value: 'ember', selected: storedOrbColor() === 'ember' }, 'Ember')
        );
        sel.addEventListener('change', () => setOrbColor(sel.value as OrbColor));
        return sel;
      })()
    )
  );
}

/** Dictation anywhere: a global shortcut that types what you say into any app. */
async function dictationCard(): Promise<HTMLElement> {
  type Prefs = { enabled: boolean; shortcut: string; insert: 'type' | 'paste' | 'copy'; stopOnPause: boolean; holdToTalk: boolean };
  let prefs: Prefs = { enabled: false, shortcut: 'Ctrl+Alt+D', insert: 'type', stopOnPause: false, holdToTalk: false };
  try {
    prefs = await tauriInvoke<Prefs>('dictation_prefs');
  } catch {
    /* defaults */
  }
  const save = async (next: Partial<Prefs>) => {
    const before = prefs;
    prefs = { ...prefs, ...next };
    try {
      await call('set_dictation_prefs', { prefs });
      toast(prefs.enabled ? `Dictation anywhere is on: press ${prefs.shortcut} in any app.` : 'Saved.');
    } catch {
      prefs = before;
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
    h(
      'p',
      { class: 'muted' },
      'Press a shortcut in any app, speak, and press it again: what you said is transcribed with the speech-to-text above and typed where your cursor is. A small pill shows while Cortex listens; it never takes focus.'
    ),
    row(
      'Dictation anywhere',
      'Registers the shortcut below for the whole system while Cortex runs.',
      switchInput(prefs.enabled, (v) => void save({ enabled: v }))
    ),
    row('Shortcut', 'Like Ctrl+Alt+D or Alt+Shift+Space.', shortcut),
    row(
      'How the text goes in',
      'Typing works on X11 and Wayland (xdotool, wtype or ydotool); macOS and Windows paste it. GNOME on Wayland pastes by default, as it lets no app type into another. Terminals paste with Ctrl+Shift+V, so typing suits them better.',
      how
    ),
    row(
      'Hold to talk',
      'Listen while the shortcut is held and insert when you let go. A quick tap still starts and stops as usual.',
      switchInput(prefs.holdToTalk, (v) => void save({ holdToTalk: v }))
    ),
    row(
      'Finish when I pause',
      'Also stop after about two seconds of silence, not only on the second press.',
      switchInput(prefs.stopOnPause, (v) => void save({ stopOnPause: v }))
    )
  );
}
