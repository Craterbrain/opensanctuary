// A licensed Bible's notice must come with a one-step link to its publisher (Biblica for the NIV, Lockman for the NASB and AMP;
// docs/LICENSED_SCRIPTURE.md). A projected slide can't be clicked, so the console shows the link wherever the passage is: next to the title in the
// Preview and Live panels. The host opens it in the browser, and only for the publishers' own sites.
import type { ScheduleItem } from '../core/models.ts';
import { hostTokenHeader, resolveHostSessionToken } from '../core/host_session.ts';

/** The publisher link of a schedule item, or null for anything that isn't a licensed passage with one. */
export function licensedLinkOf(item: ScheduleItem | null | undefined): { tag: string; link: string } | null {
  const l = item?.licensed;
  if (!l || typeof l.link !== 'string' || !/^https:\/\//i.test(l.link)) return null;
  return { tag: String(l.tag || l.version || 'Publisher'), link: l.link };
}

export async function openPublisherLink(link: string): Promise<boolean> {
  try {
    await resolveHostSessionToken();
    const res = await fetch('/api/system/open-url', { method: 'POST', headers: { 'Content-Type': 'application/json', ...hostTokenHeader() }, body: JSON.stringify({ url: link }) });
    const j = await res.json().catch(() => ({}));
    return res.ok && j.ok === true;
  } catch {
    return false;
  }
}

/** Puts (or removes) the link right after `titleEl`. Safe to call on every render. */
export function showLicensedLink(titleEl: HTMLElement | null, item: ScheduleItem | null | undefined): void {
  if (!titleEl || !titleEl.parentElement) return;
  const id = `${titleEl.id}-licensed-link`;
  const existing = document.getElementById(id);
  const info = licensedLinkOf(item);
  if (!info) {
    existing?.remove();
    return;
  }
  const a = (existing as HTMLAnchorElement | null) ?? document.createElement('a');
  a.id = id;
  a.className = 'licensed-link';
  a.href = info.link;
  a.textContent = `${info.tag} ↗`;
  a.title = `Open the publisher's page (${info.link})`;
  a.setAttribute('data-no-iconify', '');
  a.onclick = (e) => { e.preventDefault(); void openPublisherLink(info.link); };
  if (!existing) titleEl.after(a);
}
