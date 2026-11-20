/** The home screen: greeting, suggestions, the connect banner and the folder for code mode. */
import { api } from '../core/api';
import { agentPanel } from '../pages/agentpanel';
import { clear, h } from '../ui/h';
import { icon } from '../ui/icons';
import { greeting } from '../core/clock';
import { navigate, state } from '../core/state';
import { openSettings } from '../settings/index';
import { button } from '../ui/ui';
import { flag, textPref, workdir } from '../core/prefs';
import { viewCache } from './chat-view';
import { temporaryNext, setTemporaryNext } from './send';
import { composer } from './composer/index';

const SUGGESTIONS: { icon: string; text: string; page?: boolean; more?: boolean; image?: boolean }[] = [
  { icon: 'image', text: 'Create an image', image: true },
  { icon: 'skill', text: 'Set up and discover what this assistant can do' },
  { icon: 'globe', text: 'What is the current state of the AI industry?' },
  { icon: 'plug', text: 'Connect your apps for better answers', page: true },
  { icon: 'sparkle', text: 'Explain a concept like I am new to it', more: true },
  { icon: 'file', text: 'Look through my documents folder and tell me what is there', more: true },
  { icon: 'terminal', text: 'Analyze a CSV file and chart what matters', more: true },
  { icon: 'apps', text: 'Build me a small app: a pomodoro timer', more: true },
  { icon: 'clock', text: 'Every weekday at 9:00, brief me on AI news', more: true },
];

/** Connectors the banner above the composer suggests, shown in turn. */
const BANNER_APPS = ['M', 'S', 'G', 'N'];
const connectBanner = textPref('connectBannerDismissed');
let bannerDismissed = connectBanner.get() === '1';
let showMoreSuggestions = false;

export class HomeView {
  readonly el = h('div', { class: 'home' });
  readonly key: string;
  constructor(
    private projectId: string | null,
    private agentId: string | null = null
  ) {
    this.key = `home:${projectId ?? ''}:${agentId ?? ''}:${state.mode}`;
    this.build();
  }

