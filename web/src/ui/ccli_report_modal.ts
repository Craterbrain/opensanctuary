/**
 * OpenSanctuary / OS-Next CCLI Usage Report Modal Controller
 * docs/CCLI_REPORTING.md -- CSV export + due-date reminder, built per the
 * design doc's "Report window UI" section: a date-range picker, a visible
 * Reportable/Excluded-Public-Domain split (never a silent filter), and an
 * "Export CSV" action, plus a persistent sidebar explaining what counts as
 * "used," the PD exclusion rule, and the current reporting period.
 */
import { api } from '../core/api_client.ts';
import { escapeHtml, escapeUserHtml } from '../core/presentation_helpers.ts';
import type { ToastType } from '../core/ui_utils.ts';
import { keyring, KEYRING_SERVICES } from '../core/keyring.ts';

export interface CcliReportModalContext {
  showToast: (message: string, type?: ToastType) => void;
  showModal: (el: HTMLElement) => void;
  closeModal: (el: HTMLElement) => void;
}

interface CcliUsageRow {
  title: string;
  author: string;
  ccli_number: string | null;
  use_count: number;
  first_used_ms: number;
  last_used_ms: number;
  is_public_domain: boolean;
}

let ctx: CcliReportModalContext | null = null;

export function initCcliReportModal(context: CcliReportModalContext): void {
  ctx = context;

  const modal = document.getElementById('ccli-report-modal');

  document.getElementById('btn-close-ccli-report')?.addEventListener('click', () => {
    if (modal) ctx!.closeModal(modal);
  });
  document.getElementById('btn-close-ccli-report-2')?.addEventListener('click', () => {
    if (modal) ctx!.closeModal(modal);
  });
  document.getElementById('btn-ccli-report-refresh')?.addEventListener('click', () => loadAndRenderReport());
  document.getElementById('btn-ccli-report-export')?.addEventListener('click', () => exportCurrentRangeCsv());
  document.getElementById('btn-ccli-report-save-creds')?.addEventListener('click', () => saveCcliCredentials());
  document.getElementById('btn-ccli-report-clear-creds')?.addEventListener('click', () => clearCcliCredentials());
  document.getElementById('btn-ccli-report-open-assist')?.addEventListener('click', () => openUploadAssist());
}

/** Formats a Date as the `YYYY-MM-DD` value an `<input type="date">` expects. */
function toDateInputValue(d: Date): string {
  return d.toISOString().slice(0, 10);
}

/** Start-of-day ms for a date-input value, in local time -- matches the operator's own calendar day. */
function startOfDayMs(dateStr: string): number {
  return new Date(`${dateStr}T00:00:00`).getTime();
}

/** End-of-day ms for a date-input value, so the selected end date is inclusive. */
function endOfDayMs(dateStr: string): number {
  return new Date(`${dateStr}T23:59:59.999`).getTime();
}

export async function openCcliReportModal(): Promise<void> {
  const modal = document.getElementById('ccli-report-modal');
  if (!modal) return;

  const startInput = document.getElementById('ccli-report-start') as HTMLInputElement | null;
  const endInput = document.getElementById('ccli-report-end') as HTMLInputElement | null;
  if (startInput && !startInput.value) {
    const jan1 = new Date();
    jan1.setMonth(0, 1);
    startInput.value = toDateInputValue(jan1);
  }
  if (endInput && !endInput.value) {
    endInput.value = toDateInputValue(new Date());
  }

  await updateDueDateInfo();
  await updateCredentialsStatus();
  ctx!.showModal(modal);
  await loadAndRenderReport();
}

async function updateCredentialsStatus(): Promise<void> {
  const statusEl = document.getElementById('ccli-report-creds-status');
  const usernameInput = document.getElementById('ccli-report-username') as HTMLInputElement | null;
  if (!statusEl) return;
  try {
    const creds = await keyring.scope(KEYRING_SERVICES.CCLI).getUserCredentials();
    if (creds?.username) {
      statusEl.textContent = `Saved for "${creds.username}". Leave the password field blank and click Save to keep it unchanged.`;
      if (usernameInput && !usernameInput.value) usernameInput.value = creds.username;
    } else {
      statusEl.textContent = 'No login saved yet -- the Upload Assistant will still open without one, just without login autofill.';
    }
  } catch (_) {
    statusEl.textContent = '';
  }
}

