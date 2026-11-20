/** The views on screen, so streamed events update them in place. */
import type { ChatView } from './chat-view';
import type { HomeView } from './home-view';

export const viewCache: { chat: ChatView | null; home: HomeView | null } = { chat: null, home: null };
