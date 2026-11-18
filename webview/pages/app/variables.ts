/**
 * Prompt variables, as in Open WebUI and LibreChat: `{{current_date}}` and friends fill in on
 * send; any other `{{name}}` asks for a value first, and `{{name|a|b}}` offers a choice. Works for
 * typed prompts and workflows, so a saved prompt becomes a small form.
 */
import { api } from './api';
import { h, modal, clockHour12 } from './dom';

const PATTERN = /\{\{\s*([A-Za-z_][\w ]{0,40}?)\s*(\|[^{}]*)?\}\}/g;

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

export interface Variable {
  name: string;
  /** A fixed set to pick from, when written as `{{name|a|b}}`. */
  options: string[];
}

/** The variables in `text` the user must fill, each once, in order of appearance. */
export function variablesIn(text: string, known: Record<string, string>): Variable[] {
  const out: Variable[] = [];
  for (const m of text.matchAll(PATTERN)) {
    const name = m[1]!.trim();
    if (name.toLowerCase() in known || out.some((v) => v.name === name)) continue;
    const options = (m[2] ?? '').split('|').map((o) => o.trim()).filter(Boolean);
    out.push({ name, options });
  }
  return out;
}

/** `text` with every variable replaced by its value. */
export function substitute(text: string, values: Record<string, string>): string {
  return text.replace(PATTERN, (whole, raw: string) => {
    const name = raw.trim();
    const value = values[name] ?? values[name.toLowerCase()];
    return value === undefined ? whole : value;
  });
}

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
    const inputs = wanted.map((v) => {
      const input = v.options.length
        ? h('select', { class: 'input' }, v.options.map((o) => h('option', { value: o }, o)))
        : h('input', { class: 'input', placeholder: label(v.name) });
      return { v, input: input as HTMLInputElement | HTMLSelectElement };
    });
    const submit = () => {
      result = Object.fromEntries(inputs.map(({ v, input }) => [v.name, input.value.trim()]));
      close();
    };
    for (const { input } of inputs) {
      input.addEventListener('keydown', (e) => {
        if ((e as KeyboardEvent).key === 'Enter') {
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
      inputs.map(({ v, input }) => h('label', { class: 'field' }, h('span', { class: 'field-label' }, label(v.name)), input)),
      h(
        'div',
        { class: 'dialog-actions' },
        h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'),
        h('button', { class: 'btn btn-primary', on: { click: submit } }, 'Send')
      )
    );
  }).then(() => result);
}

/** `trip_length` as "Trip length". */
function label(name: string): string {
  const words = name.replace(/_/g, ' ').trim();
  return words.charAt(0).toUpperCase() + words.slice(1);
}
