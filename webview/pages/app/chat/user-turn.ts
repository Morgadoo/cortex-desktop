/** A message the user sent, as the transcript shows it: its chips, images, attached files and actions. */
import type { Part, Turn } from '../core/api';
import { h } from '../ui/h';
import { icon } from '../ui/icons';
import { timeLabel } from '../core/clock';
import { IMAGE_TEMPLATES } from './send';
import { bookmarkButton, deleteExchangeButton, editButton } from './turn-actions';
import { textOf } from './transcript';

export function renderUserTurn(turn: Turn, conversationId: string, branch: HTMLElement | null = null): HTMLElement {
  const text = textOf(turn.parts);
  const copy = h('button', { class: 'icon-btn small', title: 'Copy' }, icon('copy', 15));
  copy.addEventListener('click', () => void navigator.clipboard.writeText(text));
  // A skill invoked with /name reads as a chip ahead of the message.
  let rest = text;
  const chips: HTMLElement[] = [];
  const steered = rest.match(/^\(Sent while you were working\) /);
  if (steered) {
    chips.push(h('span', { class: 'skill-chip' }, icon('forward', 14), 'Added while working'));
    rest = rest.slice(steered[0].length);
  }
  // A follow-up the user scheduled from this chat, running here.
  const followUp = rest.match(/^\(Scheduled follow-up: (.+?)\) /);
  if (followUp) {
    chips.push(h('span', { class: 'skill-chip' }, icon('clock', 14), `Scheduled: ${followUp[1]}`));
    rest = rest.slice(followUp[0].length);
  }
  const workflow = rest.match(/^Run the "(.+?)" workflow\. Go through these steps[\s\S]*?\n\n/);
  if (workflow) {
    chips.push(h('span', { class: 'skill-chip' }, icon('workflow', 14), workflow[1]!));
    rest = rest.slice(workflow[0].length);
  }
  const library = rest.match(/^Search the "(.+?)" library \(search_library\) for this, and cite what you use\.\s*/);
  if (library) {
    chips.push(h('span', { class: 'skill-chip' }, icon('library', 14), library[1]!));
    rest = rest.slice(library[0].length);
  }
  const folder = rest.match(/^Work with the files in the folder `(.+?)`\.\s*/);
  if (folder) {
    chips.push(h('span', { class: 'skill-chip' }, icon('folder', 14), folder[1]!.split(/[\\/]/).pop() || folder[1]!));
    rest = rest.slice(folder[0].length);
  }
  const canvasAsk = rest.match(/^Use a canvas for this\.\s*/);
  if (canvasAsk) {
    chips.push(h('span', { class: 'skill-chip' }, icon('file', 14), 'Canvas'));
    rest = rest.slice(canvasAsk[0].length);
  }
  const skill = rest.match(/^Use the "([a-z0-9-]+)" skill\.\s*/);
  if (skill) {
    chips.push(h('span', { class: 'skill-chip' }, icon('skill', 14), skill[1]!));
    rest = rest.slice(skill[0].length);
  }
  // Image mode's instruction (imageDirective) reads as a chip, as Grok shows its image tool.
  const imageAsk = rest.match(/^Create (an image|(\d) images \(variations\)) with generate_image \(shape: (\w+)[^)]*\)\.([^\n]*)\n\n/);
  if (imageAsk) {
    const template = IMAGE_TEMPLATES.find((t) => imageAsk[4]!.trim() && imageAsk[4]!.includes(t.brief.slice(0, 40)));
    const label = ['Image', template?.label, imageAsk[3] !== 'square' ? imageAsk[3]![0]!.toUpperCase() + imageAsk[3]!.slice(1) : null, imageAsk[2] ? `×${imageAsk[2]}` : null].filter(Boolean).join(' · ');
    chips.push(h('span', { class: 'skill-chip' }, icon('image', 14), label));
    rest = rest.slice(imageAsk[0].length);
  }
  const quote = rest.match(/^((?:> .*(?:\n|$))+)\n*/);
  let quoteEl: HTMLElement | null = null;
  if (quote) {
    quoteEl = h('div', { class: 'bubble-quote' }, icon('forward', 13), quote[1]!.replace(/^> /gm, '').trim());
    rest = rest.slice(quote[0].length);
  }
  const images = turn.parts.filter((p): p is Extract<Part, { kind: 'image' }> => p.kind === 'image');
  const gallery = images.length
    ? h(
        'div',
        { class: 'bubble-images' },
        images.map((i) => h('img', { src: `data:${i.mediaType};base64,${i.data}`, alt: 'Attached image' }))
      )
    : null;
  // Attached text files read as file chips that open in place, not as raw markup.
  const files: HTMLElement[] = [];
  rest = rest
    .replace(/\n*<attachment name="([^"]*)">\n?([\s\S]*?)\n?<\/attachment>/g, (_, name: string, body: string) => {
      files.push(h('details', { class: 'bubble-file' }, h('summary', {}, icon('file', 14), h('span', {}, name), h('small', { class: 'muted' }, `${body.split('\n').length} lines`)), h('pre', {}, body)));
      return '';
    })
    .trim();
  // Files kept in the Uploads library to be searched: chips too, without the note to the model.
  rest = rest
    .replace(/\n*\(Attached and kept in the "Uploads" library: ([^)]*)\. Search it with search_library[^)]*\)/g, (_, list: string) => {
      for (const m of list.matchAll(/"([^"]+)"/g))
        files.push(h('div', { class: 'bubble-file searched', title: 'Kept in the Uploads library; the assistant searches it' }, icon('library', 14), h('span', {}, m[1]!), h('small', { class: 'muted' }, 'searched')));
      return '';
    })
    .trim();
  const body = h('div', { class: 'bubble-text' }, rest);
  const bubble = h('div', { class: 'bubble' }, gallery, quoteEl, chips, body, files.length ? h('div', { class: 'bubble-files' }, files) : null);
  // Long messages (pasted documents, logs) fold to a few lines, with Show more.
  const long = rest.length > 1200 || rest.split('\n').length > 16;
  let toggle: HTMLElement | null = null;
  if (long) {
    body.classList.add('folded');
    toggle = h('button', { class: 'fold-toggle' }, 'Show more');
    toggle.addEventListener('click', () => {
      const folded = body.classList.toggle('folded');
      toggle!.textContent = folded ? 'Show more' : 'Show less';
    });
    bubble.append(toggle);
  }
  return h(
    'div',
    { class: 'turn turn-user', 'data-index': String(turn.index) },
    bubble,
    h(
      'div',
      { class: `turn-actions user-actions${branch ? ' pinned' : ''}` },
      branch,
      turn.index >= 0 ? editButton(turn, conversationId, text) : null,
      copy,
      turn.index >= 0 ? bookmarkButton(turn, conversationId, 15) : null,
      turn.index >= 0 ? deleteExchangeButton(turn, conversationId) : null,
      h('span', { class: 'turn-time' }, timeLabel(turn.time))
    )
  );
}
