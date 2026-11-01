/**
 * Bring a picture from the operator's own computer into the media library.
 *
 * Until now the only ways to use an image in the slide editor were to type a URL
 * (window.prompt, which the desktop webview doesn't implement, so nothing happened)
 * or to pick one that was already in the library. This opens the system file chooser,
 * sends the chosen file to POST /api/media/upload (which checks it by content and files
 * it under /media/images), and returns the path the editor can use.
 */
import { hostTokenHeader } from './host_session.ts';
import { showToast } from './ui_utils.ts';

export interface UploadedImage {
  /** e.g. /media/images/sunset_3f2a....jpg */
  file_path: string;
  /** The name the operator knows it by (the original file name). */
  name: string;
}

export const IMAGE_ACCEPT = 'image/png,image/jpeg,image/gif,image/webp,image/bmp,.png,.jpg,.jpeg,.gif,.webp,.bmp';

/** Uploads one already-chosen file. Throws an Error with a message fit to show the operator. */
export async function uploadImageFile(file: File): Promise<UploadedImage> {
  const res = await fetch(`/api/media/upload?name=${encodeURIComponent(file.name)}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/octet-stream', ...hostTokenHeader() },
    body: file,
  });
  if (!res.ok) {
    const detail = (await res.text().catch(() => '')).trim();
    if (res.status === 401 || res.status === 403) throw new Error('Not authorised to add media. Reload the console and try again.');
    if (res.status === 413) throw new Error('That picture is too large (the limit is 32 MB).');
    throw new Error(detail || `Upload failed (${res.status}).`);
  }
  const json = await res.json();
  return { file_path: json.file_path, name: file.name.replace(/\.[^.]+$/, '') || 'Image' };
}

/** Opens the file chooser; resolves with the chosen files (empty if cancelled). */
export function chooseImageFiles(multiple = false): Promise<File[]> {
  return new Promise(resolve => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = IMAGE_ACCEPT;
    input.multiple = multiple;
    input.style.display = 'none';
    let settled = false;
    const finish = (files: File[]) => {
      if (settled) return;
      settled = true;
      input.remove();
      resolve(files);
    };
    input.addEventListener('change', () => finish(Array.from(input.files ?? [])));
    input.addEventListener('cancel', () => finish([]));
    // Not every webview fires `cancel`. The page regains focus when the system dialog closes, and
    // `change` (if any) follows within a moment, so give it a grace period before calling it a cancel.
    window.addEventListener('focus', () => setTimeout(() => finish(Array.from(input.files ?? [])), 1000), { once: true });
    document.body.appendChild(input);
    input.click();
  });
}

/**
 * Choose picture(s) on this computer and add them to the library. Errors are shown as
 * a toast (one per rejected file) and the successful ones are still returned.
 */
export async function importImagesFromComputer(multiple = false): Promise<UploadedImage[]> {
  const files = await chooseImageFiles(multiple);
  const done: UploadedImage[] = [];
  for (const file of files) {
    try {
      done.push(await uploadImageFile(file));
    } catch (e) {
      showToast(`${file.name}: ${(e as Error).message}`, 'error', 6000);
    }
  }
  return done;
}
