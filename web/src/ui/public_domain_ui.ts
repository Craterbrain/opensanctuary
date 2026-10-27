// Downloading the public-domain hymn library (the Bibles repository's songs/public-domain-hymns.json) into the song library.
// Used by the first-run wizard's Songs step and by Import > Public-domain hymns.
import { showToast } from '../core/ui_utils';
import { hostTokenHeader } from '../core/host_session.ts';

/** Downloads the hymns and adds the ones the library doesn't have; returns the message to show. Never throws. */
export async function addHymnLibrary(): Promise<{ ok: boolean; message: string; added: number }> {
  let res: Response;
  try {
    res = await fetch('/api/songs/public-domain', { method: 'POST', headers: { ...hostTokenHeader() } });
  } catch (e: any) {
    return { ok: false, added: 0, message: `Could not download the hymns: ${e?.message ?? e}` };
  }
  if (!res.ok) {
    const detail = (await res.text().catch(() => '')).trim();
    return { ok: false, added: 0, message: detail ? `Could not download the hymns: ${detail}` : `Could not download the hymns (HTTP ${res.status}).` };
  }
  const j = await res.json();
  const added = Number(j.added) || 0;
  const had = Number(j.already_present) || 0;
  const message = added
    ? `Added ${added} hymn${added === 1 ? '' : 's'}${had ? ` (${had} you already had were left as they are)` : ''}.`
    : `Nothing to add: all ${j.total} hymns are already in your library.`;
  return { ok: true, added, message };
}

/** Wires a "download the hymns" button and the line that reports the result. `onChanged` runs after hymns were added. */
export function initPublicDomainCard(buttonId: string, statusId: string, onChanged: () => void = () => {}, root: ParentNode = document): void {
  const btn = root.querySelector(`#${buttonId}`) as HTMLButtonElement | null;
  const status = root.querySelector(`#${statusId}`) as HTMLElement | null;
  const idle = btn?.textContent ?? '';
  btn?.addEventListener('click', async () => {
    btn.disabled = true;
    btn.textContent = 'Downloading…';
    try {
      const r = await addHymnLibrary();
      if (status) {
        status.style.display = 'block';
        status.style.color = r.ok ? 'var(--os-success-text)' : 'var(--os-danger-text)';
        status.textContent = r.message;
      }
      showToast(r.message, r.ok ? 'success' : 'error', 5000);
      if (r.ok && r.added) onChanged();
    } finally {
      btn.disabled = false;
      btn.textContent = idle;
    }
  });
}
