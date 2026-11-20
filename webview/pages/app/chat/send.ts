/** Sending a message: one model, a comparison, or a regeneration, with the options the composer set. */
import { tauriInvoke } from '../../../shared/tauri';
import { api, newId } from '../core/api';
import { clear, h } from '../ui/h';
import { modal, toast } from '../ui/overlay';
import { changed, navigate, state, temporaryByDefault } from '../core/state';
import { openSettings } from '../settings/index';
import { modalHeader } from '../ui/ui';
import { followUps } from './queue';
import { viewCache } from './chat-view';
import { worktreeNext } from './home-view';

export async function regenerate(conversationId: string) {
  const model = state.model;
  if (!model) return void toast('Choose a model first.', 'error');
  try {
    await api.send({ conversationId, text: '', model, speed: state.speed, approval: state.approval });
  } catch (e) {
    toast(String(e), 'error');
  }
}

export async function send(text: string, conversationId: string | null, images: { mediaType: string; data: string }[] = []) {
  const model = state.model;
  if (!model) {
    toast('Add a provider and choose a model first.', 'error');
    void openSettings('providers');
    return false;
  }
  if (!conversationId && compareModels.length > 1) return sendComparison(text, images);
  const id = conversationId ?? newId();
  const projectId = state.view.kind === 'home' ? (state.view.projectId ?? null) : null;
  const agentId = !conversationId && state.view.kind === 'home' ? (state.view.agentId ?? null) : null;
  const agent = agentId ? state.agents.find((a) => a.id === agentId) : undefined;
  let workdir = !conversationId && state.mode === 'code' ? state.workdir : null;
  // A new Code session can work in its own git worktree, on a branch of its own.
  if (workdir && worktreeNext()) {
    try {
      const [folder, branch] = await tauriInvoke<[string, string]>('chat_create_worktree', { dir: workdir, name: text.split('\n')[0]!.slice(0, 60) });
      const settings = await api.appSettings();
      if (!settings.filesystem_allowed_roots.includes(folder)) settings.filesystem_allowed_roots.push(folder);
      await api.saveAppSettings(settings);
      workdir = folder;
      toast(`Working in a new worktree on ${branch}.`);
    } catch (e) {
      toast(String(e), 'error');
      return false;
    }
  }
  const temporary = !conversationId && temporaryNext;
  if (conversationId) followUps.delete(conversationId);
  if (!conversationId) {
    pendingUser.set(id, { text, images });
    navigate({ kind: 'chat', id });
  } else {
    viewCache.chat?.addOptimistic(text, images);
  }
  try {
    await api.send({
      conversationId: id,
      text,
      model: agent?.model ?? model,
      speed: agent?.think ? 'think' : state.speed,
      approval: state.approval,
      projectId,
      workdir,
      agentId,
      temporary,
      images,
      connectors: connectorChoice(),
      toolsOff: [...toolsOff],
    });
    if (temporary) temporaryChats.add(id);
    temporaryNext = temporaryByDefault();
    void refreshConversations();
  } catch (e) {
    toast(String(e), 'error');
    return false;
  }
  return true;
}

/**
 * The assistant's built-in tools, each switchable for the chats you send from the + menu's
 * Tools (as Grok and Vibe show them): image generation is one tool among the others.
 */
export const TOOL_GROUPS: { label: string; icon: string; names: string[]; hint: string }[] = [
  { label: 'Web search', icon: 'globe', names: ['web_search'], hint: 'Search the web and cite sources' },
  { label: 'Image generation', icon: 'image', names: ['generate_image'], hint: 'Create and edit images' },
  { label: 'Video generation', icon: 'video', names: ['generate_video'], hint: 'Short clips, or an image brought to life' },
  { label: 'Animations', icon: 'video', names: ['make_animation'], hint: 'Explainers, infographics and titles as code' },
  { label: 'Browser', icon: 'globe', names: ['browser_view', 'browser_act', 'browser'], hint: 'Open, read and use web pages' },
  { label: 'Canvas', icon: 'file', names: ['canvas'], hint: 'Documents, apps and sheets beside the chat' },
  { label: 'Run code', icon: 'terminal', names: ['run_code'], hint: 'Python and JavaScript in a sandbox' },
  { label: 'Sub-agents', icon: 'workflow', names: ['delegate'], hint: 'Research in parallel' },
  { label: 'Message agents', icon: 'chat', names: ['message_agent'], hint: 'Ask your other agents' },
  { label: 'Schedule tasks', icon: 'clock', names: ['schedule_task'], hint: 'Runs at set times' },
  { label: 'Knowledge', icon: 'book', names: ['remember', 'forget', 'knowledge_write'], hint: 'Save what it learns about you' },
];
export const toolsOff = new Set<string>();

