/** Where a mini app is served and what files it has: a leaf, so canvas, diagrams and share need not import each other for these. */
/** Where a mini app is served: the `cortexapp` protocol, spelled per platform. */
export function appUrl(id: string, version = 0): string {
  const windows = navigator.userAgent.includes('Windows');
  // The trailing slash lets a multi-file app's relative links (style.css, js/app.js) resolve.
  return `${windows ? 'http://cortexapp.localhost' : 'cortexapp://localhost'}/${id}/?v=${version}`;
}

/** A multi-file app's files (Vibe's file list), or `null` for a one-file app. */
export function appFiles(code: string): { path: string; content: string }[] | null {
  if (!code.trimStart().startsWith('{')) return null;
  try {
    const files = (JSON.parse(code) as { lumoAppFiles?: { path: string; content: string }[] }).lumoAppFiles;
    return Array.isArray(files) && files.length ? files : null;
  } catch {
    return null;
  }
}
