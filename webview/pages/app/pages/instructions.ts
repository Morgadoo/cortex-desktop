/** The Instructions dialog: how the assistant should answer, in the user's words, with a preview. */
import { api, type Instructions } from '../core/api';
import { clear, h } from '../ui/h';
import { icon } from '../ui/icons';
import { markdown } from '../ui/markdown';
import { modal, toast } from '../ui/overlay';
import { state } from '../core/state';
import { modalHeader } from '../ui/ui';

const TONES: { id: string; label: string; description: string }[] = [
  { id: '', label: 'Default', description: 'Natural style, no personalization.' },
  { id: 'formal', label: 'Polite', description: 'Professional and courteous.' },
  { id: 'casual', label: 'Casual', description: 'Relaxed, like a friendly colleague.' },
  { id: 'empathetic', label: 'Empathetic', description: 'Warm; acknowledges feelings first.' },
  { id: 'objective', label: 'Objective', description: 'Neutral, factual, weighs trade-offs.' },
  { id: 'concise', label: 'Concise', description: 'Leads with the answer, skips filler.' },
  { id: 'witty', label: 'Witty', description: 'Light and good-humoured where it fits.' },
  { id: 'motivational', label: 'Motivational', description: 'Encouraging and energising.' },
  { id: 'reflective', label: 'Reflective', description: 'Surfaces assumptions and second-order effects.' },
];

const STYLE_CHIPS = ['Be concise', 'Use bullet points', 'Explain step by step', 'Cite your sources', 'Ask before assuming', 'Use metric units', 'Answer in Portuguese'];

const PREVIEW_PROMPT = 'Recommend a book for me to read.';

export function openInstructions(): Promise<void> {
  const draft: Instructions = { ...state.instructions };
  return modal((close) => {
    const toneSelect = h(
      'select',
      { class: 'input' },
      TONES.map((t) => h('option', { value: t.id, selected: t.id === draft.tone }, t.label))
    );
    const toneHint = h('p', { class: 'hint' }, TONES.find((t) => t.id === draft.tone)?.description ?? '');
    toneSelect.addEventListener('change', () => {
      draft.tone = toneSelect.value;
      toneHint.textContent = TONES.find((t) => t.id === draft.tone)?.description ?? '';
    });
    const name = h('input', { class: 'input', value: draft.name, placeholder: 'How should the assistant call you?' });
    name.addEventListener('input', () => (draft.name = name.value));
    const custom = h('textarea', { class: 'input textarea', rows: 6, placeholder: 'Be concise, use a friendly tone, etc.' });
    custom.value = draft.custom;
    custom.addEventListener('input', () => (draft.custom = custom.value));
    const chips = h(
      'div',
      { class: 'chip-row' },
      STYLE_CHIPS.map((chip) =>
        h(
          'button',
          {
            class: 'chip add',
            on: {
              click: () => {
                custom.value = custom.value.trim() ? `${custom.value.trim()}\n${chip}.` : `${chip}.`;
                draft.custom = custom.value;
              },
            },
          },
          icon('plus', 13),
          chip
        )
      )
    );

    const previewBody = h('div', { class: 'preview-body' });
    const runPreview = async () => {
      if (!state.model) {
        clear(previewBody, h('p', { class: 'muted' }, 'Choose a model to preview replies.'));
        return;
      }
      clear(previewBody, h('div', { class: 'bubble preview-q' }, PREVIEW_PROMPT), h('div', { class: 'status-line' }, h('span', { class: 'dots' }), 'Writing'));
      try {
        const reply = await api.previewInstructions(draft, state.model, PREVIEW_PROMPT);
        const answer = h('div', { class: 'markdown' });
        answer.append(markdown(reply));
        clear(previewBody, h('div', { class: 'bubble preview-q' }, PREVIEW_PROMPT), answer);
      } catch (e) {
        clear(previewBody, h('div', { class: 'bubble preview-q' }, PREVIEW_PROMPT), h('p', { class: 'error-text' }, String(e)));
      }
    };
    const refresh = h('button', { class: 'icon-btn round', title: 'Preview again', on: { click: () => void runPreview() } }, icon('retry', 16));
    clear(previewBody, h('div', { class: 'bubble preview-q' }, PREVIEW_PROMPT), h('p', { class: 'muted' }, 'Press ↻ to see a reply with these instructions.'));

    const save = async () => {
      await api.saveInstructions(draft);
      state.instructions = { ...draft };
      toast('Instructions saved.');
      close();
    };

    return h(
      'div',
      { class: 'instructions' },
      modalHeader('Instructions', close),
      h(
        'div',
        { class: 'instructions-grid' },
        h(
          'div',
          { class: 'instructions-form' },
          h('label', { class: 'field-label' }, 'Your name'),
          name,
          h('label', { class: 'field-label' }, 'Tone'),
          toneSelect,
          toneHint,
          h('hr'),
          h('label', { class: 'field-label' }, 'Additional instructions'),
          custom,
          chips,
          h('div', { class: 'grow' }),
          h('button', { class: 'btn btn-primary', on: { click: () => void save() } }, 'Save')
        ),
        h('div', { class: 'preview' }, h('div', { class: 'preview-tools' }, refresh), previewBody)
      )
    );
  }, true);
}
