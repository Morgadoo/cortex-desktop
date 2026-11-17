/** A coding session's working-tree changes: the files touched and their diffs. */
import { tauriInvoke } from '../../shared/tauri';
import { clear, h, icon, modal, promptText, toast } from './dom';
import { iconButton, modalHeader } from './ui';

export interface ChangedFile {
  path: string;
  status: 'modified' | 'added' | 'deleted' | 'renamed' | 'untracked';
  additions: number;
  deletions: number;
}

export interface Changes {
  git: boolean;
  branch: string | null;
  files: ChangedFile[];
  diff: string;
  truncated: boolean;
}

export const loadChanges = (conversationId: string) => tauriInvoke<Changes>('chat_code_changes', { conversationId });

/** Unified diff lines, colored: additions, removals, hunk headers and context. */
export function diffLines(lines: string[]): HTMLElement {
  return h(
    'div',
    { class: 'diff' },
    lines.map((line) => {
      const kind = line.startsWith('@@') ? 'hunk' : line.startsWith('+') ? 'add' : line.startsWith('-') ? 'del' : 'ctx';
      return h('div', { class: `diff-line ${kind}` }, line || ' ');
    })
  );
}

/**
 * Side by side: the old file on the left, the new on the right, removals paired with the
 * additions that replace them (Vibe's "Split" view).
 */
export function splitDiffLines(lines: string[]): HTMLElement {
  const rows: [string | null, string | null, string][] = [];
  let removed: string[] = [];
  let added: string[] = [];
  const flush = () => {
    for (let i = 0; i < Math.max(removed.length, added.length); i++) rows.push([removed[i] ?? null, added[i] ?? null, 'change']);
    removed = [];
    added = [];
  };
  for (const line of lines) {
    if (line.startsWith('@@')) {
      flush();
      rows.push([line, line, 'hunk']);
    } else if (line.startsWith('-')) removed.push(line.slice(1));
    else if (line.startsWith('+')) added.push(line.slice(1));
    else {
      flush();
      rows.push([line.slice(1), line.slice(1), 'ctx']);
    }
  }
  flush();
  const cell = (text: string | null, kind: string, side: 'del' | 'add') =>
    h('div', { class: `split-cell ${text === null ? 'empty' : kind === 'change' ? side : kind}` }, text ?? '');
  return h('div', { class: 'diff split' }, rows.map(([left, right, kind]) => h('div', { class: 'split-row' }, cell(left, kind, 'del'), cell(right, kind, 'add'))));
}

/** The diff of each file, keyed by its path, from `git diff` output. */
function splitDiff(diff: string): Map<string, string[]> {
  const out = new Map<string, string[]>();
  let current: string[] | null = null;
  for (const line of diff.split('\n')) {
    const header = line.match(/^diff --git a\/(.+?) b\/(.+)$/);
    if (header) {
      current = [];
      out.set(header[2]!, current);
      continue;
    }
    if (!current || /^(index |--- |\+\+\+ |similarity |rename |new file mode|deleted file mode|old mode|new mode)/.test(line)) continue;
    current.push(line);
  }
  return out;
}

const STATUS_LETTER: Record<ChangedFile['status'], string> = { modified: 'M', added: 'A', deleted: 'D', renamed: 'R', untracked: 'U' };

