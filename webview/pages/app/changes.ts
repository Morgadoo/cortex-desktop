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
      const show = (file: ChangedFile | null, row: HTMLElement | null) => {
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
                  ? diffLines(diffs.get(f.path)!)
                  : h('p', { class: 'muted diff-note' }, 'Binary file or no textual changes.')
            )
          ),
          changes.truncated && !file ? h('p', { class: 'muted diff-note' }, 'The diff is long; showing the beginning.') : null
        );
      };
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
      modalHeader(h('div', { class: 'modal-title' }, h('h2', {}, 'Changes'), title), close, { actions: [pullRequest, refresh] }),
      h('div', { class: 'changes-body' }, list, view)
    );
  }, true);
}

function stat(additions: number, deletions: number): HTMLElement {
  return h('span', { class: 'chg-stat' }, h('span', { class: 'plus' }, `+${additions}`), h('span', { class: 'minus' }, `−${deletions}`));
}
