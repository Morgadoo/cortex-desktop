/**
 * Formatting shared by the whole interface: sizes, counts, HTML escaping and base64 for IPC.
 *
 * Sizes are decimal (1 KB = 1,000 bytes), as macOS and most Linux file managers show them, so a
 * file shows the same size in every panel and in the system's own windows.
 */

/** A size in bytes: `512 B`, `34 KB`, `1.2 MB`, `3.4 GB`. */
export function fileSize(bytes: number): string {
  if (bytes < 1000) return `${bytes} B`;
  if (bytes < 1_000_000) return `${Math.round(bytes / 1000)} KB`;
  if (bytes < 1_000_000_000) return `${(bytes / 1_000_000).toFixed(1)} MB`;
  return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
}

/** A length of text: `800 characters`, `12k characters`, `1.2M characters`. */
export function characterCount(chars: number): string {
  if (chars >= 1_000_000) return `${(chars / 1_000_000).toFixed(1)}M characters`;
  if (chars >= 1000) return `${Math.round(chars / 1000)}k characters`;
  return `${chars} characters`;
}

/** Text made safe to put inside HTML, in element content or a quoted attribute. */
export function escapeHtml(text: string): string {
  return text.replace(/[&<>"']/g, (c) => ESCAPES[c] ?? c);
}

const ESCAPES: Record<string, string> = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' };

/** Bytes as base64. Chunked, because spreading a large array into one call overflows the stack. */
export function bytesToBase64(bytes: Uint8Array): string {
  let binary = '';
  for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(binary);
}

/** A blob as base64, for IPC. */
export async function blobToBase64(blob: Blob): Promise<string> {
  return bytesToBase64(new Uint8Array(await blob.arrayBuffer()));
}
