/**
 * Saving to and opening OpenSanctuary's own files: `.ossong` (songs), `.ospres` (presentations),
 * `.osservice` (a whole schedule) and `.osbible` (a Bible translation). They are zip files holding
 * the item together with its pictures, videos and music (see src/storage/bundle.rs).
 *
 * Opening one is two steps on purpose. The file is first sent for *inspection* (the server describes it
 * and changes nothing); if it holds songs that carry a copyright or CCLI number, an advisory pop-up
 * says so and asks to continue. It never blocks: Continue always imports. Only then is the file sent
 * again to be imported.
 */
import { closeModal, dialogManager, registerModalReset, showConfirmDialog } from './dialog_manager.ts';
import { hostTokenHeader, resolveHostSessionToken } from '../core/host_session.ts';
import { escapeHtml } from '../core/presentation_helpers.ts';
import { showToast } from '../core/ui_utils.ts';

export type BundleKind = 'song' | 'bible' | 'presentation' | 'service';

export const BUNDLE_EXTENSIONS: Record<BundleKind, string> = {
  song: 'ossong',
  bible: 'osbible',
  presentation: 'ospres',
  service: 'osservice',
};

/** For an `<input type="file" accept>`: every kind of OpenSanctuary file. */
export const BUNDLE_ACCEPT = Object.values(BUNDLE_EXTENSIONS).map((e) => `.${e}`).join(',');

export function bundleKindForFile(name: string): BundleKind | null {
  const ext = name.toLowerCase().split('.').pop() ?? '';
  const hit = (Object.entries(BUNDLE_EXTENSIONS) as [BundleKind, string][]).find(([, e]) => e === ext);
  return hit ? hit[0] : null;
}

export interface CopyrightNote {
  title: string;
  author: string;
  copyright: string;
  ccli_number: string;
}

export interface BundleSummary {
  kind: BundleKind;
  title: string;
  names: string[];
  counts: { songs: number; presentations: number; items: number; chapters: number };
  media_count: number;
  media_bytes: number;
  copyrighted: CopyrightNote[];
  library: { new: number; identical: number; different: number } | null;
  bible: { abbreviation: string; name: string } | null;
  bible_installed: boolean;
}

export interface ImportResult {
  kind: BundleKind | null;
  added: number;
  skipped_identical: number;
  added_as_copies: number;
  media_written: number;
  media_reused: number;
  backgrounds_added?: number;
  themes_added: string[];
  bible: string | null;
  warnings: string[];
}

/** One line of the advisory: the song, and what it says about its rights. */
export function copyrightLine(n: CopyrightNote): string {
  const rights: string[] = [];
  if (n.copyright.trim()) rights.push(n.copyright.trim());
  if (n.ccli_number.trim()) rights.push(`CCLI #${n.ccli_number.trim()}`);
  const by = n.author.trim() ? ` (${n.author.trim()})` : '';
  return `${n.title}${by}${rights.length ? ` — ${rights.join(' · ')}` : ''}`;
}

/**
 * Whether opening this file calls for the copyright advisory: it has songs under copyright, and importing
 * would actually add some. A file whose songs are all in the library already changes nothing, so it's not
 * worth interrupting for.
 */
export function advisoryApplies(summary: Pick<BundleSummary, 'copyrighted' | 'library'>): boolean {
  if (summary.copyrighted.length === 0) return false;
  return !summary.library || summary.library.new + summary.library.different > 0;
}

/** What to tell the user after an import, in plain words. */
export function importMessage(kind: BundleKind, title: string, r: ImportResult): string {
  const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
  const notes: string[] = [];
  if (r.skipped_identical) notes.push(`${r.skipped_identical} already in your library`);
  if (r.added_as_copies) notes.push(`${plural(r.added_as_copies, 'added as a copy', 'added as copies')} because yours differ`);
  if (r.themes_added.length) notes.push(`${plural(r.themes_added.length, 'theme', 'themes')} added`);
  if (r.backgrounds_added) notes.push(`${plural(r.backgrounds_added, 'picture', 'pictures')} added to Media as backgrounds`);
  const tail = notes.length ? ` (${notes.join('; ')})` : '';
  const added = r.added + r.added_as_copies;
  switch (kind) {
    case 'song':
      return added || !r.skipped_identical ? `✓ Imported ${plural(added, 'song', 'songs')}${tail}` : `✓ Nothing new: ${plural(r.skipped_identical, 'song is', 'songs are')} already in your library`;
    case 'presentation':
      return added || !r.skipped_identical
        ? `✓ Imported ${plural(added, 'presentation', 'presentations')}${tail}`
        : `✓ Nothing new: ${plural(r.skipped_identical, 'presentation is', 'presentations are')} already in your library`;
    case 'service':
      return `✓ Opened service "${title}" (${plural(r.added, 'item', 'items')})${r.themes_added.length ? `, ${plural(r.themes_added.length, 'theme', 'themes')} added` : ''}`;
    case 'bible':
      return `✓ Installed ${r.bible ?? title} (${plural(r.added, 'chapter', 'chapters')})`;
  }
}

