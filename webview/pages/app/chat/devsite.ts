/** Finding a dev server's address in a command's output, for the Terminal panel's preview. */
/** The last local web address a command printed (a dev server's), if any. */
export function localSite(output: string): string | null {
  const plain = output.replace(/\x1b\[[0-9;]*m/g, '');
  const hits = [...plain.matchAll(/https?:\/\/(?:localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1\])(?::\d{2,5})?(?:\/[^\s'"<>)]*)?/g)];
  const last = hits[hits.length - 1]?.[0];
  return last ? last.replace('0.0.0.0', 'localhost').replace(/[.,;]+$/, '') : null;
}
