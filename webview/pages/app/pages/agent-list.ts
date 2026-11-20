/** The agents in state: loading them, pinning them, and how an agent's name and avatar show. */
import { api, type Agent } from '../core/api';
import { h } from '../ui/h';
import { changed, state } from '../core/state';
import { json } from '../core/prefs';

export async function reloadAgents() {
  state.agents = await api.agents();
  changed();
}

/** Agents pinned to the sidebar's Pinned section (LibreChat's pinned agents), on this device. */
const pinned = json<string[]>('cortex.pinnedAgents', () => []);

export function pinnedAgents(): string[] {
  return pinned.get();
}

export function togglePinnedAgent(id: string) {
  const all = pinnedAgents();
  const next = all.includes(id) ? all.filter((a) => a !== id) : [...all, id];
  pinned.set(next);
}

/** An agent's name with its title next to it: `João [3D Artist]`. */
export function agentName(agent: Agent): (string | HTMLElement)[] {
  const title = agent.title?.trim();
  // Names and titles are the user's: the interface translation leaves them alone.
  const name = h('span', { class: 'agent-name' }, agent.name);
  return title ? [name, h('span', { class: 'agent-role' }, `[${title}]`)] : [name];
}
export function avatar(agent: Agent, size: 'small' | 'large' = 'small'): HTMLElement {
  return h('span', { class: `agent-avatar ${size}` }, agent.avatar || agent.name.charAt(0).toUpperCase());
}
