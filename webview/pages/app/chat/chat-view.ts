/** The conversation view: its transcript, live turn, usage meter and side panel. */
import type { ChatEvent, UsageTotals } from '../core/api';
import { openOutputsPanel } from '../canvas/outputs';
import { applyEvent, followStreamOn } from '../core/state';
import { closePanel, enableAskAboutSelection, setPanelHost } from '../canvas/canvas';
import { escapeHtml } from '../core/format';
import { loadChanges, openChanges, openFiles, openDebug } from './changes';
import { openTerminal } from './terminal';
import { call, shown } from '../core/call';
import { followUpRow } from './queue';
import { conversationMenu } from './conversation-menu';
import { planOf, renderChecklist, renderUserTurn, textOf } from './transcript';
import { bookmarked, renderAssistantTurn, scrollToTurn, sourcesOf, versionSwitcher } from './turn-actions';
import { autoPlay } from './speech';
import { renderLiveTurn } from './live-turn';
import { pendingUser, refreshConversations, temporaryChats } from './send';
import { userParts } from './composer/composer';
import { composer } from './composer/index';
import { renderApprovals, renderQuestion } from './approvals';
import { avatarHue } from './home-view';
import { tauriInvoke } from '../../../shared/tauri';
import { api, type Conversation, type Part } from '../core/api';
import { actionFor } from '../core/shortcuts';
import { clear, h } from '../ui/h';
import { icon } from '../ui/icons';
import { markdown } from '../ui/markdown';
import { popover, toast } from '../ui/overlay';
import { changed, navigate, state } from '../core/state';
import { hostOf } from '../ui/favicon';
import { trigger } from '../ui/ui';
import { showContext } from './chat-settings';
import type { HomeView } from './home-view';

export class ChatView {
  readonly el = h('div', { class: 'chat' });
  readonly key: string;
  private conversation: Conversation | null = null;
  private header = h('header', { class: 'chat-header' });
  private scroller = h('div', { class: 'chat-scroll' });
  private transcript = h('div', { class: 'transcript' });
  private liveSlot = h('div', {});
  private overlay = h('div', { class: 'composer-overlay' });
  private bottom = h('div', { class: 'chat-bottom' });
  private jump = h('button', { class: 'jump-btn', title: 'Scroll to bottom' }, icon('arrowDown', 18));
  /** The prompt of the part being read, pinned at the top once it scrolls away (Vibe's sticky prompt). */
  private sticky = h('button', { class: 'sticky-prompt' });
  private stickyTarget: HTMLElement | null = null;
  private planPanel = h('aside', { class: 'plan-panel' });
  private sidePanel = h('aside', { class: 'side-panel' });
  /** Files changed in the session's folder: undefined until asked, null while asking, -1 without git. */
  private changeCount: number | null | undefined = undefined;
  private stickToBottom = true;

  constructor(private id: string) {
    this.key = `chat:${id}`;
    this.scroller.append(this.sticky, h('div', { class: 'chat-column' }, this.transcript, this.liveSlot));
    this.sticky.addEventListener('click', () => this.stickyTarget?.scrollIntoView({ behavior: 'smooth', block: 'start' }));
    this.scroller.addEventListener('scroll', () => this.updateSticky());
    // Ctrl+O folds or unfolds every activity group at once.
    this.el.addEventListener('keydown', (e) => {
      if (actionFor(e) === 'toggleSteps') {
        e.preventDefault();
        const groups = [...this.transcript.querySelectorAll<HTMLDetailsElement>('details.activity'), ...this.liveSlot.querySelectorAll<HTMLDetailsElement>('details.activity')];
        const open = groups.some((g) => !g.open);
        groups.forEach((g) => (g.open = open));
      }
    });
    this.scroller.addEventListener('scroll', () => {
      const distance = this.scroller.scrollHeight - this.scroller.scrollTop - this.scroller.clientHeight;
      this.stickToBottom = distance < 80;
      this.jump.classList.toggle('visible', distance > 300);
    });
    this.jump.addEventListener('click', () => this.scrollToBottom(true));
    enableAskAboutSelection(this.scroller, (quote) => composer.setQuote(quote));
    // Coding sessions read like Vibe Code: "Send a message".
    composer.attach(id, state.conversations.find((c) => c.id === id)?.workdir ? 'Send a message' : 'Type / for quick access');
    this.bottom.append(this.jump, this.overlay, composer.el, h('div', { class: 'disclaimer' }, 'AI can make mistakes. Check answers.'));
    this.el.append(this.header, this.scroller, this.bottom, this.planPanel, this.sidePanel);
    // Apps open beside the conversation; Escape closes the panel.
    setPanelHost((content) => {
      clear(this.sidePanel, content);
      this.el.classList.toggle('with-panel', content !== null);
    });
    this.el.addEventListener('keydown', (e) => {
      if (e.key === 'Escape' && this.el.classList.contains('with-panel') && !document.querySelector('.modal-backdrop')) closePanel();
    });
    void this.refresh(true);
  }

