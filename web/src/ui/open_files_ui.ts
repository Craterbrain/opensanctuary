// Files opened from the desktop (double-click, "Open with"): the host lists them (src/network/open_files.rs) and the console takes each one
// through the same steps as the Import dialog, so a copyrighted song still shows its advisory and a bad file is refused with a reason.
import { showToast } from '../core/ui_utils';
import { hostTokenHeader, resolveHostSessionToken } from '../core/host_session.ts';
import { dialogManager, registerModalReset, showConfirmDialog } from './dialog_manager.ts';
import { bundleKindForFile } from './bundle_ui.ts';

export interface PendingOpenFile { id: string; name: string }

export interface OpenFilesDeps {
  /** Opens one file the way the Import dialog does. */
  open: (file: File, mode: 'replace' | 'append') => Promise<unknown>;
  /** Items in the current schedule: opening a service replaces it, so a busy schedule is asked about first. */
  scheduleItemCount: () => number;
}

export async function fetchPendingOpenFiles(): Promise<PendingOpenFile[]> {
  try {
    await resolveHostSessionToken();
    const res = await fetch('/api/open-file/pending', { headers: { ...hostTokenHeader() } });
    if (!res.ok) return [];
    const list = await res.json();
    return Array.isArray(list) ? list.filter((f) => f && typeof f.id === 'string' && typeof f.name === 'string') : [];
  } catch {
    return [];
  }
}

/** Reads one waiting file from the host (once; it leaves the list). */
export async function takeOpenFile(f: PendingOpenFile): Promise<File | null> {
  try {
    const res = await fetch(`/api/open-file/${encodeURIComponent(f.id)}/take`, { method: 'POST', headers: { ...hostTokenHeader() } });
    if (!res.ok) return null;
    return new File([await res.blob()], f.name, { type: 'application/octet-stream' });
  } catch {
    return null;
  }
}

function confirmReplaceSchedule(name: string, items: number): Promise<boolean> {
  return new Promise<boolean>((resolve) => {
    showConfirmDialog(
      '📋 Open this service?',
      `Open "${name}" and replace the current schedule (${items} item${items === 1 ? '' : 's'})?`,
      'The current schedule is replaced, not saved. Cancel keeps it as it is.',
      () => resolve(true),
      'Open service',
    );
    // Cancelling the confirm dialog never calls back: answer "no" when it closes without accepting.
    registerModalReset('confirm-modal', () => setTimeout(() => resolve(false), 0));
  }).finally(() => dialogManager.unregisterResetHook('confirm-modal'));
}

let busy = false;

/** Opens every file the host has waiting, one after another. Safe to call again while it runs (the later call does nothing). */
export async function openPendingFiles(deps: OpenFilesDeps): Promise<number> {
  if (busy) return 0;
  busy = true;
  let opened = 0;
  try {
    for (let round = 0; round < 10; round++) {
      const list = await fetchPendingOpenFiles();
      if (!list.length) break;
      for (const f of list) {
        const file = await takeOpenFile(f);
        if (!file) {
          showToast(`Could not read ${f.name}.`, 'error');
          continue;
        }
        const items = deps.scheduleItemCount();
        if (bundleKindForFile(file.name) === 'service' && items > 0 && !(await confirmReplaceSchedule(file.name, items))) {
          showToast('Left the current schedule as it was.', 'info');
          continue;
        }
        await deps.open(file, 'replace');
        opened++;
      }
    }
  } finally {
    busy = false;
  }
  return opened;
}
