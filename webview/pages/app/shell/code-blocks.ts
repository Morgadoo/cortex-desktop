/** Connects Markdown code blocks to the backend: which languages run, running them, saving them. */
import { invoke } from '../core/call';
import { saveFile } from '../core/files';
import { codeBlocks } from '../ui/markdown';

codeBlocks.save = (name, blob) => void saveFile(name, blob);
codeBlocks.run = (language, code) => invoke<string>('chat_run_snippet', { language, code });
// Learned once, at start.
void invoke<string[]>('chat_snippet_languages')
  .then((l) => (codeBlocks.languages = l))
  .catch(() => undefined);