  /** An agent's chat takes a colour from its avatar (OpenBot's bubble colours). */
  private tintForAgent() {
    const agent = this.conversation?.agentId ? state.agents.find((a) => a.id === this.conversation!.agentId) : undefined;
    this.el.classList.toggle('agent-tinted', Boolean(agent));
    if (agent) this.el.style.setProperty('--agent-hue', String(avatarHue(agent.avatar || agent.name)));
    else this.el.style.removeProperty('--agent-hue');
  }

  async refresh(scroll: boolean) {
    try {
      this.conversation = await api.conversation(this.id);
      // A turn running since before this window opened (or reloaded): what it streamed so far.
      if (this.conversation.running && !state.live.get(this.id)?.seenStart) {
        const events = await tauriInvoke<ChatEvent[]>('chat_live_events', { conversationId: this.id }).catch(() => [] as ChatEvent[]);
        // The replay holds everything so far, including what already arrived here: start over from it.
        if (events.length) {
          state.live.delete(this.id);
          for (const e of events) applyEvent(e);
        }
      }
      bookmarked.set(this.id, new Set(this.conversation.bookmarks ?? []));
      pendingUser.delete(this.id);
      // Opening a conversation that finished in the background marks it read.
      const summary = state.conversations.find((c) => c.id === this.id);
      if (summary?.unread && !state.live.has(this.id)) {
        summary.unread = false;
        void api.updateConversation({ id: this.id, unread: false }).then(() => changed());
      }
    } catch {
      this.conversation = {
        id: this.id,
        title: '',
        projectId: null,
        workdir: null,
        model: null,
        pinned: false,
        turns: [],
        running: true,
        usage: { inputTokens: 0, outputTokens: 0, cost: 0, contextTokens: 0 },
        compactedBefore: null,
        agentId: null,
        archived: false,
        params: { temperature: null, maxTokens: null, instructions: '' },
        bookmarks: [],
        versions: [],
      };
    }
    const optimistic = pendingUser.get(this.id);
    if (optimistic && this.conversation.turns.length === 0) {
      this.conversation.turns.push({ role: 'user', parts: userParts(optimistic.text, optimistic.images), index: -1, time: new Date().toISOString(), rating: 0 });
    }
    this.tintForAgent();
    this.renderTranscript();
    this.update();
    if (scroll) this.scrollToBottom(false);
  }

  addOptimistic(text: string, images: { mediaType: string; data: string }[] = []) {
    this.conversation?.turns.push({ role: 'user', parts: userParts(text, images), index: -1, time: new Date().toISOString(), rating: 0 });
    this.renderTranscript();
    this.scrollToBottom(true);
  }