async function saveCcliCredentials(): Promise<void> {
  const usernameInput = document.getElementById('ccli-report-username') as HTMLInputElement | null;
  const passwordInput = document.getElementById('ccli-report-password') as HTMLInputElement | null;
  const username = usernameInput?.value.trim() || '';
  let password = passwordInput?.value || '';
  if (!username) {
    ctx!.showToast('Enter a username first.', 'warning');
    return;
  }
  // An empty password field on an update means "keep the existing one" --
  // re-typing a password every time just to change the username would be
  // an annoying, password-manager-defeating requirement.
  if (!password) {
    const existing = await keyring.scope(KEYRING_SERVICES.CCLI).getUserCredentials().catch(() => null);
    password = existing?.password || '';
  }
  try {
    const ok = await keyring.scope(KEYRING_SERVICES.CCLI).setUserCredentials({ username, password });
    if (!ok) {
      ctx!.showToast('Could not save CCLI login -- the OS credential store rejected it.', 'error');
      return;
    }
    if (passwordInput) passwordInput.value = '';
    ctx!.showToast('CCLI login saved.', 'success');
    await updateCredentialsStatus();
  } catch (e: any) {
    ctx!.showToast(`Could not save CCLI login: ${e?.message || e}`, 'error');
  }
}

async function clearCcliCredentials(): Promise<void> {
  const usernameInput = document.getElementById('ccli-report-username') as HTMLInputElement | null;
  const passwordInput = document.getElementById('ccli-report-password') as HTMLInputElement | null;
  try {
    await keyring.scope(KEYRING_SERVICES.CCLI).deleteUserCredentials();
    if (usernameInput) usernameInput.value = '';
    if (passwordInput) passwordInput.value = '';
    ctx!.showToast('CCLI login cleared.', 'info');
    await updateCredentialsStatus();
  } catch (e: any) {
    ctx!.showToast(`Could not clear CCLI login: ${e?.message || e}`, 'error');
  }
}

async function openUploadAssist(): Promise<void> {
  const range = currentRangeMs();
  if (!range) {
    ctx!.showToast('Pick a start and end date first.', 'warning');
    return;
  }
  try {
    const result = await api.reports.openCcliAssist(range.start, range.end);
    if (!result.ok) {
      ctx!.showToast(result.error || 'Could not open the CCLI Upload Assistant.', 'error');
      return;
    }
    ctx!.showToast('Opening the CCLI Upload Assistant -- you still need to click Log In and Upload yourself.', 'info');
  } catch (e: any) {
    ctx!.showToast(`Could not open the CCLI Upload Assistant: ${e?.message || e}`, 'error');
  }
}

async function updateDueDateInfo(): Promise<void> {
  const infoEl = document.getElementById('ccli-report-due-date-info');
  if (!infoEl) return;
  try {
    const settings = await api.settings.get();
    const dueDate = (settings?.ccliReportingDueDate || '').trim();
    if (!dueDate) {
      infoEl.textContent = 'Set in Settings → Integrations → CCLI Reporting Due Date.';
      return;
    }
    const due = new Date(`${dueDate}T00:00:00`);
    const daysLeft = Math.ceil((due.getTime() - Date.now()) / 86400000);
    if (daysLeft < 0) {
      infoEl.textContent = `Due ${dueDate} — ${Math.abs(daysLeft)} day${Math.abs(daysLeft) === 1 ? '' : 's'} overdue.`;
    } else if (daysLeft === 0) {
      infoEl.textContent = `Due today (${dueDate}).`;
    } else {
      infoEl.textContent = `Due ${dueDate} — ${daysLeft} day${daysLeft === 1 ? '' : 's'} away.`;
    }
  } catch (_) {
    infoEl.textContent = 'Set in Settings → Integrations → CCLI Reporting Due Date.';
  }
}

