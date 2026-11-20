/** Saving a file the interface made, through the system's save dialog. */
import { tauriInvoke } from '../../../shared/tauri';
import { blobToBase64 } from './format';
import { toast } from '../ui/overlay';

/** Saves a blob through the system's save dialog (the webview does not download blobs). */
export async function saveFile(name: string, blob: Blob): Promise<void> {
  try {
    if (await tauriInvoke<boolean>('chat_save_file', { name, data: await blobToBase64(blob) })) toast('Saved.');
  } catch (e) {
    toast(String(e), 'error');
  }
}