  renderTranscript() {
    this.changeCount = undefined;
    const c = this.conversation;
    if (!c) return;
    const running = state.live.has(this.id);
    const last = c.turns.length - 1;
    const divider = c.compactedBefore;
    let marked = false;
    clear(
      this.transcript,
      c.turns.flatMap((turn, i) => {
        // The branch switcher sits on the turn that varies: an edited message, or a regenerated answer.
        const previous = c.turns[i - 1];
        const v = c.versions.find(
          (b) => (turn.role === 'user' && b.kind === 'prompt' && turn.index === b.from) || (turn.role === 'assistant' && b.kind === 'answer' && previous?.role === 'user' && (previous.index === b.from || previous.index + 1 === b.from))
        );
        const branch = v ? versionSwitcher(c.id, v) : null;
        const el = turn.role === 'user' ? renderUserTurn(turn, c.id, branch) : renderAssistantTurn(c, turn, i === last, running, branch);
        // Turns before the summary point reach the model only through the summary.
        if (divider !== null && divider !== undefined && !marked && turn.role === 'user' && turn.index >= divider) {
          marked = true;
          return [h('div', { class: 'compaction-divider' }, h('span', {}, icon('book', 14), 'Earlier messages are summarized for the model')), el];
        }
        return [el];
      }),
      !running && c.turns[last]?.role === 'assistant' ? followUpRow(c.id) : null
    );
  }