export function openChanges(conversationId: string, folder: string) {
  void modal((close) => {
    const list = h('div', { class: 'changes-files' });
    const view = h('div', { class: 'changes-diff' }, h('div', { class: 'status-line' }, h('span', { class: 'dots' }), 'Reading changes'));
    const title = h('span', { class: 'muted' }, folder);
    const refresh = iconButton({ icon: 'retry', title: 'Refresh' });
    // Unified or Split, as in Vibe's diff viewer; remembered for next time.
    let layout: 'unified' | 'split' = (() => { try { return localStorage.getItem('lumo.diffLayout') === 'split' ? 'split' : 'unified'; } catch { return 'unified'; } })();
    let rerender = () => {};
    const unified = h('button', { class: `seg${layout === 'unified' ? ' active' : ''}` }, 'Unified');
    const split = h('button', { class: `seg${layout === 'split' ? ' active' : ''}` }, 'Split');
    const setLayout = (next: 'unified' | 'split') => {
      layout = next;
      unified.classList.toggle('active', next === 'unified');
      split.classList.toggle('active', next === 'split');
      try { localStorage.setItem('lumo.diffLayout', next); } catch { /* a convenience */ }
      rerender();
    };
    unified.addEventListener('click', () => setLayout('unified'));
    split.addEventListener('click', () => setLayout('split'));
    const layoutSwitch = h('div', { class: 'segmented' }, unified, split);
    const load = async () => {
      const changes = await loadChanges(conversationId).catch((e) => ({ git: false, branch: null, files: [], diff: String(e), truncated: false }) as Changes);
      title.textContent = changes.branch ? `${folder} · ${changes.branch}` : folder;
      if (!changes.git) {
        clear(list);
        clear(view, h('div', { class: 'empty-state' }, h('p', { class: 'muted' }, 'This folder is not a git repository, so there are no changes to compare.')));
        return;
      }
      if (!changes.files.length) {
        clear(list);
        clear(view, h('div', { class: 'empty-state' }, icon('check', 22), h('p', { class: 'muted' }, 'No uncommitted changes.')));
        return;
      }
      const diffs = splitDiff(changes.diff);
      let current: { file: ChangedFile | null; row: HTMLElement | null } = { file: null, row: null };
      const show = (file: ChangedFile | null, row: HTMLElement | null) => {
        current = { file, row };
        list.querySelectorAll('.active').forEach((el) => el.classList.remove('active'));
        row?.classList.add('active');
        const files = file ? [file] : changes.files;
        clear(
          view,
          files.map((f) =>
            h(
              'section',
              { class: 'diff-file' },
              h('div', { class: 'diff-file-head' }, icon('file', 14), h('strong', {}, f.path), h('span', { class: `chg-status ${f.status}` }, f.status)),
              f.status === 'untracked'
                ? h('p', { class: 'muted diff-note' }, `New file, ${f.additions} lines, not yet tracked by git.`)
                : diffs.get(f.path)
                  ? (layout === 'split' ? splitDiffLines : diffLines)(diffs.get(f.path)!)
                  : h('p', { class: 'muted diff-note' }, 'Binary file or no textual changes.')
            )
          ),
          changes.truncated && !file ? h('p', { class: 'muted diff-note' }, 'The diff is long; showing the beginning.') : null
        );
      };
      rerender = () => show(current.file, current.row);
      const message = h('input', { class: 'input small', placeholder: 'Commit message' });
      const commit = h('button', { class: 'btn btn-primary btn-sm' }, 'Commit all');
      commit.addEventListener('click', async () => {
        try {
          const line = await tauriInvoke<string>('chat_code_commit', { conversationId, message: message.value });
          toast(`Committed ${line}`);
          message.value = '';
          void load();
        } catch (e) {
          toast(String(e), 'error');
        }
      });
      const all = h('button', { class: 'chg-row all active' }, h('span', { class: 'grow' }, `All files (${changes.files.length})`), stat(changes.files.reduce((a, f) => a + f.additions, 0), changes.files.reduce((a, f) => a + f.deletions, 0)));
      all.addEventListener('click', () => show(null, all));
      clear(
        list,
        h('div', { class: 'commit-box' }, message, commit),
        all,
        changes.files.map((f) => {
          const row = h('button', { class: 'chg-row', title: f.path }, h('span', { class: `chg-letter ${f.status}` }, STATUS_LETTER[f.status]), h('span', { class: 'chg-path' }, f.path), stat(f.additions, f.deletions));
          row.addEventListener('click', () => show(f, row));
          return row;
        })
      );
      show(null, all);
    };
    refresh.addEventListener('click', () => void load());
    // Push the session's branch and open a pull request: GitHub CLI when installed, else the browser.
    const pullRequest = h('button', { class: 'btn btn-secondary btn-sm' }, icon('share', 14), 'Pull request');
    pullRequest.addEventListener('click', async () => {
      const title = await promptText('Pull request title', '', 'What these changes do');
      if (!title) return;
      pullRequest.setAttribute('disabled', '');
      try {
        const pr = await tauriInvoke<{ branch: string; url: string | null; via: string }>('chat_code_pull_request', { conversationId, title });
        if (pr.url) window.open(pr.url, '_blank', 'noopener');
        toast(
          pr.via === 'gh'
            ? `Pull request opened from ${pr.branch}.`
            : pr.via === 'browser'
              ? `Pushed ${pr.branch}; finish the pull request in your browser.`
              : `Pushed ${pr.branch}. The remote is not on GitHub, so open the pull request there.`
        );
        void load();
      } catch (e) {
        toast(String(e), 'error');
      } finally {
        pullRequest.removeAttribute('disabled');
      }
    });
    void load();
    return h(
      'div',
      { class: 'changes-modal' },
      modalHeader(h('div', { class: 'modal-title' }, h('h2', {}, 'Changes'), title), close, { actions: [layoutSwitch, pullRequest, refresh] }),
      h('div', { class: 'changes-body' }, list, view)
    );
  }, true);
}