async function readError(res: Response): Promise<string> {
  const text = await res.text();
  try {
    const j = JSON.parse(text);
    return j.message || j.error || text;
  } catch {
    return text || `The server answered ${res.status}`;
  }
}

/** Asks the server what a file holds, and how it compares with the library. Changes nothing. */
export async function inspectBundle(file: File): Promise<BundleSummary> {
  await resolveHostSessionToken();
  const res = await fetch('/api/bundle/inspect', { method: 'POST', headers: { ...hostTokenHeader() }, body: file });
  if (!res.ok) throw new Error(await readError(res));
  return (await res.json()).summary as BundleSummary;
}

// ------------------------------------------------------------------------------------------------
// The copyright advisory
// ------------------------------------------------------------------------------------------------

let resolveAdvisory: ((proceed: boolean) => void) | null = null;
let advisoryWired = false;

function wireAdvisory(): void {
  if (advisoryWired) return;
  advisoryWired = true;
  const settle = (proceed: boolean) => {
    const r = resolveAdvisory;
    resolveAdvisory = null;
    r?.(proceed);
  };
  document.getElementById('btn-continue-bundle-advisory')?.addEventListener('click', () => {
    settle(true); // before closing: closing runs the reset hook below, which means "cancelled"
    closeModal('bundle-advisory-modal');
  });
  document.getElementById('btn-cancel-bundle-advisory')?.addEventListener('click', () => closeModal('bundle-advisory-modal'));
  document.getElementById('btn-close-bundle-advisory')?.addEventListener('click', () => closeModal('bundle-advisory-modal'));
  // Any other way of closing it (Escape, clicking outside) means "not now".
  registerModalReset('bundle-advisory-modal', () => settle(false));
}

/**
 * Shows the advisory for songs that aren't public domain and resolves once the user has answered:
 * `true` for Continue, `false` for anything that closes the pop-up without it.
 */
export function confirmCopyright(notes: CopyrightNote[]): Promise<boolean> {
  const modal = document.getElementById('bundle-advisory-modal');
  if (!modal) return Promise.resolve(true); // no pop-up in this page: never stand in the way
  wireAdvisory();
  const lead = document.getElementById('bundle-advisory-lead');
  if (lead) {
    lead.textContent =
      notes.length === 1 ? 'This file includes a song that is under copyright:' : `This file includes ${notes.length} songs that are under copyright:`;
  }
  const list = document.getElementById('bundle-advisory-list');
  if (list) list.innerHTML = notes.map((n) => `<li>${escapeHtml(copyrightLine(n))}</li>`).join('');
  return new Promise<boolean>((resolve) => {
    resolveAdvisory?.(false); // a pop-up already waiting is superseded
    resolveAdvisory = resolve;
    dialogManager.openModal(modal, { zIndex: 10000 });
  });
}

// ------------------------------------------------------------------------------------------------
// Opening a file
// ------------------------------------------------------------------------------------------------

export interface ImportOptions {
  /** For a service: replace the current schedule (default) or add to it. */
  mode?: 'replace' | 'append';
  /** Called once something was added to the library, so the panels can reload. */
  onLibraryChanged?: (kind: BundleKind) => void;
}

export interface ImportOutcome {
  kind: BundleKind;
  title: string;
  result: ImportResult;
  /** What was said to the user. */
  message: string;
  /** For a service: the new state of the show. */
  snapshot: any | null;
}

/**
 * Opens one OpenSanctuary file. Resolves to what was imported, or `null` if it was refused, failed
 * (the user has been told why) or was cancelled at the advisory.
 */
