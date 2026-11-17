/** A code run in the conversation: the program, what it printed, and the charts it saved. */
import type { Part } from './api';
import { h, icon, modal, toast } from './dom';
import { iconButton } from './ui';

interface RunReport {
  language: string;
  exit_code: number | null;
  timed_out: boolean;
  seconds: number;
  stdout: string;
  stderr: string;
  files: { name: string; bytes: number; new: boolean }[];
  isolated: boolean;
}

const LANGUAGE_NAMES: Record<string, string> = { python: 'Python', javascript: 'JavaScript', bash: 'Bash' };
const IMAGE = /\.(png|jpe?g|gif|webp|svg)$/i;

/** Where the chat loads a file from a conversation's workspace. */
export function workspaceUrl(conversationId: string, name: string): string {
  const windows = navigator.userAgent.includes('Windows');
  const path = name.split('/').map(encodeURIComponent).join('/');
  return `${windows ? 'http://lumoapp.localhost' : 'lumoapp://localhost'}/sandbox/${conversationId}/${path}?t=${Date.now()}`;
}

function parse(output: string | null): RunReport | null {
  if (!output) return null;
  try {
    const json = output.slice(0, output.lastIndexOf('}') + 1);
    return JSON.parse(json) as RunReport;
  } catch {
    return null;
  }
}

function size(bytes: number): string {
  return bytes < 1024 ? `${bytes} B` : bytes < 1024 * 1024 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/** An image the model generated, saved in the conversation's workspace. */
export function renderImage(part: Extract<Part, { kind: 'tool' }>, live: boolean, conversationId: string): HTMLElement {
  const prompt = typeof part.arguments.prompt === 'string' ? part.arguments.prompt : '';
  const running = live && part.output === null;
  let file = '';
  try {
    file = part.output && !part.isError ? ((JSON.parse(part.output) as { file?: string }).file ?? '') : '';
  } catch {
    file = '';
  }
  let body: HTMLElement;
  if (running) {
    body = h('div', { class: 'image-wait' }, h('span', { class: 'dots' }), 'Creating the image…');
  } else if (file) {
    const url = workspaceUrl(conversationId, file);
    const img = h('img', { class: 'generated-image', src: url, alt: prompt, title: 'Open larger' });
    img.addEventListener('click', () =>
      void modal((close) => h('div', { class: 'image-modal', on: { click: close } }, h('img', { src: url, alt: prompt })), true)
    );
    body = img;
  } else {
    body = h('pre', { class: 'run-stderr' }, part.output ?? 'The image could not be made.');
  }
  const copy = iconButton({ icon: 'copy', title: 'Copy prompt', size: 'sm' });
  copy.addEventListener('click', () => {
    void navigator.clipboard.writeText(prompt);
    toast('Prompt copied.');
  });
  return h(
    'div',
    { class: 'canvas-step' },
    h('div', { class: 'canvas-label' }, running ? 'Creating an image…' : part.isError ? 'Could not create the image' : 'Created an image'),
    h('div', { class: 'image-card' }, body, h('div', { class: 'image-caption' }, h('span', { class: 'muted' }, prompt), copy))
  );
}

export function renderCodeRun(part: Extract<Part, { kind: 'tool' }>, live: boolean, conversationId: string): HTMLElement {
  const language = typeof part.arguments.language === 'string' ? part.arguments.language : 'python';
  const code = typeof part.arguments.code === 'string' ? part.arguments.code : '';
  const running = live && part.output === null;
  const report = parse(part.output);
  const name = LANGUAGE_NAMES[report?.language ?? language] ?? language;
  const failed = part.isError;

  const status = running
    ? h('span', { class: 'dots' })
    : report?.timed_out
      ? h('span', { class: 'step-badge' }, 'timed out')
      : failed
        ? h('span', { class: 'step-badge' }, report?.exit_code != null ? `exit ${report.exit_code}` : 'failed')
        : h('span', { class: 'run-ok' }, icon('check', 14));
  const copy = iconButton({ icon: 'copy', title: 'Copy code', size: 'sm' });
  copy.addEventListener('click', (e) => {
    e.preventDefault();
    void navigator.clipboard.writeText(code);
    toast('Code copied.');
  });
  const codeBlock = h(
    'details',
    { class: 'run-code', open: running },
    h('summary', {}, icon('chevron', 13), `${code.split('\n').length} lines of ${name}`, h('span', { class: 'spacer' }), copy),
    h('pre', { class: 'step-pre' }, h('code', {}, code))
  );

  const body: (HTMLElement | null)[] = [codeBlock];
  if (report) {
    const out = [report.stdout.trimEnd(), report.stderr.trimEnd()].filter(Boolean);
    if (out.length) {
      body.push(
        h(
          'div',
          { class: 'run-output' },
          report.stdout.trim() ? h('pre', { class: 'run-stdout' }, report.stdout.trimEnd()) : null,
          report.stderr.trim() ? h('pre', { class: 'run-stderr' }, report.stderr.trimEnd()) : null
        )
      );
    }
    const made = report.files.filter((f) => f.new);
    const images = made.filter((f) => IMAGE.test(f.name));
    if (images.length) {
      body.push(h('div', { class: 'run-images' }, images.map((f) => h('img', { src: workspaceUrl(conversationId, f.name), alt: f.name, title: f.name }))));
    }
    const others = made.filter((f) => !IMAGE.test(f.name));
    if (others.length) {
      body.push(h('div', { class: 'run-files' }, others.map((f) => h('span', { class: 'run-file' }, icon('file', 13), f.name, h('small', { class: 'muted' }, size(f.bytes))))));
    }
  } else if (part.output && !running) {
    body.push(h('pre', { class: 'run-stderr' }, part.output));
  }
  const meta = report ? `${report.seconds.toFixed(1)}s${report.isolated ? ' · sandboxed' : ''}` : '';
  return h(
    'div',
    { class: 'canvas-step' },
    h('div', { class: 'canvas-label' }, running ? `Running ${name}…` : `Ran ${name}`),
    h(
      'div',
      { class: `run-card${failed ? ' failed' : ''}` },
      h('div', { class: 'run-head' }, icon('terminal', 15), h('strong', {}, name), status, h('span', { class: 'spacer' }), h('span', { class: 'muted run-meta' }, meta)),
      ...body
    )
  );
}
