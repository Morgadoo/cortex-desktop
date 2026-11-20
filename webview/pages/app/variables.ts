/**
 * Prompt variables, as in Open WebUI and LibreChat: `{{current_date}}` and friends fill in on
 * send; any other `{{name}}` asks for a value first, and `{{name|a|b}}` offers a choice. Works for
 * typed prompts and workflows, so a saved prompt becomes a small form.
 */
import { api } from './core/api';
import { h, modal, clockHour12 } from './dom';
import { invalid, substitute, variablesIn, type Variable } from './promptvars';

/** Filled without asking. */
export async function builtIns(): Promise<Record<string, string>> {
  const now = new Date();
  let name = '';
  try {
    name = (await api.instructions()).name;
  } catch {
    // Without a saved name the variable stays empty.
  }
  return {
    current_date: now.toLocaleDateString(undefined, { year: 'numeric', month: 'long', day: 'numeric' }),
    current_time: now.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', hour12: clockHour12() }),
    current_datetime: now.toLocaleString(undefined, { hour12: clockHour12() }),
    current_weekday: now.toLocaleDateString(undefined, { weekday: 'long' }),
    user_name: name,
  };
}

export { substitute, variablesIn, type Variable } from './promptvars';

/** Fills the variables in `text`, asking for the ones it cannot know; null if the user cancels. */
export async function fillVariables(text: string): Promise<string | null> {
  if (!text.includes('{{')) return text;
  const known = await builtIns();
  const wanted = variablesIn(text, known);
  if (!wanted.length) return substitute(text, known);
  const values = await askValues(wanted);
  return values ? substitute(text, { ...known, ...values }) : null;
}

function askValues(wanted: Variable[]): Promise<Record<string, string> | null> {
  let result: Record<string, string> | null = null;
  return modal((close) => {
    // Each input is the control for its type, and a value is checked before anything is sent.
    const inputs = wanted.map((v) => {
      const input = v.options.length
        ? h(
            'select',
            { class: 'input' },
            v.options.map((o) => h('option', { value: o }, o))
          )
        : v.type === 'long'
          ? h('textarea', { class: 'input textarea short', rows: 3, placeholder: label(v.name) })
          : h('input', { class: 'input', type: v.type === 'text' ? 'text' : v.type === 'number' ? 'number' : v.type, step: v.type === 'number' ? 'any' : undefined, placeholder: label(v.name) });
      const error = h('small', { class: 'error-text' });
      return { v, input: input as HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement, error };
    });
    const submit = () => {
      let ok = true;
      for (const { v, input, error } of inputs) {
        const why = invalid(v, input.value);
        error.textContent = why ?? '';
        if (why && ok) {
          ok = false;
          input.focus();
        }
      }
      if (!ok) return;
      result = Object.fromEntries(inputs.map(({ v, input }) => [v.name, input.value.trim()]));
      close();
    };
    for (const { input } of inputs) {
      input.addEventListener('keydown', (e) => {
        if ((e as KeyboardEvent).key === 'Enter' && !(input instanceof HTMLTextAreaElement && !(e as KeyboardEvent).ctrlKey)) {
          e.preventDefault();
          submit();
        }
      });
    }
    setTimeout(() => inputs[0]?.input.focus());
    return h(
      'div',
      { class: 'dialog-small wide' },
      h('h2', {}, 'Fill in the prompt'),
      h('p', { class: 'muted' }, 'This prompt has blanks to fill before it is sent.'),
      inputs.map(({ v, input, error }) => h('label', { class: 'field' }, h('span', { class: 'field-label' }, label(v.name), v.optional ? h('small', { class: 'muted' }, ' (optional)') : null), input, error)),
      h('div', { class: 'dialog-actions' }, h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'), h('button', { class: 'btn btn-primary', on: { click: submit } }, 'Send'))
    );
  }).then(() => result);
}

/** `trip_length` as "Trip length". */
function label(name: string): string {
  const words = name.replace(/_/g, ' ').trim();
  return words.charAt(0).toUpperCase() + words.slice(1);
}