  private renderHeader() {
    const c = this.conversation;
    const summary = state.conversations.find((s) => s.id === this.id);
    const title = summary?.title || c?.title || 'New chat';
    const project = state.projects.find((p) => p.id === (summary?.projectId ?? c?.projectId));
    const workdir = summary?.workdir ?? c?.workdir;
    const titleBtn = h('button', { class: 'crumb-title' }, title, icon('chevron', 14));
    titleBtn.addEventListener('click', () => conversationMenu(titleBtn, this.id, title, summary?.pinned ?? false));
    const pin = h('button', { class: `icon-btn${summary?.pinned ? ' active' : ''}`, title: summary?.pinned ? 'Unpin' : 'Pin' }, icon('pin', 18));
    pin.addEventListener('click', async () => {
      await api.updateConversation({ id: this.id, pinned: !summary?.pinned });
      await refreshConversations();
    });
    const meter = usageMeter(this.id, state.usage.get(this.id), c?.usage);
    // The chat at a glance (Open WebUI's chat overview): every message you sent, where earlier
    // turns were summarized for the model, and a jump to each.
    const prompts = (c?.turns ?? []).filter((t) => t.role === 'user' && t.index >= 0);
    const outlineBtn = prompts.length > 2 ? h('button', { class: 'icon-btn', title: 'Outline of this chat' }, icon('list', 18)) : null;
    outlineBtn?.addEventListener('click', () => {
      const cut = c?.compactedBefore ?? null;
      const items: HTMLElement[] = [];
      let marked = false;
      prompts.forEach((t, i) => {
        if (cut !== null && !marked && t.index >= cut) {
          marked = true;
          items.push(h('div', { class: 'menu-note muted' }, '↑ summarized for the model'));
        }
        const text = textOf(t.parts)
          .replace(/<attachment[\s\S]*?<\/attachment>/g, '')
          .replace(/\s+/g, ' ')
          .trim();
        items.push(
          h(
            'button',
            {
              class: 'menu-item outline-item',
              on: {
                click: () => {
                  close();
                  scrollToTurn(t.index);
                },
              },
            },
            h('span', { class: 'outline-n' }, String(i + 1)),
            h('span', {}, text.length > 90 ? `${text.slice(0, 90)}…` : text || '(attachments)')
          )
        );
      });
      const close = popover(outlineBtn, h('div', { class: 'menu outline-menu' }, h('div', { class: 'menu-heading' }, `${prompts.length} messages`), items));
    });
    // Bookmarked messages of this chat, to jump to.
    const marks = summary?.bookmarks ?? [];
    const marksBtn = marks.length ? h('button', { class: 'icon-btn', title: `Bookmarks (${marks.length})` }, icon('bookmark', 18), h('span', { class: 'badge-count' }, String(marks.length))) : null;
    marksBtn?.addEventListener('click', () => {
      const close = popover(
        marksBtn,
        h(
          'div',
          { class: 'menu bookmarks-menu' },
          h('div', { class: 'menu-heading' }, 'Bookmarks'),
          marks.map(([index, snippet]) =>
            h(
              'button',
              {
                class: 'menu-item rich',
                on: {
                  click: () => {
                    close();
                    if (!scrollToTurn(index)) toast('That message is in another version of this chat.');
                  },
                },
              },
              icon('bookmark', 15),
              h('span', {}, snippet)
            )
          )
        )
      );
    });
    const exportBtn = h('button', { class: 'icon-btn', title: 'Share or export' }, icon('share', 18));
    let changesBtn: HTMLElement | null = null;
    let terminalBtn: HTMLElement | null = null;
    let filesBtn: HTMLElement | null = null;
    let debugBtn: HTMLElement | null = null;
    if (workdir) {
      if (this.changeCount === undefined) {
        this.changeCount = null;
        void loadChanges(this.id)
          .then((c) => (this.changeCount = c.git ? c.files.length : -1))
          .catch(() => (this.changeCount = -1))
          .then(() => this.renderHeader());
      }
      debugBtn = trigger({ icon: 'tool', label: 'Debug', title: "The session's raw history and usage", chevron: false, onClick: () => openDebug(this.id) });
      filesBtn = trigger({ icon: 'folder', label: 'Files', title: 'Browse the files in this folder', chevron: false, onClick: () => openFiles(this.id, workdir.split(/[\\/]/).pop() ?? workdir) });
      if (this.changeCount !== -1) {
        terminalBtn = trigger({ icon: 'terminal', label: 'Terminal', title: 'Background commands in this folder', chevron: false, onClick: () => openTerminal(this.id, workdir.split(/[\\/]/).pop() ?? workdir) });
        changesBtn = trigger({
          icon: 'file',
          label: 'Changes',
          title: 'Uncommitted changes in this folder',
          chevron: false,
          class: 'changes-btn',
          trailing: this.changeCount ? h('span', { class: 'count-badge' }, String(this.changeCount)) : null,
        });
        changesBtn.addEventListener('click', () => openChanges(this.id, workdir.split(/[\\/]/).pop() ?? workdir));
      }
    }
    exportBtn.addEventListener('click', () => {
      const item = (ic: string, label: string, hint: string, run: () => void) =>
        h(
          'button',
          {
            class: 'menu-item rich',
            on: {
              click: () => {
                close();
                run();
              },
            },
          },
          icon(ic, 17),
          h('span', {}, h('strong', {}, label), h('small', {}, hint))
        );
      const close = popover(
        exportBtn,
        h(
          'div',
          { class: 'menu' },
          item('link', 'Share a link', 'Anyone on your network with the link can open it', () => {
            const shared = this.sharePage();
            if (shared) void import('../canvas/share').then((s) => s.shareLink(shared.title, shared.page));
          }),
          item('globe', 'Share as web page', 'One file anyone can open in a browser', () => void this.shareAsPage()),
          item('download', 'Export as Markdown', 'The text, for notes and documents', () => void api.exportConversation(this.id).then((saved) => saved && toast('Conversation exported.')))
        )
      );
    });
    clear(
      this.header,
      h(
        'div',
        { class: 'crumbs' },
        temporaryChats.has(this.id) ? h('span', { class: 'temp-badge' }, icon('wait', 14), 'Temporary') : null,
        project ? h('button', { class: 'crumb', on: { click: () => navigate({ kind: 'home', projectId: project.id }) } }, icon('folder', 15), project.name) : null,
        workdir ? h('span', { class: 'crumb' }, icon('terminal', 15), workdir.split(/[\\/]/).pop() ?? workdir) : null,
        project || workdir ? h('span', { class: 'crumb-sep' }, '/') : null,
        titleBtn
      ),
      h(
        'div',
        { class: 'header-actions' },
        filesBtn,
        terminalBtn,
        changesBtn,
        debugBtn,
        meter,
        outlineBtn,
        marksBtn,
        h('button', { class: 'icon-btn', title: 'Outputs: what this chat made', on: { click: () => void openOutputsPanel(this.id) } }, icon('outputs', 17)),
        pin,
        exportBtn
      )
    );
  }

