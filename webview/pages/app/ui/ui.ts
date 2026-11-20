/**
 * The design system's components. Every screen builds its controls from these, so a button, a
 * chip or a dialog header looks and behaves the same everywhere, with the same hover, focus and
 * pressed states. The look lives in app.css under the matching class names, built on the tokens
 * at its top (colours, radii, type scale, spacing and motion).
 */
import { h } from './h';
import { icon } from './icons';

type Child = HTMLElement | string | null | undefined | false;

export type ButtonVariant = 'primary' | 'secondary' | 'subtle' | 'danger' | 'link';
export type Size = 'sm' | 'md';

export interface ButtonOptions {
  label?: string;
  icon?: string;
  variant?: ButtonVariant;
  size?: Size;
  title?: string;
  disabled?: boolean;
  onClick?: (e: MouseEvent) => void;
  /** Extra classes, for a layout hook rather than a new look. */
  class?: string;
}

/** A text button, optionally with a leading icon. */
export function button(opts: ButtonOptions): HTMLButtonElement {
  const { label, variant = 'secondary', size = 'md' } = opts;
  const el = h(
    'button',
    {
      type: 'button',
      class: ['btn', `btn-${variant}`, size === 'sm' ? 'btn-sm' : '', opts.class ?? ''].filter(Boolean).join(' '),
      title: opts.title ?? undefined,
      disabled: opts.disabled ?? false,
    },
    opts.icon ? icon(opts.icon, size === 'sm' ? 15 : 16) : null,
    label ?? null
  );
  if (opts.onClick) el.addEventListener('click', opts.onClick);
  return el;
}

export interface IconButtonOptions {
  icon: string;
  title: string;
  size?: 'xs' | 'sm' | 'md';
  active?: boolean;
  onClick?: (e: MouseEvent) => void;
  class?: string;
}

/** A square button holding only an icon; `title` doubles as its accessible name. */
export function iconButton(opts: IconButtonOptions): HTMLButtonElement {
  const size = opts.size ?? 'md';
  const el = h(
    'button',
    {
      type: 'button',
      class: ['icon-btn', size === 'md' ? '' : size === 'sm' ? 'small' : 'tiny', opts.active ? 'active' : '', opts.class ?? ''].filter(Boolean).join(' '),
      title: opts.title,
      'aria-label': opts.title,
    },
    icon(opts.icon, size === 'xs' ? 14 : size === 'sm' ? 16 : 18)
  );
  if (opts.onClick) el.addEventListener('click', opts.onClick);
  return el;
}

/** A removable token in the composer or a field: an attachment, a skill, a workflow. */
export function chip(opts: { icon?: string; leading?: Element; label: string; onRemove?: () => void; tone?: 'default' | 'accent' }): HTMLElement {
  return h(
    'span',
    { class: `chip${opts.tone === 'accent' ? ' chip-accent' : ''}` },
    opts.leading ?? (opts.icon ? icon(opts.icon, 14) : null),
    h('span', { class: 'chip-label' }, opts.label),
    opts.onRemove ? iconButton({ icon: 'close', title: `Remove ${opts.label}`, size: 'xs', class: 'chip-x', onClick: opts.onRemove }) : null
  );
}

/** A quiet button that opens a menu or panel: the composer's model and mode pickers. */
export function trigger(opts: { label: string; title: string; icon?: string; trailing?: HTMLElement | null; chevron?: boolean; class?: string; onClick?: (e: MouseEvent) => void }): HTMLButtonElement {
  const el = h(
    'button',
    { type: 'button', class: `pill-btn${opts.class ? ` ${opts.class}` : ''}`, title: opts.title, 'aria-haspopup': opts.chevron === false ? undefined : 'menu' },
    opts.icon ? icon(opts.icon, 15) : null,
    opts.label,
    opts.trailing ?? null,
    opts.chevron === false ? null : icon('chevron', 14)
  );
  if (opts.onClick) el.addEventListener('click', opts.onClick);
  return el;
}

/** A small status label. */
export function badge(text: string, tone: 'neutral' | 'accent' | 'success' | 'warning' | 'danger' = 'neutral'): HTMLElement {
  return h('span', { class: `status-badge tone-${tone}` }, text);
}

