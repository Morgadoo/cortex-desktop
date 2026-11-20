import { tauriInvoke } from '../../shared/tauri';
import { h, icon, popover, toast } from './ui/dom';
import { state } from './core/state';

const IMPROVE_PRESETS: [string, string][] = [
  ['Improve writing', 'Improve the writing: clearer and tighter, same meaning.'],
  ['Fix spelling and grammar', 'Fix spelling, grammar and punctuation only; change nothing else.'],
  ['Make shorter', 'Make it about half as long, keeping every key point.'],
  ['Make longer', 'Expand it with more detail and explanation, without inventing facts.'],
  ['More formal', 'Rewrite it in a more formal, professional tone.'],
  ['Simpler words', 'Rewrite it in plain, simple words.'],
  ['As a bullet list', 'Turn it into a concise Markdown bullet list.'],
];

/**
 * "Improve with AI" for a Markdown editor (Open WebUI's AI Enhance): rewrites the selection, or
 * everything when nothing is selected, in place with the chat's model. Undo puts it back.
 */
export function improveButton(area: HTMLTextAreaElement, changed: () => void): HTMLElement {
  let before: string | null = null;
  const undo = h('button', { class: 'btn btn-ghost btn-sm', hidden: true }, 'Undo');
  undo.addEventListener('click', () => {
    if (before === null) return;
    area.value = before;
    before = null;
    undo.hidden = true;
    changed();
  });
  const trigger = h('button', { class: 'btn btn-secondary btn-sm', title: 'Rewrite the selection, or the whole page, with AI' }, icon('sparkle', 14), 'Improve');
  const run = async (instruction: string) => {
    const start = area.selectionStart;
    const end = area.selectionEnd;
    const whole = start === end;
    const selected = whole ? area.value : area.value.slice(start, end);
    // Only the words change: the blank lines and spaces around the selection stay as they were.
    const lead = selected.match(/^\s*/)![0];
    const trail = selected.slice(lead.length).match(/\s*$/)![0];
    const text = selected.slice(lead.length, selected.length - trail.length);
    if (!text.trim()) return toast('Write something first.');
    if (!state.model) return toast('Choose a model first.', 'error');
    trigger.setAttribute('disabled', '');
    trigger.lastChild!.textContent = 'Improving…';
    try {
      const context = whole ? null : `${area.value.slice(Math.max(0, start - 3000), start)}[…]${area.value.slice(end, end + 3000)}`;
      const out = await tauriInvoke<string>('chat_improve_text', { model: state.model, text, instruction, context });
      before = area.value;
      if (whole) area.value = `${lead}${out}${trail}`;
      else area.setRangeText(`${lead}${out}${trail}`, start, end, 'select');
      undo.hidden = false;
      changed();
      area.focus();
    } catch (e) {
      toast(String(e), 'error');
    } finally {
      trigger.removeAttribute('disabled');
      trigger.lastChild!.textContent = 'Improve';
    }
  };
  trigger.addEventListener('click', () => {
    const custom = h('input', { class: 'input', placeholder: 'Or say how, then Enter' });
    let closeMenu = () => {};
    custom.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && custom.value.trim()) {
        closeMenu();
        void run(custom.value.trim());
      }
    });
    const scope = area.selectionStart === area.selectionEnd ? 'the whole page' : 'the selection';
    const list = h(
      'div',
      { class: 'menu improve-menu' },
      h('div', { class: 'menu-heading' }, `Rewrite ${scope}`),
      ...IMPROVE_PRESETS.map(([label, instruction]) =>
        h(
          'button',
          {
            class: 'menu-item',
            on: {
              click: () => {
                closeMenu();
                void run(instruction);
              },
            },
          },
          h('span', {}, label)
        )
      ),
      custom
    );
    closeMenu = popover(trigger, list, 'above');
    setTimeout(() => custom.focus());
  });
  return h('span', { class: 'improve-tools' }, trigger, undo);
}