  update() {
    this.renderHeader();
    const live = state.live.get(this.id);
    clear(this.liveSlot, live ? renderLiveTurn(this.id, live) : null);
    // While the model works through a plan, the plan stays in view beside the conversation.
    const plan = live ? [...live.parts].reverse().find((p): p is Extract<Part, { kind: 'tool' }> => p.kind === 'tool' && p.name === 'update_plan') : undefined;
    const showPlan = Boolean(plan && !live?.error);
    clear(this.planPanel, showPlan && plan ? [h('div', { class: 'plan-head' }, icon('check', 15), 'Plan'), renderChecklist(planOf(plan))] : null);
    this.planPanel.classList.toggle('visible', showPlan);
    this.el.classList.toggle('with-plan', showPlan);
    const approval = renderApprovals();
    const question = live?.question ? renderQuestion(this.id, live.question) : null;
    clear(this.overlay, approval, question);
    composer.el.classList.toggle('hidden', Boolean(question));
    composer.renderBar();
    if (this.stickToBottom && followStreamOn()) this.scrollToBottom(false);
  }

  private updateSticky() {
    const top = this.scroller.getBoundingClientRect().top;
    const prompts = [...this.transcript.querySelectorAll<HTMLElement>('.turn-user')];
    // The last prompt that has scrolled fully out of view, while its answer is still on screen.
    const passed = prompts.filter((p) => p.getBoundingClientRect().bottom < top + 4);
    const current = passed[passed.length - 1] ?? null;
    const next = current ? prompts[prompts.indexOf(current) + 1] : undefined;
    const answerVisible = !next || next.getBoundingClientRect().top > top + 60;
    const show = Boolean(current && answerVisible);
    this.sticky.classList.toggle('visible', show);
    if (show && current !== this.stickyTarget) {
      this.stickyTarget = current;
      // The message itself: not its file chips or the Show more button.
      const text = (current!.querySelector('.bubble-text') ?? current!.querySelector('.bubble'))?.textContent?.trim() ?? '';
      clear(this.sticky, h('span', {}, text.length > 160 ? `${text.slice(0, 160)}…` : text));
    }
  }

  /**
   * The conversation as one self-contained web page: what was asked and answered, rendered,
   * with code, citations and sources, readable in light or dark, loading nothing from elsewhere.
   */
  /** The conversation as one self-contained page. */
  private sharePage(): { title: string; page: string } | null {
    const c = this.conversation;
    if (!c) return null;
    const title = this.header.querySelector('.crumb-title')?.textContent?.trim() || 'Conversation';
    const body = document.createElement('div');
    for (const turn of c.turns) {
      if (turn.role === 'user') {
        body.append(h('section', { class: 'you' }, h('div', { class: 'who' }, 'You'), h('div', { class: 'bubble' }, textOf(turn.parts))));
        continue;
      }
      const answer = h('div', { class: 'markdown' });
      for (const part of turn.parts) if (part.kind === 'text') answer.append(markdown(part.text));
      // Frames, buttons and hover-only bits do not travel; links and text do.
      answer.querySelectorAll('iframe, button, .code-head, .preview-head, script, .cite-icon, .site-icon').forEach((el) => el.remove());
      const sources = sourcesOf(turn.parts).filter((s) => s.cited);
      body.append(
        h(
          'section',
          { class: 'assistant' },
          h('div', { class: 'who' }, 'Assistant'),
          answer,
          sources.length
            ? h(
                'div',
                { class: 'sources' },
                h('strong', {}, 'Sources'),
                h(
                  'ol',
                  {},
                  sources.map((s) => h('li', {}, h('a', { href: s.url }, s.title), ` · ${hostOf(s.url)}`))
                )
              )
            : null
        )
      );
    }
    const esc = escapeHtml;
    const page = `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'">
<title>${esc(title)}</title>
<style>
:root { --bg: #fff; --fg: #1d1d1f; --muted: #6b6b70; --card: #f3f3f5; --border: #e3e3e6; --accent: #6d4aff; color-scheme: light dark; }
@media (prefers-color-scheme: dark) { :root { --bg: #1e1e20; --fg: #ececee; --muted: #9a9aa0; --card: #2a2a2d; --border: #38383c; --accent: #9b84ff; } }
body { margin: 0; background: var(--bg); color: var(--fg); font: 16px/1.6 system-ui, -apple-system, "Segoe UI", sans-serif; }
main { max-width: 780px; margin: 0 auto; padding: 32px 16px 64px; }
h1.title { font-size: 1.5rem; margin: 0 0 4px; } .meta { color: var(--muted); font-size: .85rem; margin-bottom: 28px; }
section { margin: 0 0 28px; } .who { font-size: .75rem; font-weight: 700; text-transform: uppercase; letter-spacing: .04em; color: var(--muted); margin-bottom: 6px; }
.you .bubble { background: var(--card); border-radius: 14px; padding: 10px 14px; white-space: pre-wrap; }
a { color: var(--accent); } pre { background: var(--card); border: 1px solid var(--border); border-radius: 10px; padding: 12px; overflow-x: auto; font-size: .85rem; }
code { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; } :not(pre) > code { background: var(--card); border-radius: 4px; padding: 1px 4px; }
table { border-collapse: collapse; } th, td { border: 1px solid var(--border); padding: 4px 8px; } img { max-width: 100%; border-radius: 8px; }
.sources { font-size: .85rem; color: var(--muted); border-top: 1px solid var(--border); margin-top: 12px; padding-top: 8px; } .sources ol { margin: 4px 0 0; padding-left: 20px; }
details > summary { display: none; } .footer { color: var(--muted); font-size: .8rem; margin-top: 40px; }
</style></head><body><main>
<h1 class="title">${esc(title)}</h1><div class="meta">Shared from Cortex · ${new Date().toLocaleDateString(undefined, { year: 'numeric', month: 'long', day: 'numeric' })}</div>
${body.innerHTML}
<div class="footer">AI answers can contain mistakes. Check important information.</div>
</main></body></html>`;
    return { title, page };
  }

