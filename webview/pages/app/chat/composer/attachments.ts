/** What goes with a message: files, images, a quote and image options, shown as chips. */
import { api } from '../../core/api';
import { clear } from '../../ui/h';
import { popover, toast } from '../../ui/overlay';
import { blobToBase64 } from '../../core/format';
import { chip, iconButton } from '../../ui/ui';
import { pausedQueues, queued, saveQueues, setQueue } from '../queue';
import { compareModels, modelName, send, toolsOff, setCompareModels } from '../send';
import { h } from '../../ui/h';
import { icon } from '../../ui/icons';
import { IMAGE_TEMPLATES } from '../send';
import { Composer, IMAGE_TYPES, readImage } from './composer';
import { call, invoke, reported } from '../../core/call';

const methods = {
  /** Turns image mode on (with the image tool) or off. */
  setImage(this: Composer, next: { shape: 'square' | 'landscape' | 'portrait'; count: number; template: string | null } | null) {
    this.image = next;
    if (next) toolsOff.delete('generate_image');
    this.textarea.placeholder = next ? 'Describe the image you want…' : this.placeholder;
    this.renderChips();
    this.renderBar();
  },

  imageOptions(this: Composer, anchor: HTMLElement) {
    if (!this.image) return;
    const img = this.image;
    let close = () => {};
    const pick = (change: Partial<typeof img>) => {
      this.setImage({ ...img, ...change });
      close();
      this.focus();
    };
    const row = (label: string, on: boolean, run: () => void, ic?: string) =>
      h('button', { class: `menu-item${on ? ' selected' : ''}`, on: { click: run } }, ic ? icon(ic, 15) : icon(on ? 'check' : 'dot', 14), h('span', { class: 'grow' }, label));
    close = popover(
      anchor,
      h(
        'div',
        { class: 'menu image-menu' },
        h('div', { class: 'menu-heading' }, 'Shape'),
        (['square', 'landscape', 'portrait'] as const).map((s) => row(s === 'square' ? 'Square 1:1' : s === 'landscape' ? 'Landscape 3:2' : 'Portrait 2:3', img.shape === s, () => pick({ shape: s }))),
        h('div', { class: 'menu-heading' }, 'How many'),
        h(
          'div',
          { class: 'count-row' },
          [1, 2, 3, 4].map((n) => h('button', { class: `count-btn${img.count === n ? ' active' : ''}`, on: { click: () => pick({ count: n }) } }, String(n)))
        ),
        h('div', { class: 'menu-heading' }, 'Template'),
        row('None: just my description', !img.template, () => pick({ template: null })),
        IMAGE_TEMPLATES.map((t) => row(t.label, img.template === t.id, () => pick({ template: t.id }), t.icon))
      ),
      'above'
    );
  },

  setQuote(this: Composer, text: string) {
    this.quote = text;
    this.renderChips();
    this.focus();
  },

  renderChips(this: Composer) {
    this.chipsDrawnRunning = this.running();
    const extra: HTMLElement[] = [];
    // Messages waiting for the running task to end, with Send now (steer) and remove.
    const waiting = this.conversationId ? (queued.get(this.conversationId) ?? []) : [];
    if (waiting.length && this.conversationId) {
      const id = this.conversationId;
      const paused = pausedQueues.has(id);
      const idle = !this.running();
      // The queue's own controls: pause or resume it, and send the next one when nothing runs
      // (after a restart, or while paused).
      extra.push(
        h(
          'div',
          { class: 'queued-head' },
          h('span', { class: 'queued-label' }, `${waiting.length} queued${paused ? ' · paused' : ''}`),
          h('span', { class: 'spacer' }),
          idle
            ? h(
                'button',
                {
                  class: 'link-btn',
                  title: 'Send the first queued message now',
                  on: {
                    click: () => {
                      // A reply started meanwhile: the queue resumes, so this goes as soon as it ends.
                      if (this.running()) {
                        pausedQueues.delete(id);
                        saveQueues();
                        this.renderChips();
                        return void toast('A reply is running; the queue resumes and this goes next.');
                      }
                      const [next, ...rest] = waiting;
                      setQueue(id, rest);
                      this.renderChips();
                      if (next) void send(next, id);
                    },
                  },
                },
                'Send next'
              )
            : null,
          h(
            'button',
            {
              class: 'link-btn',
              title: paused ? 'Send queued messages again as each turn ends' : 'Keep them waiting until you resume',
              on: {
                click: () => {
                  if (paused) pausedQueues.delete(id);
                  else pausedQueues.add(id);
                  saveQueues();
                  this.renderChips();
                },
              },
            },
            paused ? 'Resume' : 'Pause'
          ),
          h(
            'button',
            {
              class: 'link-btn',
              on: {
                click: () => {
                  setQueue(id, []);
                  this.renderChips();
                },
              },
            },
            'Clear'
          )
        )
      );
    }
    waiting.forEach((text, i) => {
      const id = this.conversationId!;
      const drop = () => {
        const list = [...(queued.get(id) ?? [])];
        list.splice(i, 1);
        setQueue(id, list);
        this.renderChips();
      };
      const move = (by: -1 | 1) => {
        const list = [...(queued.get(id) ?? [])];
        const j = i + by;
        if (j < 0 || j >= list.length) return;
        [list[i], list[j]] = [list[j]!, list[i]!];
        setQueue(id, list);
        this.renderChips();
      };
      extra.push(
        h(
          'div',
          { class: 'queued-row' },
          icon('clock', 14),
          h('span', { class: 'queued-label' }, `${i + 1}`),
          h('span', { class: 'queued-text', title: text }, text),
          waiting.length > 1 ? iconButton({ icon: 'chevronUp', title: 'Move up', size: 'xs', class: 'chip-x', onClick: () => move(-1) }) : null,
          waiting.length > 1 ? iconButton({ icon: 'chevron', title: 'Move down', size: 'xs', class: 'chip-x', onClick: () => move(1) }) : null,
          this.running()
            ? h(
                'button',
                {
                  class: 'link-btn',
                  title: 'Send it to the running task now',
                  on: {
                    click: () => {
                      drop();
                      void api.steer(id, text);
                    },
                  },
                },
                'Send now'
              )
            : null,
          h(
            'button',
            {
              class: 'link-btn',
              title: 'Edit',
              on: {
                click: () => {
                  drop();
                  this.setText(text);
                  this.focus();
                },
              },
            },
            'Edit'
          ),
          iconButton({ icon: 'close', title: 'Remove from the queue', size: 'xs', class: 'chip-x', onClick: drop })
        )
      );
    });
    if (this.quote) {
      extra.push(
        h(
          'div',
          { class: 'quote-chip' },
          icon('forward', 15),
          h('span', {}, this.quote),
          iconButton({
            icon: 'close',
            title: 'Remove quote',
            size: 'xs',
            class: 'chip-x',
            onClick: () => {
              this.quote = null;
              this.renderChips();
            },
          })
        )
      );
    }
    if (!this.conversationId && compareModels.length > 1) {
      extra.push(
        chip({
          icon: 'apps',
          tone: 'accent',
          label: `Compare · ${compareModels.map(modelName).join(' vs ')}`,
          onRemove: () => {
            setCompareModels([]);
            this.renderChips();
          },
        })
      );
    }
    const tokenChip = (ic: string, label: string, clearIt: () => void) =>
      chip({
        icon: ic,
        label,
        tone: 'accent',
        onRemove: () => {
          clearIt();
          this.renderChips();
          this.renderBar();
        },
      });
    if (this.workflow) extra.push(tokenChip('workflow', `Workflow · ${this.workflow.name}`, () => (this.workflow = null)));
    if (this.library) extra.push(tokenChip('library', `Library · ${this.library}`, () => (this.library = null)));
    if (this.folder) extra.push(tokenChip('folder', this.folder.split(/[\\/]/).pop() || this.folder, () => (this.folder = null)));
    if (this.useCanvas) {
      extra.push(
        chip({
          icon: 'file',
          tone: 'accent',
          label: 'Canvas',
          onRemove: () => {
            this.useCanvas = false;
            this.renderChips();
          },
        })
      );
    }
    if (this.image) {
      const img = this.image;
      const label = ['Image', img.template ? IMAGE_TEMPLATES.find((t) => t.id === img.template)?.label : null, img.shape !== 'square' ? (img.shape === 'landscape' ? 'Landscape' : 'Portrait') : null, img.count > 1 ? `×${img.count}` : null]
        .filter(Boolean)
        .join(' · ');
      const c = chip({
        icon: 'image',
        tone: 'accent',
        label,
        onRemove: () => {
          this.setImage(null);
          this.renderChips();
        },
      });
      c.classList.add('chip-clickable');
      c.title = 'Shape, how many, and templates';
      c.addEventListener('click', (e) => {
        if (!(e.target as HTMLElement).closest('.chip-x')) this.imageOptions(c);
      });
      extra.push(c);
    }
    clear(
      this.chips,
      extra,
      this.attachments.map((file, i) =>
        chip({
          leading: file.image ? h('img', { class: 'chip-thumb', src: `data:${file.image.mediaType};base64,${file.image.data}`, alt: '' }) : icon('file', 14),
          label: file.name,
          onRemove: () => {
            this.attachments.splice(i, 1);
            this.renderChips();
            this.renderBar();
          },
        })
      )
    );
  },

  pickFiles(this: Composer) {
    const input = h('input', { type: 'file', multiple: true, class: 'hidden' });
    // WebKitGTK only reports the choice for an input that is in the document.
    document.body.append(input);
    input.addEventListener('change', () => {
      input.remove();
      void this.addFiles(Array.from(input.files ?? []));
    });
    input.click();
  },

  /** Attaches files: images as images, documents and workbooks as their text. */
  async addFiles(this: Composer, files: File[]) {
    for (const file of files) {
      if (IMAGE_TYPES.includes(file.type)) {
        if (file.size > 8 * 1024 * 1024) {
          toast(`${file.name} is larger than 8 MB.`, 'error');
          continue;
        }
        this.attachments.push({ name: file.name, text: '', image: await readImage(file), file: true });
        continue;
      }
      // Videos: spread-out frames as images, and what is said as a transcript (ffmpeg).
      if (file.type.startsWith('video/') || /\.(mp4|mov|m4v|mkv|avi)$/i.test(file.name)) {
        try {
          if (file.size > 200 * 1024 * 1024) throw new Error(`${file.name} is larger than 200 MB.`);
          toast(`Reading ${file.name}…`);
          const data = await blobToBase64(file);
          const v = await invoke<{ seconds: number; frames: { at: number; data: string }[]; transcript: string | null; note: string | null }>('chat_read_video', { name: file.name, data });
          const clock = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`;
          for (const f of v.frames) this.attachments.push({ name: `${file.name} · ${clock(f.at)}`, text: '', image: { mediaType: 'image/jpeg', data: f.data }, file: true });
          const about = `Video ${file.name}, ${clock(v.seconds)} long: ${v.frames.length} frames are attached as images, taken at ${v.frames.map((f) => clock(f.at)).join(', ')}.`;
          this.attachments.push({ name: `${file.name} (${v.transcript ? 'transcript' : 'about'})`, text: v.transcript ? `${about}\n\nWhat is said in it:\n${v.transcript}` : `${about} ${v.note ?? ''}`, file: true });
          if (v.note) toast(`${file.name}: ${v.note}`);
        } catch (e) {
          toast(`${file.name}: ${String(e)}`, 'error');
        }
        continue;
      }
      // Recordings are transcribed with the speech-to-text model, and the transcript attached.
      if (file.type.startsWith('audio/') || /\.(mp3|m4a|wav|ogg|oga|opus|flac|webm)$/i.test(file.name)) {
        try {
          if (file.size > 25 * 1024 * 1024) throw new Error(`${file.name} is larger than 25 MB.`);
          toast(`Transcribing ${file.name}…`);
          const data = await blobToBase64(file);
          const text = await invoke<string>('chat_transcribe', { audio: data, mime: file.type || 'audio/mpeg' });
          this.attachments.push({ name: `${file.name} (transcript)`, text, file: true });
        } catch (e) {
          toast(`${file.name}: ${String(e)}`, 'error');
        }
        continue;
      }
      // PDFs, Office documents and ZIP archives are read into text, as libraries read them.
      if (/\.(pdf|docx|pptx|odt|odp|zip)$/i.test(file.name)) {
        try {
          const data = await blobToBase64(file);
          const text = await invoke<string>('chat_read_document', { name: file.name, data });
          this.attachments.push({ name: file.name, text: text.length > 400_000 ? `${text.slice(0, 400_000)}\n… (cut; add the file to a library to search all of it)` : text, file: true });
        } catch (e) {
          toast(`${file.name}: ${String(e)}`, 'error');
        }
        continue;
      }
      // Workbooks are read into CSV, one block per sheet.
      if (/\.(xlsx|xlsm|xls|ods)$/i.test(file.name)) {
        try {
          const data = await reported(blobToBase64(file));
          const sheets = await call<[string, string][]>('chat_read_spreadsheet', { data });
          const text = sheets.map(([name, csv]) => `Sheet "${name}" (CSV):\n${csv}`).join('\n\n');
          this.attachments.push({ name: file.name, text: text.length > 400_000 ? `${text.slice(0, 400_000)}\n… (cut; analyse the full file with run_code)` : text, file: true });
        } catch {
          // Already shown.
        }
        continue;
      }
      if (file.size > 512 * 1024) {
        toast(`${file.name} is larger than 512 KB.`, 'error');
        continue;
      }
      const text = await file.text();
      if (text.includes('\u0000')) {
        toast(`${file.name} is not a text file.`, 'error');
        continue;
      }
      this.attachments.push({ name: file.name, text, file: true });
    }
    this.renderChips();
    this.renderBar();
  },

  /** Attaches a block of text (forwarded messages and the like) to the next message. */
  attachText(this: Composer, name: string, text: string) {
    this.attachments.push({ name, text });
    this.renderChips();
    this.renderBar();
  },
};

type Methods = typeof methods;

declare module './composer' {
  interface Composer extends Methods {}
}

Object.assign(Composer.prototype, methods);