export async function importBundleFile(file: File, opts: ImportOptions = {}): Promise<ImportOutcome | null> {
  if (!bundleKindForFile(file.name)) {
    showToast(`"${file.name}" isn't an OpenSanctuary file (.ossong, .ospres, .osservice or .osbible).`, 'warning');
    return null;
  }
  showToast(`Reading ${file.name}...`, 'info');
  let summary: BundleSummary;
  try {
    summary = await inspectBundle(file);
  } catch (e: any) {
    showToast(`Couldn't open ${file.name}: ${e?.message ?? e}`, 'error', 7000);
    return null;
  }

  let replace = false;
  if (summary.kind === 'bible' && summary.bible_installed) {
    const abbr = summary.bible?.abbreviation ?? summary.title;
    const ok = await new Promise<boolean>((resolve) => {
      showConfirmDialog(
        '📖 Bible already installed',
        `${abbr} is already installed. Replace it with the version in this file?`,
        'The Bible you have now is removed first. Songs, schedules and other Bibles are not touched.',
        () => resolve(true),
        '↻ Replace',
      );
      // Cancelling the confirm dialog never calls back, so answer "no" when it closes without accepting
      // (Accept resolves first, so this is then a no-op).
      registerModalReset('confirm-modal', () => setTimeout(() => resolve(false), 0));
    });
    dialogManager.unregisterResetHook('confirm-modal');
    if (!ok) {
      showToast('Import cancelled', 'info');
      return null;
    }
    replace = true;
  }

  if (advisoryApplies(summary)) {
    const proceed = await confirmCopyright(summary.copyrighted);
    if (!proceed) {
      showToast('Import cancelled', 'info');
      return null;
    }
  }

  try {
    await resolveHostSessionToken();
    const q = new URLSearchParams();
    if (replace) q.set('replace', 'true');
    if (opts.mode === 'append') q.set('mode', 'append');
    const res = await fetch(`/api/bundle/import${q.toString() ? `?${q}` : ''}`, { method: 'POST', headers: { ...hostTokenHeader() }, body: file });
    if (!res.ok) {
      showToast(`Couldn't import ${file.name}: ${await readError(res)}`, 'error', 7000);
      return null;
    }
    const body = await res.json();
    const result = body.result as ImportResult;
    const title = body.title ?? summary.title;
    const message = importMessage(summary.kind, title, result);
    showToast(message, 'success', 6000);
    if (summary.kind === 'song' || summary.kind === 'presentation' || summary.kind === 'bible') opts.onLibraryChanged?.(summary.kind);
    return { kind: summary.kind, title, result, message, snapshot: body.snapshot ?? null };
  } catch (e: any) {
    showToast(`Couldn't import ${file.name}: ${e?.message ?? e}`, 'error', 7000);
    return null;
  }
}

// ------------------------------------------------------------------------------------------------
// Saving a file
// ------------------------------------------------------------------------------------------------

export interface ExportRequest {
  kind: BundleKind;
  /** Songs or presentations; everything of that kind when omitted. */
  ids?: string[];
  /** A Bible's abbreviation. */
  version?: string;
  /** A service's name. */
  title?: string;
}

/** The file name the server chose, from its Content-Disposition header. */
export function filenameFromDisposition(header: string | null, fallback: string): string {
  if (!header) return fallback;
  const star = /filename\*=UTF-8''([^;]+)/i.exec(header);
  if (star) {
    try {
      return decodeURIComponent(star[1] ?? '');
    } catch {
      /* fall through to the plain form */
    }
  }
  const plain = /filename="([^"]+)"/i.exec(header);
  return plain?.[1] ?? fallback;
}

export function saveBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

/** Downloads an OpenSanctuary file. Resolves to whether it was saved. */
export async function exportBundle(req: ExportRequest, fallbackName: string): Promise<boolean> {
  try {
    await resolveHostSessionToken();
    const res = await fetch('/api/export/bundle', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', ...hostTokenHeader() },
      body: JSON.stringify(req),
    });
    if (!res.ok) {
      showToast(`Export failed: ${await readError(res)}`, 'error', 7000);
      return false;
    }
    const name = filenameFromDisposition(res.headers.get('content-disposition'), `${fallbackName}.${BUNDLE_EXTENSIONS[req.kind]}`);
    saveBlob(await res.blob(), name);
    const missing = Number(res.headers.get('x-os-missing-media') ?? 0);
    const media = Number(res.headers.get('x-os-media-count') ?? 0);
    showToast(`✓ Saved ${name}${media ? ` (with ${media} media file${media === 1 ? '' : 's'})` : ''}`, 'success', 5000);
    if (missing) {
      showToast(`${missing} media file${missing === 1 ? ' was' : 's were'} missing from your library and left out.`, 'warning', 8000);
    }
    return true;
  } catch (e: any) {
    showToast(`Export failed: ${e?.message ?? e}`, 'error', 7000);
    return false;
  }
}
