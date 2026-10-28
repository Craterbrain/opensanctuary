// Importing a Bible from a link or a file (Import dialog > "Bible from a Link or File"). The server does the
// download and the reading (src/storage/bible_import.rs); this only asks, and asks again before replacing.

import { errorMessage } from '../core/ui_utils.ts';
import { showToast } from '../core/ui_utils';
import { dialogManager, registerModalReset, showConfirmDialog } from './dialog_manager.ts';
import { hostTokenHeader } from '../core/host_session.ts';

export interface InstalledBible {
  abbreviation: string;
  name: string;
  format: string;
  chapters: number;
  verses: number;
}

export interface BibleImportOutcome {
  ok: boolean;
  message: string;
  bible?: InstalledBible;
}

/** What to tell the operator after an install: "Installed WEB (World English Bible): 1,189 chapters, 31,102 verses (USFM)." */
export function installedMessage(b: InstalledBible): string {
  const name = b.name && b.name !== b.abbreviation ? ` (${b.name})` : '';
  return `Installed ${b.abbreviation}${name}: ${b.chapters.toLocaleString('en-US')} chapters, ${b.verses.toLocaleString('en-US')} verses (${b.format}).`;
}

/** A link worth sending: https, with a host name. (The server checks it again, harder.) */
export function looksLikeLink(text: string): boolean {
  try {
    const u = new URL(text.trim());
    return u.protocol === 'https:' && u.hostname.includes('.');
  } catch {
    return false;
  }
}

type Send = (replace: boolean) => Promise<Response>;

async function run(send: Send, label: string): Promise<BibleImportOutcome> {
  let res = await send(false);
  let body: any = await res.json().catch(() => ({}));
  if (res.status === 409 && body?.code === 'already_installed') {
    const ok = await new Promise<boolean>((resolve) => {
      showConfirmDialog(
        '📖 Bible already installed',
        `${body.abbreviation} is already installed. Replace it with ${label}?`,
        `The file holds ${body.bible?.chapters ?? '?'} chapters and ${body.bible?.verses ?? '?'} verses (${body.bible?.format ?? 'unknown format'}). The installed ${body.abbreviation} is deleted first; other Bibles are not touched.`,
        () => resolve(true),
        '↻ Replace',
      );
      // Cancelling the confirm dialog never calls back: answer "no" when it closes without accepting.
      registerModalReset('confirm-modal', () => setTimeout(() => resolve(false), 0));
    });
    dialogManager.unregisterResetHook('confirm-modal');
    if (!ok) return { ok: false, message: 'Left the installed Bible as it was.' };
    res = await send(true);
    body = await res.json().catch(() => ({}));
  }
  if (!res.ok || body?.success === false) return { ok: false, message: body?.error || `The import failed (${res.status}).` };
  return { ok: true, message: installedMessage(body.bible), bible: body.bible };
}

export function importBibleFromLink(url: string, abbreviation: string): Promise<BibleImportOutcome> {
  const abbr = abbreviation.trim();
  return run(
    (replace) => fetch('/api/bibles/import/link', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', ...hostTokenHeader() },
      body: JSON.stringify({ url: url.trim(), abbreviation: abbr || undefined, replace }),
    }),
    'the one at that link',
  );
}

export function importBibleFile(file: File, abbreviation: string): Promise<BibleImportOutcome> {
  const abbr = abbreviation.trim();
  return run(
    (replace) => {
      const q = new URLSearchParams({ name: file.name, replace: String(replace) });
      if (abbr) q.set('abbreviation', abbr);
      return fetch(`/api/bibles/import/file?${q}`, { method: 'POST', headers: { ...hostTokenHeader() }, body: file });
    },
    file.name,
  );
}

/** Wires the Import dialog card. `onInstalled` refreshes the sidebar. */
export function initBibleImportCard(onInstalled: () => void): void {
  const url = document.getElementById('bible-link-url') as HTMLInputElement | null;
  const abbr = document.getElementById('bible-link-abbr') as HTMLInputElement | null;
  const go = document.getElementById('btn-import-bible-link') as HTMLButtonElement | null;
  const file = document.getElementById('file-bible-import') as HTMLInputElement | null;
  const status = document.getElementById('bible-link-status');
  const say = (text: string, kind: 'info' | 'ok' | 'error') => {
    if (!status) return;
    status.style.display = 'block';
    status.style.color = kind === 'ok' ? 'var(--os-success-text)' : kind === 'error' ? 'var(--os-danger-text)' : 'var(--text-dim)';
    status.textContent = text;
  };
  const finish = (o: BibleImportOutcome) => {
    say(o.message, o.ok ? 'ok' : 'error');
    showToast(o.message, o.ok ? 'success' : 'error', 6000);
    if (o.ok) onInstalled();
  };
  go?.addEventListener('click', async () => {
    if (!url || !looksLikeLink(url.value)) {
      say('Paste a link that starts with https:// and points at a Bible file.', 'error');
      return;
    }
    go.disabled = true;
    say('Downloading and reading it… (a whole Bible can take a minute)', 'info');
    try {
      finish(await importBibleFromLink(url.value, abbr?.value ?? ''));
    } catch (e) {
      finish({ ok: false, message: `The import failed: ${errorMessage(e)}` });
    } finally {
      go.disabled = false;
    }
  });
  file?.addEventListener('change', async () => {
    const f = file.files?.[0];
    if (!f) return;
    say(`Reading ${f.name}…`, 'info');
    try {
      finish(await importBibleFile(f, abbr?.value ?? ''));
    } catch (e) {
      finish({ ok: false, message: `The import failed: ${errorMessage(e)}` });
    } finally {
      file.value = '';
    }
  });
}
