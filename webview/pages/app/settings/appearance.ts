/** Settings → Appearance: theme, accent, background, readability and the interface language. */
import { invoke } from '../core/call';
import { clear, h } from '../ui/h';
import { icon } from '../ui/icons';
import { toast } from '../ui/overlay';
import { state } from '../core/state';
import { button, card, switchInput } from '../ui/ui';
import { EMOJI_STYLES, emojiStatus, emojiStyle, onEmojiStatus, setEmojiStyle, type EmojiStatus, type EmojiStyle } from '../ui/emoji-fluent';
import { setMaterial, storedMaterial, type Material } from '../theme/appearance';
import {
  ACCENTS,
  BACKDROPS,
  BACKGROUNDS,
  backdropPreview,
  backdropVeil,
  setBackdrop,
  setBackdropImage,
  setBackdropVeil,
  storedBackdrop,
  TEXT_SIZES,
  ZOOMS,
  accentHex,
  readability,
  setAccent,
  setBackground,
  setReadability,
  storedAccent,
  storedBackground,
  type TextSize,
} from '../theme/appearance';
import { effectiveLanguage, setStoredLanguage, storedLanguage } from '../core/i18n';
import { type Theme, setTheme, storedTheme } from '../theme/theme';
import { row } from './row';

/**
 * The interface language (LibreChat's multilingual UI). Portuguese, Spanish, French and German
 * ship complete; the others are translated by the chosen model once, then kept.
 */
function languageCard(): HTMLElement {
  const box = card(h('h3', {}, 'Language'));
  void (async () => {
    const languages = await invoke<[string, string][]>('chat_ui_languages').catch(() => [] as [string, string][]);
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
      const cov = await invoke<{ known: number; total: number }>('chat_ui_coverage', { code: current }).catch(() => null);
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
              const after = await invoke<{ known: number; total: number }>('chat_translate_ui', { model: state.model, code: current });
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
    box.append(row('Interface language', 'Menus, buttons and settings. Answers follow the language you write in. Only the app’s own text is translated, never your chats.', select), extra);
  })();
  return box;
}

export function renderAppearance(content: HTMLElement) {
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
  const sizeSelect = h(
    'select',
    { class: 'input select-sm' },
    TEXT_SIZES.map((t) => h('option', { value: t.id, selected: t.id === r.textSize }, `${t.label} (${t.px}px)`))
  );
  sizeSelect.addEventListener('change', () => setReadability({ textSize: sizeSelect.value as TextSize }));
  const zoomSelect = h(
    'select',
    { class: 'input select-sm' },
    ZOOMS.map((z) => h('option', { value: String(z), selected: Math.abs(z - r.zoom) < 0.001 }, `${Math.round(z * 100)}%`))
  );
  zoomSelect.addEventListener('change', () => setReadability({ zoom: Number(zoomSelect.value) }));
  content.append(
    card(
      h('h3', {}, 'Readability'),
      row('Chat text size', 'Messages and answers; the rest of the interface keeps its size.', sizeSelect),
      row('Interface zoom', 'Everything, larger or smaller. Ctrl+= and Ctrl+- step it, Ctrl+0 resets.', zoomSelect),
      row(
        'High contrast',
        'Stronger text, borders and focus rings, and underlined links.',
        switchInput(r.highContrast, (v) => setReadability({ highContrast: v }))
      ),
      row(
        'Reduce motion',
        'Turns off animations and smooth scrolling, also when the system does not ask for it.',
        switchInput(r.reduceMotion, (v) => setReadability({ reduceMotion: v }))
      )
    )
  );

  // Material: solid surfaces, or Liquid Glass (Apple's 2025 design language).
  const materials: [Material, string, string][] = [
    ['glass', 'Liquid Glass', 'Translucent, lightly lit glass'],
    ['solid', 'Solid', 'Opaque surfaces'],
  ];
  const materialRow = h(
    'div',
    { class: 'theme-options' },
    materials.map(([id, label, hint]) =>
      h(
        'button',
        {
          class: `theme-option material-${id}${storedMaterial() === id ? ' active' : ''}`,
          title: hint,
          on: {
            click: () => {
              setMaterial(id);
              rerender();
            },
          },
        },
        h('span', { class: `material-swatch ${id}` }),
        label
      )
    )
  );
  content.append(
    card(
      h('h3', {}, 'Material'),
      h('p', { class: 'muted' }, 'Liquid Glass gives the message box, menus, dialogs and question cards a light glass finish: what is behind them shows through, blurred, under a thin bright rim. Solid keeps them opaque.'),
      materialRow
    )
  );

  // Emoji: Microsoft's Fluent set (as in Teams), downloaded once, or the system's.
  const emojiSelect = h(
    'select',
    { class: 'input select-sm' },
    EMOJI_STYLES.map((s) => h('option', { value: s.id, selected: s.id === emojiStyle() }, s.label))
  );
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
  void emojiStatus(emojiStyle())
    .then(showEmoji)
    .catch(() => undefined);
  const emojiCard = card(h('h3', {}, 'Emoji'), row('Emoji style', 'Microsoft’s Fluent emoji, the set Teams uses, in 3D or flat, or the ones your system has. Copying a message still copies the emoji themselves.', emojiSelect), emojiNote);
  // Stop listening once the settings page is gone.
  new MutationObserver((_, obs) => {
    if (!emojiCard.isConnected) {
      unwatch();
      obs.disconnect();
    }
  }).observe(document.body, { childList: true, subtree: true });
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
      on: {
        click: () => {
          setAccent('theme');
          rerender();
        },
      },
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
        on: {
          click: () => {
            setAccent(a.id);
            rerender();
          },
        },
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
        on: {
          click: () => {
            setBackground(b.id);
            rerender();
          },
        },
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
      h(
        'div',
        { class: 'card-actions start' },
        button({
          variant: 'subtle',
          size: 'sm',
          icon: 'reset',
          label: 'Reset appearance',
          onClick: () => {
            setAccent('theme');
            setBackground('default');
            setBackdrop('none');
            rerender();
          },
        })
      )
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
      if (file)
        void setBackdropImage(file)
          .then(rerender)
          .catch((e) => toast(String(e instanceof Error ? e.message : e), 'error'));
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
        on: {
          click: () => {
            if (b.id === 'custom') pick();
            else {
              setBackdrop(b.id);
              rerender();
            }
          },
        },
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