/** An on/off switch. */
export function switchInput(checked: boolean, onChange: (value: boolean) => void, title = ''): HTMLInputElement {
  const input = h('input', { type: 'checkbox', role: 'switch', class: 'switch', checked, title: title || undefined, 'aria-label': title || undefined });
  input.addEventListener('click', (e) => e.stopPropagation());
  input.addEventListener('change', () => onChange(input.checked));
  return input;
}

/** A labelled form control, with an optional hint under it. */
export function field(label: string, control: HTMLElement, hint?: string | HTMLElement | null, ...trailing: HTMLElement[]): HTMLElement {
  return h('div', { class: 'field' }, h('label', { class: 'field-label' }, label, ...trailing), control, hint ? (typeof hint === 'string' ? h('small', { class: 'field-hint' }, hint) : hint) : null);
}

/** A page's title row: heading, one-line description, and its main actions on the right. */
export function pageHeader(title: string, subtitle: string, ...actions: (HTMLElement | null)[]): HTMLElement {
  return h('header', { class: 'page-header' }, h('div', {}, h('h1', {}, title), subtitle ? h('p', { class: 'muted' }, subtitle) : null), actions.some(Boolean) ? h('div', { class: 'page-actions' }, actions) : null);
}

/** What a list shows when it has nothing in it, with the action that fills it. */
export function emptyState(ic: string, title: string, body: string, ...actions: (HTMLElement | null)[]): HTMLElement {
  return h('div', { class: 'empty-state' }, h('div', { class: 'empty-icon' }, icon(ic, 22)), h('h3', {}, title), h('p', { class: 'muted' }, body), actions.some(Boolean) ? h('div', { class: 'empty-actions' }, actions) : null);
}

/** A dialog's header: its title, optional extra controls, and the close button. */
export function modalHeader(title: string | HTMLElement, close: () => void, opts: { flush?: boolean; actions?: Child[] } = {}): HTMLElement {
  return h(
    'div',
    { class: `modal-head${opts.flush ? ' flush' : ''}` },
    typeof title === 'string' ? h('h2', {}, title) : title,
    ...(opts.actions?.length ? [h('span', { class: 'spacer' }), ...(opts.actions as (HTMLElement | string)[]).filter(Boolean)] : []),
    iconButton({ icon: 'close', title: 'Close', onClick: close, class: 'modal-close' })
  );
}

/** A dialog's footer buttons, right-aligned: secondary actions first, the main one last. */
export function dialogActions(...buttons: (HTMLElement | null)[]): HTMLElement {
  return h('div', { class: 'dialog-actions' }, buttons);
}

/** A grouped section of a settings or detail page. */
export function card(...children: (HTMLElement | null)[]): HTMLElement {
  return h('section', { class: 'card' }, children);
}

/** A segmented control: a few mutually exclusive views. Returns the element and a setter. */
export function segmented<T extends string>(options: { value: T; label: string }[], value: T, onChange: (value: T) => void): HTMLElement {
  const root = h('div', { class: 'segmented', role: 'tablist' });
  const render = (current: T) => {
    root.replaceChildren(
      ...options.map((o) => {
        const el = h('button', { type: 'button', role: 'tab', class: `seg${o.value === current ? ' active' : ''}`, 'aria-selected': String(o.value === current) }, o.label);
        el.addEventListener('click', () => {
          render(o.value);
          onChange(o.value);
        });
        return el;
      })
    );
  };
  render(value);
  return root;
}

/** Tabs over a page's sections, with optional counts. */
export function tabs<T extends string>(options: { value: T; label: string; count?: number }[], value: T, onChange: (value: T) => void): HTMLElement {
  return h(
    'div',
    { class: 'tabs', role: 'tablist' },
    options.map((o) =>
      h(
        'button',
        { type: 'button', role: 'tab', class: `tab${o.value === value ? ' active' : ''}`, 'aria-selected': String(o.value === value), on: { click: () => onChange(o.value) } },
        o.label,
        o.count !== undefined ? h('span', { class: 'tab-count' }, String(o.count)) : null
      )
    )
  );
}
