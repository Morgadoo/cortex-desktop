/**
 * The one composer the home screen and conversations share. `composer/index.ts` builds it with
 * every part loaded and sets it here, so the modules the composer itself uses can reach it
 * without importing that index. Read it inside functions, never while modules load.
 */
import type { Composer } from './composer';

export let composer: Composer;

export function setComposer(built: Composer) {
  composer = built;
}
