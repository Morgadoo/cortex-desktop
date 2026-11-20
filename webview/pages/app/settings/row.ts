/** The settings dialog's row: a label, what it does, and its control. */
import { h } from '../ui/h';

export function row(label: string, description: string, control: HTMLElement): HTMLElement {
  return h('div', { class: 'setting-row' }, h('div', {}, h('strong', {}, label), description ? h('small', { class: 'muted' }, description) : null), control);
}
