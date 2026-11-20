/** Saving a file the interface made, through the system's save dialog. */
import { blobToBase64 } from './format';
import { toast } from '../ui/overlay';
import { call, reported } from './call';

/** Saves a blob through the system's save dialog (the webview does not download blobs). */
export async function saveFile(name: string, blob: Blob): Promise<void> {
  try {
    if (await call<boolean>('chat_save_file', { name, data: await reported(blobToBase64(blob)) })) toast('Saved.');
  } catch {
    // Already shown.
  }
}
