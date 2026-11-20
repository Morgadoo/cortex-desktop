/** A conversation's own model settings, and what fills its context window. */
import { api, type ChatParams } from '../core/api';
import { h } from '../ui/h';
import { modal, toast } from '../ui/overlay';
import { modalHeader } from '../ui/ui';
import { reported } from '../core/call';

const SLICE_COLORS = ['var(--accent)', '#4fa3d1', '#5fb98a', '#d9a441', '#c96f9b', '#8c8fd6', '#9aa0a6', '#d97a5b'];

/** The context view: what the next request carries, and how full the model's window is. */
export async function showContext(id: string) {
  const view = await reported(api.contextBreakdown(id)).catch(() => null);
  if (!view) return;
  const used = view.measured || view.slices.reduce((n, s) => n + s.tokens, 0);
  const pct = Math.min(100, Math.round((used / view.window) * 100));
  const k = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(n >= 10_000 ? 0 : 1)}k` : String(n));
  void modal((close) =>
    h(
      'div',
      { class: 'dialog-small wide context-view' },
      modalHeader('Context', close),
      h('p', { class: 'muted' }, `${used.toLocaleString()} of ${view.window.toLocaleString()} tokens in use (${pct}%). Older messages are summarized automatically at 60%.`),
      h(
        'div',
        { class: 'context-bar' },
        view.slices.map((s, i) => h('span', { title: s.label, style: { width: `${(s.tokens / view.window) * 100}%`, background: SLICE_COLORS[i % SLICE_COLORS.length]! } }))
      ),
      h(
        'div',
        { class: 'context-legend' },
        view.slices.map((s, i) =>
          h(
            'div',
            { class: 'context-row' },
            h('span', { class: 'context-dot', style: { background: SLICE_COLORS[i % SLICE_COLORS.length]! } }),
            h('span', { class: 'grow' }, s.label),
            h('span', { class: 'muted' }, `${k(s.tokens)} · ${((s.tokens / view.window) * 100).toFixed(1)}%`)
          )
        )
      ),
      view.summarizedMessages ? h('p', { class: 'muted small' }, `${view.summarizedMessages} earlier messages are carried as a summary.`) : null,
      view.clearedResults ? h('p', { class: 'muted small' }, `${view.clearedResults} older tool results are shortened; the assistant runs a tool again if it needs one.`) : null,
      h('p', { class: 'muted small' }, 'The total is what the provider measured; the split is an estimate.')
    )
  );
}

/** One conversation's model settings: temperature, reply length and its own instructions. */
export async function openChatSettings(id: string) {
  let current: ChatParams;
  try {
    current = (await api.conversation(id)).params;
  } catch {
    current = { temperature: null, maxTokens: null, instructions: '' };
  }
  await modal((close) => {
    const temperature = h('input', { class: 'input', type: 'number', min: '0', max: '2', step: '0.1', placeholder: 'Model default', value: current.temperature ?? '' });
    const maxTokens = h('input', { class: 'input', type: 'number', min: '16', step: '1', placeholder: 'Model default', value: current.maxTokens ?? '' });
    const topP = h('input', { class: 'input', type: 'number', min: '0.05', max: '1', step: '0.05', placeholder: 'Model default', value: current.topP ?? '' });
    const instructions = h('textarea', { class: 'input', rows: 5, placeholder: 'For example: answer in Portuguese; keep code in TypeScript.' }, current.instructions);
    const number = (el: HTMLInputElement) => (el.value.trim() === '' || Number.isNaN(Number(el.value)) ? null : Number(el.value));
    const save = async () => {
      try {
        await reported(api.setParams(id, { temperature: number(temperature), maxTokens: number(maxTokens), topP: number(topP), instructions: instructions.value }));
        close();
        toast('Saved. The next reply uses these settings.');
      } catch {
        // Already shown.
      }
    };
    return h(
      'div',
      { class: 'dialog-small wide' },
      h('h2', {}, 'Chat settings'),
      h('p', { class: 'muted' }, 'For this conversation only. Empty fields follow your defaults (Settings → General), then the model.'),
      h('label', { class: 'field' }, h('span', { class: 'field-label' }, 'Temperature'), temperature, h('span', { class: 'field-hint' }, '0 is focused and repeatable, 1 is balanced, up to 2 is inventive.')),
      h('label', { class: 'field' }, h('span', { class: 'field-label' }, 'Longest reply (tokens)'), maxTokens),
      h('label', { class: 'field' }, h('span', { class: 'field-label' }, 'Top-p'), topP, h('span', { class: 'field-hint' }, 'Only the likeliest words adding up to this share are considered; lower is more predictable.')),
      h('label', { class: 'field' }, h('span', { class: 'field-label' }, 'Instructions for this chat'), instructions, h('span', { class: 'field-hint' }, 'Added to your standing instructions.')),
      h('div', { class: 'dialog-actions' }, h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'), h('button', { class: 'btn btn-primary', on: { click: () => void save() } }, 'Save'))
    );
  });
}
