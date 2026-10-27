// Import dialog > "Public-domain hymns": adds the hymns bundled with the program (src/storage/pd_songs.rs) that the library doesn't have yet.
import { showToast } from '../core/ui_utils';
import { hostTokenHeader } from '../core/host_session.ts';

export interface HymnLibraryState { total: number; present: number; missing: number }

/** What the library has of the bundled hymns, or null when the server can't say. */
export async function fetchHymnLibraryState(): Promise<HymnLibraryState | null> {
  try {
    const res = await fetch('/api/songs/public-domain');
    if (!res.ok) return null;
    const j = await res.json();
    return { total: Number(j.total) || 0, present: Number(j.present) || 0, missing: Number(j.missing) || 0 };
  } catch {
    return null;
  }
}

/** The sentence under the button for a given state. */
export function hymnStateText(s: HymnLibraryState | null): string {
  if (!s) return '';
  if (s.missing === 0) return `All ${s.total} hymns are already in your song library.`;
  if (s.present === 0) return `${s.total} hymns, none in your library yet.`;
  return `${s.missing} of ${s.total} hymns are not in your library yet.`;
}

/** Adds the missing hymns; returns the message to show. */
export async function addHymnLibrary(): Promise<{ ok: boolean; message: string; added: number }> {
  const res = await fetch('/api/songs/public-domain', { method: 'POST', headers: { ...hostTokenHeader() } });
  if (!res.ok) return { ok: false, added: 0, message: `Could not add the hymns (HTTP ${res.status}).` };
  const j = await res.json();
  const added = Number(j.added) || 0;
  const had = Number(j.already_present) || 0;
  const message = added
    ? `Added ${added} hymn${added === 1 ? '' : 's'}${had ? ` (${had} you already had were left as they are)` : ''}.`
    : `Nothing to add: all ${j.total} hymns are already in your library.`;
  return { ok: true, added, message };
}

/** Wires the Import dialog card. `onChanged` refreshes the song list. Returns a function that refreshes the card's text (call when the dialog opens). */
export function initPublicDomainCard(onChanged: () => void): () => void {
  const btn = document.getElementById('btn-add-public-domain') as HTMLButtonElement | null;
  const info = document.getElementById('public-domain-state');
  const status = document.getElementById('public-domain-status');
  const refresh = async () => {
    const s = await fetchHymnLibraryState();
    if (info) info.textContent = hymnStateText(s);
    if (btn) btn.disabled = !!s && s.missing === 0;
  };
  btn?.addEventListener('click', async () => {
    btn.disabled = true;
    try {
      const r = await addHymnLibrary();
      if (status) {
        status.style.display = 'block';
        status.style.color = r.ok ? 'var(--os-success-text)' : 'var(--os-danger-text)';
        status.textContent = r.message;
      }
      showToast(r.message, r.ok ? 'success' : 'error', 5000);
      if (r.ok && r.added) onChanged();
    } catch (e: any) {
      showToast(`Could not add the hymns: ${e?.message ?? e}`, 'error');
    } finally {
      await refresh();
    }
  });
  void refresh();
  return () => { void refresh(); };
}