  private async shareAsPage() {
    const shared = this.sharePage();
    if (!shared) return;
    const { title, page } = shared;
    await call<boolean>('chat_save_shared_page', { title, html: page }).then((ok) => {
      if (ok) toast('Saved as a web page. Send the file to share the conversation.');
    }, shown);
  }

  /** The turn ended: the stored conversation replaces the live one. */
  async finished() {
    await this.refresh(false);
  }

  /** Redraws with something new under the last answer, keeping it in view if the bottom was. */
  showAddedBelow() {
    const distance = this.scroller.scrollHeight - this.scroller.scrollTop - this.scroller.clientHeight;
    this.renderTranscript();
    if (distance < 160) this.scrollToBottom(true);
  }

  /** Reads the newest answer aloud, when auto-play is on. */
  readLatestAloud() {
    const last = this.conversation?.turns[this.conversation.turns.length - 1];
    if (last?.role === 'assistant') autoPlay(textOf(last.parts));
  }

  private scrollToBottom(smooth: boolean) {
    requestAnimationFrame(() => this.scroller.scrollTo({ top: this.scroller.scrollHeight, behavior: smooth ? 'smooth' : 'auto' }));
  }
}

/** Tokens, cost and how full the model's window is, for the conversation header. */
function usageMeter(id: string, live: { usage: UsageTotals; window: number } | undefined, stored: UsageTotals | undefined): HTMLElement | null {
  const usage = live?.usage ?? stored;
  if (!usage || !(usage.inputTokens || usage.outputTokens)) return null;
  const pct = live?.window ? Math.min(100, Math.round((usage.contextTokens / live.window) * 100)) : null;
  const money = (n: number) => `$${n.toFixed(n < 0.01 ? 4 : 2)}`;
  const lines = [
    pct !== null && live ? `Context: ${usage.contextTokens.toLocaleString()} of ${live.window.toLocaleString()} tokens (${pct}%)` : '',
    `Used: ${usage.inputTokens.toLocaleString()} in, ${usage.outputTokens.toLocaleString()} out`,
    usage.cost > 0 ? `Cost: ${money(usage.cost)}` : '',
  ].filter(Boolean);
  const total = Math.round((usage.inputTokens + usage.outputTokens) / 1000);
  return h(
    'button',
    { class: 'usage-meter', title: `${lines.join('. ')}. Click to see what fills the context.`, on: { click: () => void showContext(id) } },
    pct !== null ? h('span', { class: 'ring', style: { '--pct': String(pct) } }) : null,
    h('span', {}, `${total}k tokens${usage.cost > 0 ? ` · ${money(usage.cost)}` : ''}`)
  );
}

/** The views on screen, so streamed events update them in place. */
export const viewCache: { chat: ChatView | null; home: HomeView | null } = { chat: null, home: null };
