/**
 * A conversation as one PNG image (LobeHub's "export as screenshot"): the title, then each
 * message as a bubble, drawn on a canvas in the current theme so it reads like the app. Markdown
 * is flattened to its text; code keeps a monospace face.
 */

export interface ImageMessage {
  role: 'user' | 'assistant';
  text: string;
}

/** Markdown to the lines a picture needs: no markup, code fenced lines marked. */
export function plainLines(markdown: string): { text: string; code: boolean; heading: boolean }[] {
  const out: { text: string; code: boolean; heading: boolean }[] = [];
  let inCode = false;
  for (const raw of markdown.replace(/\r/g, '').split('\n')) {
    if (/^\s*```/.test(raw)) {
      inCode = !inCode;
      continue;
    }
    if (inCode) {
      out.push({ text: raw, code: true, heading: false });
      continue;
    }
    const heading = /^\s*#{1,6}\s/.test(raw);
    const text = raw
      .replace(/^\s*#{1,6}\s+/, '')
      .replace(/^\s*[-*+]\s+/, '• ')
      .replace(/!\[([^\]]*)\]\([^)]*\)/g, '$1')
      .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
      .replace(/(\*\*|__)(.*?)\1/g, '$2')
      .replace(/(\*|_)(\S.*?)\1/g, '$2')
      .replace(/`([^`]+)`/g, '$1')
      .replace(/^\s*>\s?/, '')
      .replace(/<attachment name="([^"]*)">[\s\S]*$/, '📎 $1');
    out.push({ text, code: false, heading });
  }
  // Runs of blank lines collapse to one.
  return out.filter((l, i, all) => l.text.trim() || l.code || (i > 0 && all[i - 1]!.text.trim()));
}

/** Breaks `text` into lines no wider than `width`. */
function wrap(ctx: CanvasRenderingContext2D, text: string, width: number): string[] {
  if (!text) return [''];
  const lines: string[] = [];
  let line = '';
  for (const word of text.split(/(\s+)/)) {
    const next = line + word;
    if (ctx.measureText(next).width <= width || !line) {
      // A single word wider than the line is cut by characters.
      if (ctx.measureText(next).width > width && !line) {
        let part = '';
        for (const ch of word) {
          if (ctx.measureText(part + ch).width > width) {
            lines.push(part);
            part = '';
          }
          part += ch;
        }
        line = part;
      } else line = next;
    } else {
      lines.push(line.trimEnd());
      line = word.trimStart();
    }
  }
  lines.push(line.trimEnd());
  return lines;
}

const css = (name: string, fallback: string) => getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;

/** Draws the conversation; returns PNG bytes. */
export async function conversationPng(title: string, messages: ImageMessage[]): Promise<Uint8Array> {
  const scale = 2;
  const width = 860;
  const pad = 40;
  const bubbleMax = width - pad * 2;
  const font = css('--lumo-font-sans', 'Inter, system-ui, sans-serif');
  const colors = {
    bg: css('--bg', '#17171b'),
    text: css('--text', '#ececf1'),
    muted: css('--text-muted', '#9b9ba7'),
    bubble: css('--bubble', '#2a2a33'),
    code: css('--code-bg', '#101014'),
    accent: css('--accent', '#8b6dff'),
    border: css('--border', '#2f2f38'),
  };
  const measure = document.createElement('canvas').getContext('2d')!;
  type Block = { role: ImageMessage['role']; lines: { text: string; code: boolean; heading: boolean }[]; height: number; inner: number };
  const lineHeight = 22;
  const blocks: Block[] = messages.map((m) => {
    const inner = m.role === 'user' ? bubbleMax * 0.78 - 32 : bubbleMax;
    const lines: Block['lines'] = [];
    for (const l of plainLines(m.text)) {
      measure.font = l.code ? `13px ui-monospace, monospace` : l.heading ? `600 16px ${font}` : `15px ${font}`;
      for (const w of wrap(measure, l.text, inner - (l.code ? 16 : 0))) lines.push({ ...l, text: w });
    }
    return { role: m.role, lines, inner, height: lines.length * lineHeight + (m.role === 'user' ? 24 : 4) };
  });
  const head = 110;
  const total = head + blocks.reduce((n, b) => n + b.height + 26, 0) + 50;
  const canvas = document.createElement('canvas');
  canvas.width = width * scale;
  canvas.height = Math.min(total, 30000) * scale;
  const ctx = canvas.getContext('2d')!;
  ctx.scale(scale, scale);
  ctx.fillStyle = colors.bg;
  ctx.fillRect(0, 0, width, total);
  ctx.textBaseline = 'top';
  ctx.fillStyle = colors.text;
  ctx.font = `700 24px ${font}`;
  ctx.fillText(wrap(ctx, title || 'Conversation', width - pad * 2)[0]!, pad, 36);
  ctx.fillStyle = colors.muted;
  ctx.font = `13px ${font}`;
  ctx.fillText(`Lumo · ${new Date().toLocaleString()}`, pad, 70);
  let y = head;
  for (const b of blocks) {
    if (b.role === 'user') {
      const w = Math.min(b.inner, Math.max(...b.lines.map((l) => { ctx.font = `15px ${font}`; return ctx.measureText(l.text).width; }))) + 32;
      const x = width - pad - w;
      ctx.fillStyle = colors.bubble;
      ctx.beginPath();
      if (typeof ctx.roundRect === 'function') ctx.roundRect(x, y, w, b.height, 16);
      else ctx.rect(x, y, w, b.height);
      ctx.fill();
      let ly = y + 12;
      for (const l of b.lines) {
        ctx.fillStyle = colors.text;
        ctx.font = `15px ${font}`;
        ctx.fillText(l.text, x + 16, ly + 3);
        ly += lineHeight;
      }
    } else {
      let ly = y;
      for (const l of b.lines) {
        if (l.code) {
          ctx.fillStyle = colors.code;
          ctx.fillRect(pad, ly, bubbleMax, lineHeight);
          ctx.fillStyle = colors.text;
          ctx.font = '13px ui-monospace, monospace';
          ctx.fillText(l.text, pad + 8, ly + 4);
        } else {
          ctx.fillStyle = colors.text;
          ctx.font = l.heading ? `600 16px ${font}` : `15px ${font}`;
          ctx.fillText(l.text, pad, ly + 3);
        }
        ly += lineHeight;
      }
    }
    y += b.height + 26;
  }
  ctx.fillStyle = colors.accent;
  ctx.fillRect(pad, total - 24, 36, 3);
  const blob = await new Promise<Blob | null>((r) => canvas.toBlob(r, 'image/png'));
  if (!blob) throw new Error('The image could not be made.');
  return new Uint8Array(await blob.arrayBuffer());
}