  build() {
    this.noModels = state.models.length === 0;
    const project = state.projects.find((p) => p.id === this.projectId);
    const agent = state.agents.find((a) => a.id === this.agentId);
    const code = state.mode === 'code' && !agent;
    const title = code
      ? h(
          'h1',
          { class: 'greeting' },
          'Ready for a session in ',
          h('button', { class: 'folder-pick', on: { click: () => void pickWorkdir() } }, icon('folder', 26), state.workdir ? (state.workdir.split(/[\\/]/).pop() ?? state.workdir) : 'Choose a folder'),
          '?'
        )
      : agent
        ? h('h1', { class: 'greeting' }, `What should ${agent.name} do?`)
        : h('h1', { class: 'greeting' }, project ? `${project.icon ? `${project.icon} ` : ''}New chat in ${project.name}` : greeting(state.instructions.name));
    // Code sessions can each get their own git worktree (Vibe's worktrees).
    const worktreeToggle =
      code && state.workdir
        ? h(
            'label',
            { class: 'worktree-toggle', title: 'The session works in a git worktree beside the repository, on a branch of its own, so the main checkout stays as it is' },
            (() => {
              const box = h('input', { type: 'checkbox', checked: worktreeNext() });
              box.addEventListener('change', () => setWorktreeNext(box.checked));
              return box;
            })(),
            h('span', {}, 'In a new worktree, on its own branch')
          )
        : null;
    composer.attach(null, code ? 'What do you want to build?' : 'Type / for quick access');
    const dismiss = h('button', { class: 'icon-btn small', title: 'Dismiss' }, icon('close', 16));
    dismiss.addEventListener('click', () => {
      bannerDismissed = true;
      connectBanner.set('1');
      this.build();
    });
    const banner =
      state.models.length === 0
        ? h('div', { class: 'composer-banner' }, icon('key', 16), h('span', {}, 'Add an API key or a local server to start chatting'), button({ variant: 'link', label: 'Add provider', onClick: () => void openSettings('providers') }))
        : !code && !agent && !bannerDismissed
          ? h(
              'div',
              { class: 'composer-banner connect' },
              h('span', { class: 'banner-app' }, BANNER_APPS[new Date().getMinutes() % BANNER_APPS.length]),
              h('span', { class: 'grow' }, 'Work mode is better with your apps connected'),
              button({ variant: 'link', label: 'Connect', class: 'strong', onClick: () => navigate({ kind: 'page', page: 'connectors' }) }),
              dismiss
            )
          : null;
    // An agent's own conversation starters: one click asks it.
    const starters = agent?.starters?.length
      ? h(
          'div',
          { class: 'suggestions' },
          agent.starters.map((text) =>
            h(
              'button',
              {
                class: 'suggestion',
                on: {
                  click: () => {
                    composer.setText(text);
                    void composer.submit();
                  },
                },
              },
              icon('sparkle', 17),
              text
            )
          )
        )
      : null;
    const suggestions =
      code || agent
        ? starters
        : h(
            'div',
            { class: 'suggestions' },
            SUGGESTIONS.filter((s) => showMoreSuggestions || !s.more).map((s) =>
              h(
                'button',
                {
                  class: 'suggestion',
                  on: {
                    click: () => (s.page ? navigate({ kind: 'page', page: 'connectors' }) : s.image ? (composer.setImage({ shape: 'square', count: 1, template: null }), composer.focus()) : (composer.setText(s.text), composer.focus())),
                  },
                },
                icon(s.icon, 17),
                s.text.replace(/: $/, '')
              )
            ),
            showMoreSuggestions
              ? null
              : h(
                  'button',
                  {
                    class: 'suggestion',
                    on: {
                      click: () => {
                        showMoreSuggestions = true;
                        this.build();
                      },
                    },
                  },
                  icon('more', 17),
                  'Discover more'
                )
          );
    const ghost = h(
      'button',
      { class: `temp-toggle icon-only${temporaryNext ? ' on' : ''}`, title: temporaryNext ? 'Temporary chat: on (not saved to history)' : 'Temporary chat' },
      icon('chatDashed', 20),
      temporaryNext ? h('span', {}, 'Temporary') : null
    );
    ghost.addEventListener('click', () => {
      setTemporaryNext(!temporaryNext);
      this.build();
    });
    clear(
      this.el,
      code ? null : ghost,
      h(
        'div',
        { class: 'home-inner' },
        agent ? h('span', { class: 'agent-avatar large' }, agent.avatar || agent.name.charAt(0)) : h('div', { class: 'brand-mark' }),
        title,
        h('div', { class: 'composer-wrap' }, banner, composer.el),
        worktreeToggle,
        suggestions,
        agent ? agentPanel(agent.id) : null
      )
    );
    setTimeout(() => composer.focus());
  }

  update() {
    // The models load after the first render; redraw once whether a provider exists is known.
    if ((state.models.length === 0) !== this.noModels) this.build();
    composer.renderBar();
  }

  private noModels = state.models.length === 0;
}

/**
 * A hue for an avatar: emoji with a colour of their own get it (🍊 orange, 🌿 green…), anything
 * else a steady hue from its characters, so each agent keeps its colour.
 */
export function avatarHue(avatar: string): number {
  const known: [RegExp, number][] = [
    [/[🍎🍓🍒🌹❤️🔥🚒🐞🦀]/u, 4],
    [/[🍊🧡🦊🎃🏀🥕]/u, 28],
    [/[🍋🌻⭐💛🐝🌟☀️]/u, 48],
    [/[🌿🍀🌱💚🐸🥝🌲🌵]/u, 135],
    [/[🐳💧🌊💙🧊🦋🫐]/u, 205],
    [/[🍇🔮💜🦄☂️]/u, 275],
    [/[🌸🎀🩷🐷🌷]/u, 330],
  ];
  for (const [re, hue] of known) if (re.test(avatar)) return hue;
  let hash = 0;
  for (const ch of avatar) hash = (hash * 31 + ch.codePointAt(0)!) >>> 0;
  return hash % 360;
}

/** Whether the next Code session starts in a new git worktree. */
export const worktree = flag('cortex.worktree', false);

export function worktreeNext(): boolean {
  return worktree.get();
}

function setWorktreeNext(on: boolean) {
  worktree.set(on);
}

async function pickWorkdir() {
  const dir = await api.pickDirectory();
  if (!dir) return;
  const settings = await api.appSettings();
  if (!settings.filesystem_allowed_roots.includes(dir)) settings.filesystem_allowed_roots.push(dir);
  settings.filesystem_write_enabled = true;
  settings.bash_enabled = true;
  await api.saveAppSettings(settings);
  state.workdir = dir;
  workdir.set(dir);
  viewCache.home?.build();
}