/** Image templates (Grok's "Modelo" cards): what to ask first and how to draw it. */
export const IMAGE_TEMPLATES: { id: string; label: string; icon: string; brief: string }[] = [
  {
    id: 'mascot',
    label: 'Mascot maker',
    icon: 'sparkle',
    brief:
      'This is a mascot or avatar. Unless I said, first ask me with ask_user for the style (options: Polished 2D game mascot, Chibi cute, Retro cartoon, Flat vector) and then the background (options: Plain white, Transparent look, In a scene). Then design one friendly, memorable character that fits the subject, centred, full body.',
  },
  {
    id: 'logo',
    label: 'Logo',
    icon: 'star',
    brief: 'This is a logo. Unless I said, first ask me with ask_user for the style (options: Minimal wordmark, Emblem or badge, Geometric icon, Hand-drawn) and the colours. Keep it simple, readable at small sizes, on a plain background.',
  },
  {
    id: 'sticker',
    label: 'Sticker',
    icon: 'image',
    brief: 'This is a die-cut sticker: bold outlines, a white border, plain background. Unless I said, ask me with ask_user for the style (options: Kawaii, Cartoon, Pixel art, Watercolour).',
  },
  { id: 'poster', label: 'Poster', icon: 'file', brief: 'This is a poster. Unless I said, ask me with ask_user for the mood (options: Bold and modern, Vintage, Minimal, Playful) and any text to show; keep text short and spelled exactly.' },
  {
    id: 'product',
    label: 'Product photo',
    icon: 'image',
    brief: 'This is a product photo. Unless I said, ask me with ask_user for the setting (options: White studio, Lifestyle scene, Flat lay, Dramatic dark). Realistic lighting, sharp focus on the product.',
  },
];

/** The message for image mode: use the image tool, with the chosen shape, count and template. */
export function imageDirective(img: { shape: string; count: number; template: string | null }, text: string): string {
  const t = IMAGE_TEMPLATES.find((x) => x.id === img.template);
  const what = img.count > 1 ? `${img.count} images (variations)` : 'an image';
  return `Create ${what} with generate_image (shape: ${img.shape}${img.count > 1 ? `, count: ${img.count}` : ''}).${t ? ` ${t.brief}` : ''}\n\n${text || 'Surprise me.'}`;
}

/** Connectors switched off from the + menu; every ready one is on unless listed here. */
export const connectorsOff = new Set<string>();
export let knownConnectors: string[] = [];
export function setKnownConnectors(ids: string[]) {
  knownConnectors = ids;
}

/** The connectors a turn may use, or null for all of them. */
function connectorChoice(): string[] | null {
  return connectorsOff.size ? knownConnectors.filter((c) => !connectorsOff.has(c)) : null;
}

/** Models a new chat is sent to side by side; comparing when there are two or more. */
export let compareModels: string[] = [];
export function setCompareModels(models: string[]) {
  compareModels = models;
}

/** Sends one prompt to every compared model, each in a temporary conversation of its own. */
async function sendComparison(text: string, images: { mediaType: string; data: string }[]) {
  const projectId = state.view.kind === 'home' ? (state.view.projectId ?? null) : null;
  const columns = compareModels.map((model) => ({ id: newId(), model }));
  navigate({ kind: 'compare', prompt: text, columns });
  const results = await Promise.allSettled(columns.map((c) => api.send({ conversationId: c.id, text, model: c.model, speed: state.speed, approval: state.approval, projectId, temporary: true, images })));
  const failed = results.find((r): r is PromiseRejectedResult => r.status === 'rejected');
  if (failed) toast(String(failed.reason), 'error');
  return results.some((r) => r.status === 'fulfilled');
}

export function modelName(reference: string): string {
  return state.models.find((m) => m.reference === reference)?.name ?? reference.split('/').pop() ?? reference;
}

/** Picks the two or three models a new chat is compared across. */
export function openCompareDialog(onDone: () => void) {
  void modal((close) => {
    const chosen = new Set(compareModels.length ? compareModels : state.model ? [state.model] : []);
    const list = h('div', { class: 'compare-pick' });
    const render = () =>
      clear(
        list,
        state.models.map((m) => {
          const box = h('input', { type: 'checkbox', checked: chosen.has(m.reference), disabled: !chosen.has(m.reference) && chosen.size >= 3 });
          box.addEventListener('change', () => {
            if (box.checked) chosen.add(m.reference);
            else chosen.delete(m.reference);
            render();
          });
          return h('label', { class: 'compare-option' }, box, h('span', { class: 'model-name' }, m.name), h('span', { class: 'muted' }, m.providerLabel));
        })
      );
    render();
    const start = () => {
      if (chosen.size < 2) return void toast('Choose at least two models.', 'error');
      compareModels = [...chosen];
      close();
      onDone();
    };
    return h(
      'div',
      { class: 'dialog-small' },
      modalHeader('Compare models', close, { flush: true }),
      h('p', { class: 'muted' }, 'Your next message goes to each model side by side. Keep the answer you like best and carry on with that model.'),
      list,
      h('div', { class: 'dialog-actions' }, h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'), h('button', { class: 'btn btn-primary', on: { click: start } }, 'Compare'))
    );
  });
}

/** The next new conversation is temporary: kept out of the history and deleted on restart. */
export let temporaryNext = temporaryByDefault();
export function setTemporaryNext(on: boolean) {
  temporaryNext = on;
}
export const temporaryChats = new Set<string>();

/** Messages typed on the home screen, shown before the new conversation is on disk. */
export const pendingUser = new Map<string, { text: string; images: { mediaType: string; data: string }[] }>();

export async function refreshConversations() {
  state.conversations = await api.conversations();
  changed();
}
