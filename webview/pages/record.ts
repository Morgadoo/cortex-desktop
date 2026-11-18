/**
 * Lumo desktop: the Recording pill, shown while the user does a task for Lumo to learn
 * (see src-tauri/src/chat/recorder.rs). It shows the time and the steps seen so far, and stops
 * or throws the recording away. It never takes focus, so the task goes on undisturbed.
 */
import { tauriInvoke } from '../shared/tauri';

document.addEventListener('DOMContentLoaded', () => {
  const time = document.getElementById('time') as HTMLElement;
  const tick = async () => {
    try {
      const s = await tauriInvoke<{ recording: boolean; seconds: number; steps: number }>('chat_record_status');
      const clock = `${Math.floor(s.seconds / 60)}:${String(s.seconds % 60).padStart(2, '0')}`;
      time.textContent = `${clock} · ${s.steps === 0 ? 'nothing yet' : s.steps === 1 ? '1 step seen' : `${s.steps} steps seen`}`;
    } catch {
      /* the next tick tries again */
    }
  };
  setInterval(() => void tick(), 1000);
  void tick();
  document.getElementById('stop')?.addEventListener('click', () => void tauriInvoke('chat_record_stop', { keep: true }));
  document.getElementById('cancel')?.addEventListener('click', () => void tauriInvoke('chat_record_stop', { keep: false }));
});
