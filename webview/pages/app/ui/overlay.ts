/** What floats over the page: modal dialogs, prompts, confirmations, popovers and toasts. */
import { h } from './h';

/** A modal dialog over the page. Resolves when closed. */
export function modal(build: (close: () => void) => HTMLElement, wide = false): Promise<void> {
  return new Promise((resolve) => {
    const backdrop = h('div', { class: 'modal-backdrop' });
    const close = () => {
      backdrop.remove();
      document.removeEventListener('keydown', onKey);
      resolve();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close();
    };
    const box = h('div', { class: wide ? 'modal modal-wide' : 'modal', role: 'dialog' });
    box.append(build(close));
    backdrop.append(box);
    backdrop.addEventListener('mousedown', (e) => {
      if (e.target === backdrop) close();
    });
    document.addEventListener('keydown', onKey);
    document.body.append(backdrop);
    box.querySelector<HTMLElement>('input, textarea, [autofocus]')?.focus();
  });
}

/** A one-field prompt in a modal; resolves to the trimmed value or null. */
export function promptText(title: string, value = '', placeholder = ''): Promise<string | null> {
  let result: string | null = null;
  return modal((close) => {
    const input = h('input', { class: 'input', value, placeholder });
    const submit = () => {
      result = input.value.trim() || null;
      close();
    };
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') submit();
    });
    return h(
      'div',
      { class: 'dialog-small' },
      h('h2', {}, title),
      input,
      h('div', { class: 'dialog-actions' }, h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'), h('button', { class: 'btn btn-primary', on: { click: submit } }, 'Save'))
    );
  }).then(() => result);
}

export function confirmDialog(title: string, body: string, action = 'Delete', danger = true): Promise<boolean> {
  let ok = false;
  return modal((close) =>
    h(
      'div',
      { class: 'dialog-small' },
      h('h2', {}, title),
      h('p', { class: 'muted' }, body),
      h(
        'div',
        { class: 'dialog-actions' },
        h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'),
        h(
          'button',
          {
            class: danger ? 'btn btn-danger' : 'btn btn-primary',
            on: {
              click: () => {
                ok = true;
                close();
              },
            },
          },
          action
        )
      )
    )
  ).then(() => ok);
}

/** A floating menu anchored under `anchor`; closes on outside click. */
export function popover(anchor: HTMLElement, content: HTMLElement, placement: 'below' | 'above' = 'below'): () => void {
  document.querySelectorAll('.popover').forEach((p) => p.remove());
  const pop = h('div', { class: 'popover' }, content);
  document.body.append(pop);
  const rect = anchor.getBoundingClientRect();
  const width = pop.offsetWidth;
  const left = Math.min(Math.max(8, rect.left), window.innerWidth - width - 8);
  pop.style.left = `${left}px`;
  if (placement === 'above' || rect.bottom + pop.offsetHeight > window.innerHeight - 8) {
    pop.style.top = `${Math.max(8, rect.top - pop.offsetHeight - 6)}px`;
  } else {
    pop.style.top = `${rect.bottom + 6}px`;
  }
  const close = () => {
    pop.remove();
    document.removeEventListener('mousedown', outside, true);
  };
  const outside = (e: MouseEvent) => {
    if (!pop.contains(e.target as Node) && !anchor.contains(e.target as Node)) close();
  };
  setTimeout(() => document.addEventListener('mousedown', outside, true));
  return close;
}

let toastTimer: number | undefined;
export function toast(message: string, kind: 'info' | 'error' = 'info') {
  document.querySelector('.toast')?.remove();
  const el = h('div', { class: `toast toast-${kind}`, role: 'status' }, message);
  document.body.append(el);
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => el.remove(), kind === 'error' ? 6000 : 3000);
}
