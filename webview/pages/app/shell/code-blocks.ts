/** Connects Markdown code blocks to the backend: which languages run, running them, saving them. */
import { tauriInvoke } from '../../../shared/tauri';
import { saveFile } from '../core/files';
import { codeBlocks } from '../ui/markdown';

codeBlocks.save = (name, blob) => void saveFile(name, blob);
codeBlocks.run = (language, code) => tauriInvoke<string>('chat_run_snippet', { language, code });
// Learned once, at start.
void tauriInvoke<string[]>('chat_snippet_languages')
  .then((l) => (codeBlocks.languages = l))
  .catch(() => undefined);