function stat(additions: number, deletions: number): HTMLElement {
  return h('span', { class: 'chg-stat' }, h('span', { class: 'plus' }, `+${additions}`), h('span', { class: 'minus' }, `−${deletions}`));
}

/** The session folder's files: filter by name, read one with line numbers (Vibe's Files panel). */
export function openFiles(conversationId: string, folder: string) {
  void modal((close) => {
    const filter = h('input', { class: 'input small', placeholder: 'Filter files' });
    const list = h('div', { class: 'changes-files' }, h('div', { class: 'status-line' }, h('span', { class: 'dots' }), 'Listing files'));
    const view = h('div', { class: 'changes-diff' }, h('div', { class: 'empty-state' }, h('p', { class: 'muted' }, 'Choose a file.')));
    let all: string[] = [];
    const open = async (path: string, row: HTMLElement) => {
      list.querySelectorAll('.active').forEach((el) => el.classList.remove('active'));
      row.classList.add('active');
      try {
        const text = await tauriInvoke<string>('chat_code_read_file', { conversationId, path });
        const lines = text.split('\n');
        clear(
          view,
          h('div', { class: 'diff-file-head' }, icon('file', 14), h('strong', {}, path), h('span', { class: 'muted' }, `${lines.length} lines`)),
          h('div', { class: 'file-view' }, lines.map((line, i) => h('div', { class: 'file-line' }, h('span', { class: 'line-no' }, String(i + 1)), h('span', {}, line || ' '))))
        );
      } catch (e) {
        clear(view, h('p', { class: 'muted diff-note' }, String(e)));
      }
    };
    const render = () => {
      const q = filter.value.toLowerCase().trim();
      const shown = all.filter((f) => !q || f.toLowerCase().includes(q)).slice(0, 500);
      clear(
        list,
        shown.map((path) => {
          const slash = path.lastIndexOf('/');
          const row = h('button', { class: 'chg-row', title: path }, icon('file', 14), h('span', { class: 'chg-path' }, h('strong', {}, path.slice(slash + 1)), slash > 0 ? h('small', { class: 'muted' }, path.slice(0, slash)) : null));
          row.addEventListener('click', () => void open(path, row));
          return row;
        }),
        shown.length ? null : h('p', { class: 'muted diff-note' }, 'No files match.')
      );
    };
    filter.addEventListener('input', render);
    void tauriInvoke<[string[], boolean]>('chat_code_files', { conversationId })
      .then(([files, truncated]) => {
        all = files;
        render();
        if (truncated) list.append(h('p', { class: 'muted diff-note' }, 'Showing the first 5,000 files.'));
      })
      .catch((e) => clear(list, h('p', { class: 'muted diff-note' }, String(e))));
    setTimeout(() => filter.focus());
    return h(
      'div',
      { class: 'changes-modal' },
      modalHeader(h('div', { class: 'modal-title' }, h('h2', {}, 'Files'), h('span', { class: 'muted' }, folder)), close, { actions: [filter] }),
      h('div', { class: 'changes-body' }, list, view)
    );
  }, true);
}

