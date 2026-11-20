/** The model list in state: reloading it, and keeping the chosen model one that exists. */
import { api } from './api';
import { changed, state } from './state';

/** Reloads models after providers change, keeping the chosen one when it still exists. */
export async function reloadModels() {
  const { models, defaultModel } = await api.models();
  state.models = models;
  if (!state.model || !models.some((m) => m.reference === state.model)) state.model = defaultModel;
  changed();
}