function formatDate(ms: number): string {
  return new Date(ms).toLocaleDateString();
}

function renderRow(row: CcliUsageRow): string {
  const ccli = row.ccli_number ? `CCLI #${escapeHtml(row.ccli_number)}` : 'No CCLI number';
  return `
    <div style="padding: 6px 10px; background: rgba(255,255,255,0.04); border: 1px solid var(--os-border, #2d3139); border-radius: 6px; font-size: 11px; display: flex; gap: 10px; align-items: center;">
      <span style="flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-main, #eee); font-weight: 600;">${escapeUserHtml(row.title)}</span>
      <span style="color: var(--text-muted, #999); min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">${escapeUserHtml(row.author || '—')}</span>
      <span style="color: var(--text-dim, #8a8f9e); white-space: nowrap;">${ccli}</span>
      <span style="color: var(--os-brand-amber, #ffa726); white-space: nowrap; font-weight: 700;">${row.use_count}×</span>
      <span style="color: var(--text-muted, #777); white-space: nowrap;">${formatDate(row.first_used_ms)}–${formatDate(row.last_used_ms)}</span>
    </div>
  `;
}

function currentRangeMs(): { start: number; end: number } | null {
  const startInput = document.getElementById('ccli-report-start') as HTMLInputElement | null;
  const endInput = document.getElementById('ccli-report-end') as HTMLInputElement | null;
  if (!startInput?.value || !endInput?.value) return null;
  return { start: startOfDayMs(startInput.value), end: endOfDayMs(endInput.value) };
}

async function loadAndRenderReport(): Promise<void> {
  const range = currentRangeMs();
  const reportableList = document.getElementById('ccli-report-reportable-list');
  const excludedList = document.getElementById('ccli-report-excluded-list');
  const reportableCount = document.getElementById('ccli-report-reportable-count');
  const excludedCount = document.getElementById('ccli-report-excluded-count');
  const periodInfo = document.getElementById('ccli-report-period-info');
  if (!range || !reportableList || !excludedList) return;

  const startInput = document.getElementById('ccli-report-start') as HTMLInputElement | null;
  const endInput = document.getElementById('ccli-report-end') as HTMLInputElement | null;
  if (periodInfo) {
    periodInfo.textContent = `${startInput?.value || ''} – ${endInput?.value || ''}`;
  }

  try {
    const res = await api.reports.ccliUsage(range.start, range.end);
    if (!res.ok) {
      ctx!.showToast(res.error || 'Could not load the CCLI usage report.', 'error');
      return;
    }
    const rows: CcliUsageRow[] = res.rows || [];
    const reportable = rows.filter(r => !r.is_public_domain);
    const excluded = rows.filter(r => r.is_public_domain);

    reportableList.innerHTML = reportable.length
      ? reportable.map(renderRow).join('')
      : '<div style="padding: 12px; text-align: center; color: var(--text-muted, #777); font-size: 11px;">No songs went live in this date range.</div>';
    excludedList.innerHTML = excluded.map(renderRow).join('');

    if (reportableCount) reportableCount.textContent = `${reportable.length} song${reportable.length === 1 ? '' : 's'}`;
    if (excludedCount) excludedCount.textContent = `${excluded.length} song${excluded.length === 1 ? '' : 's'}`;
  } catch (_) {
    ctx!.showToast('Could not load the CCLI usage report.', 'error');
  }
}

async function exportCurrentRangeCsv(): Promise<void> {
  const range = currentRangeMs();
  if (!range) {
    ctx!.showToast('Pick a start and end date first.', 'warning');
    return;
  }
  try {
    await api.reports.downloadCcliCsv(range.start, range.end);
  } catch (e) {
    ctx!.showToast(`Could not export the CSV: ${e instanceof Error ? e.message : String(e)}`, 'error');
  }
}