/** The Debug panel of a coding session: what the model is working from, message by message. */
export function openDebug(conversationId: string) {
  void modal((close) => {
    const body = h('div', { class: 'debug-body' }, h('div', { class: 'status-line' }, h('span', { class: 'dots' }), 'Reading the session'));
    void tauriInvoke<{
      id: string;
      model: string | null;
      workdir: string | null;
      agentId: string | null;
      usage: { inputTokens: number; outputTokens: number; cost: number; contextTokens: number };
      summary: { text: string; upto: number } | null;
      messages: Record<string, unknown>[];
    }>('chat_debug', { id: conversationId })
      .then((d) => {
        const kindOf = (m: Record<string, unknown>): string => {
          const content = (m.content as Record<string, unknown>[] | undefined) ?? [];
          const kinds = [...new Set(content.map((c) => String(c.type ?? 'text')))];
          return kinds.join(', ') || 'text';
        };
        const raw = JSON.stringify(d.messages, null, 2);
        const copy = h('button', { class: 'btn btn-secondary btn-sm' }, icon('copy', 14), 'Copy history as JSON');
        copy.addEventListener('click', () => { void navigator.clipboard.writeText(raw); toast('History copied.'); });
        const fact = (k: string, v: string) => h('div', { class: 'debug-fact' }, h('span', { class: 'muted' }, k), h('code', {}, v));
        clear(
          body,
          h(
            'div',
            { class: 'debug-facts' },
            fact('Session', d.id),
            fact('Model', d.model ?? 'the composer\'s'),
            fact('Folder', d.workdir ?? 'none'),
            fact('Agent', d.agentId ?? 'none'),
            fact('Tokens', `${d.usage.inputTokens.toLocaleString()} in · ${d.usage.outputTokens.toLocaleString()} out · ${d.usage.contextTokens.toLocaleString()} in the last request`),
            fact('Messages', `${d.messages.length} (${(raw.length / 1024).toFixed(0)} KB)`)
          ),
          d.summary ? h('details', { class: 'debug-msg' }, h('summary', {}, h('strong', {}, 'Summary'), h('span', { class: 'muted' }, ` replaces messages 1–${d.summary.upto}`)), h('pre', { class: 'step-pre' }, d.summary.text)) : null,
          h(
            'div',
            { class: 'debug-list' },
            d.messages.map((m, i) => {
              const json = JSON.stringify(m, null, 2);
              return h(
                'details',
                { class: 'debug-msg' },
                h('summary', {}, h('span', { class: 'muted' }, `${i + 1}.`), h('strong', {}, String(m.role ?? '?')), h('span', { class: 'muted' }, kindOf(m)), h('span', { class: 'spacer' }), h('span', { class: 'muted' }, `${(json.length / 1024).toFixed(1)} KB`)),
                h('pre', { class: 'step-pre' }, json.length > 20000 ? `${json.slice(0, 20000)}\n…` : json)
              );
            })
          ),
          h('div', { class: 'card-actions' }, copy)
        );
      })
      .catch((e) => clear(body, h('p', { class: 'muted' }, String(e))));
    return h('div', { class: 'changes-modal debug-modal' }, modalHeader(h('div', { class: 'modal-title' }, h('h2', {}, 'Debug'), h('span', { class: 'muted' }, 'what the model works from')), close), body);
  }, true);
}
