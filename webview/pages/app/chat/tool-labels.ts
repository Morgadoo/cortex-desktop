/** How a tool call reads in the transcript: its label, icon and the hint from its arguments. */
export const CONNECTOR_LABELS: Record<string, string> = {
  filesystem: 'Files',
  bash: 'Terminal',
  sandbox: 'Code',
  terminal: 'Terminal',
  browser: 'Web',
  proton_mail: 'Mail',
  luminance: 'Luminance',
  tool_results: 'Saved output',
};

export const CONNECTOR_ICONS: Record<string, string> = {
  filesystem: 'file',
  bash: 'terminal',
  sandbox: 'terminal',
  terminal: 'terminal',
  browser: 'globe',
};

/** Past-tense labels for the built-in tools, as a timeline reads them. */
const ACTIONS: Record<string, string> = {
  find_tools: 'Found tools',
  filesystem__fs_search: 'Searched files',
  filesystem__fs_read: 'Read a file',
  filesystem__fs_write: 'Wrote a file',
  filesystem__fs_edit: 'Edited a file',
  bash__run: 'Ran a command',
  browser__fetch: 'Fetched a page',
  browser__navigate: 'Browsed a page',
  tool_results__read: 'Read saved output',
  web_search: 'Searched the web',
  remember: 'Saved to knowledge',
  forget: 'Removed from knowledge',
  use_skill: 'Used a skill',
  update_plan: 'Updated the plan',
  knowledge_write: 'Saved to knowledge',
  terminal_start: 'Started a background command',
  terminal_read: 'Read command output',
  terminal_stop: 'Stopped a command',
  knowledge_read: 'Read a knowledge page',
  knowledge_search: 'Searched knowledge',
  save_skill: 'Saved a skill',
  canvas: 'Wrote a canvas',
  delegate: 'Ran sub-agents',
  manage_agents: 'Managed agents',
  schedule_task: 'Scheduled a task',
  search_library: 'Searched your libraries',
};

const BUILTIN_ICONS: Record<string, string> = {
  find_tools: 'tool',
  web_search: 'search',
  remember: 'book',
  forget: 'book',
  use_skill: 'skill',
  update_plan: 'check',
  canvas: 'file',
  delegate: 'workflow',
  manage_agents: 'workflow',
  schedule_task: 'clock',
  search_library: 'library',
  run_code: 'terminal',
  knowledge_write: 'book',
  terminal_start: 'terminal',
  terminal_read: 'terminal',
  terminal_stop: 'terminal',
  knowledge_read: 'book',
  knowledge_search: 'book',
  save_skill: 'skill',
  generate_image: 'sparkle',
  generate_video: 'video',
  make_animation: 'video',
  browser_view: 'globe',
  browser_act: 'globe',
};

/** What an approval card asks to allow. */
export const REQUESTS: Record<string, string> = {
  filesystem__fs_write: 'writing a file',
  filesystem__fs_edit: 'editing a file',
  bash__run: 'running a command',
  sandbox__run_code: 'running this code',
  terminal__start: 'starting a background command',
  browser__fetch: 'sending a web request',
  browser__navigate: 'opening a page',
};

export function humanize(name: string): string {
  const words = name.replace(/[_-]+/g, ' ').trim();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

export function toolLabel(name: string): { connector: string; action: string; icon: string } {
  if (name === 'ask_user') return { connector: '', action: 'Asked you', icon: 'chat' };
  if (BUILTIN_ICONS[name]) return { connector: '', action: ACTIONS[name] ?? humanize(name), icon: BUILTIN_ICONS[name]! };
  const [connector = '', tool = name] = name.includes('__') ? name.split(/__(.*)/s) : ['', name];
  const label = CONNECTOR_LABELS[connector] ?? humanize(connector.replace(/^mcp-/, ''));
  const known = ACTIONS[name];
  return { connector: known ? '' : label, action: known ?? humanize(tool), icon: CONNECTOR_ICONS[connector] ?? 'tool' };
}

/** The argument worth showing next to a tool step: a path, a URL, a query or a command. */
export function argumentHint(args: Record<string, unknown>): string {
  for (const key of ['path', 'url', 'query', 'fact', 'command', 'script', 'pattern', 'question', 'name']) {
    const value = args[key];
    // A file is known by its name; the folder is in the step's details.
    if (key === 'path' && typeof value === 'string' && value) return value.split(/[\\/]/).filter(Boolean).pop() ?? value;
    if (typeof value === 'string' && value) return value;
    if (key === 'script' && value && typeof value === 'object') {
      const program = (value as { program?: unknown }).program;
      if (typeof program === 'string') return program;
    }
  }
  return '';
}
