/**
 * OpenSanctuary / OS-Next Core UI Utilities
 * Shared presentation, clipboard, toast notification, timing, and debounce helpers.
 */

import { escapeHtml } from './presentation_helpers';

export type ToastType = 'info' | 'success' | 'warning' | 'error';

export interface ToastOptions {
  /** Bold first line (a notification's title). */
  title?: string;
  /** Who it came from, shown small above the text ("Video Downloader"). */
  source?: string;
  /** Don't record this toast in the notification history (it already is). */
  skipHistory?: boolean;
}

/** The notification center registers here so every toast, from anywhere, lands in its history. */
let toastObserver: ((message: string, type: ToastType) => void) | null = null;
export function setToastObserver(fn: ((message: string, type: ToastType) => void) | null): void {
  toastObserver = fn;
}

/**
 * Displays an animated toast notification in the bottom-right of the viewport.
 * Dynamically mounts or reuses #os-toast-container.
 */
export function showToast(message: string, type: ToastType = 'info', duration: number = 3500, opts: ToastOptions = {}): HTMLElement | null {
  if (typeof document === 'undefined' || !document.body) return null;
  if (!opts.skipHistory) { try { toastObserver?.(message, type); } catch { /* history is best effort */ } }

  let container = document.getElementById('os-toast-container');
  if (!container) {
    container = document.createElement('div');
    container.id = 'os-toast-container';
    document.body.appendChild(container);
  }

  const toast = document.createElement('div');
  toast.className = `os-toast os-toast-${type}`;

  const iconMap: Record<ToastType, string> = {
    success: '✓',
    error: '{icon:error}',
    warning: '⚠️',
    info: 'ℹ️',
  };
  const icon = iconMap[type] || 'ℹ️';

  const source = opts.source ? `<span class="os-toast-source">${escapeHtml(opts.source)}</span>` : '';
  const title = opts.title ? `<strong class="os-toast-title">${escapeHtml(opts.title)}</strong>` : '';
  toast.innerHTML = `
    <span class="os-toast-icon">${icon}</span>
    <span class="os-toast-msg">${source}${title}${escapeHtml(message)}</span>
    <button class="os-toast-close" title="Dismiss">{icon:close}</button>
  `;

  const closeBtn = toast.querySelector('.os-toast-close');
  const dismiss = () => {
    toast.classList.remove('show');
    setTimeout(() => {
      if (toast.parentElement) toast.remove();
    }, 250);
  };

  if (closeBtn) closeBtn.addEventListener('click', dismiss);
  container.appendChild(toast);

  if (typeof requestAnimationFrame !== 'undefined') {
    requestAnimationFrame(() => toast.classList.add('show'));
  } else {
    toast.classList.add('show');
  }

  if (duration > 0) {
    setTimeout(dismiss, duration);
  }

  return toast;
}

/**
 * Formats a duration in seconds into M:SS format.
 * Returns '0:00' for null, NaN, or non-positive values.
 */
export function formatMediaTime(sec?: number | null): string {
  if (!sec || isNaN(sec) || sec < 0) return '0:00';
  const m = Math.floor(sec / 60);
  const s = Math.floor(sec % 60);
  return `${m}:${s.toString().padStart(2, '0')}`;
}

/**
 * Creates a debounced version of a function that delays invocation until
 * after `delayMs` milliseconds have elapsed since the last call.
 */
export function debounce<T extends (...args: any[]) => any>(
  fn: T,
  delayMs: number
): ((...args: Parameters<T>) => void) & { cancel: () => void } {
  let timer: ReturnType<typeof setTimeout> | null = null;

  const debounced = (...args: Parameters<T>) => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      fn(...args);
    }, delayMs);
  };

  debounced.cancel = () => {
    if (timer) {
      clearTimeout(timer);
      timer = null;
    }
  };

  return debounced;
}

/**
 * Copies text to the system clipboard with multi-layer fallback support:
 * 1. navigator.clipboard.writeText (modern secure context)
 * 2. document.execCommand('copy') fallback
 * Optionally shows a success toast notification if provided.
 */
export async function copyToClipboard(text: string, successToast?: string): Promise<boolean> {
  let copied = false;

  if (typeof navigator !== 'undefined' && navigator.clipboard && navigator.clipboard.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      copied = true;
    } catch (err) {
      console.warn('[Clipboard] navigator.clipboard.writeText failed, falling back:', err);
    }
  }

  if (!copied && typeof document !== 'undefined' && document.body) {
    try {
      const textarea = document.createElement('textarea');
      textarea.value = text;
      textarea.style.position = 'fixed';
      textarea.style.opacity = '0';
      textarea.style.pointerEvents = 'none';
      document.body.appendChild(textarea);
      if (typeof textarea.focus === 'function') textarea.focus();
      if (typeof textarea.select === 'function') textarea.select();
      if (typeof document.execCommand === 'function') {
        copied = document.execCommand('copy');
      }
      if (textarea.parentElement) {
        textarea.parentElement.removeChild(textarea);
      }
    } catch (e) {
      console.warn('[Clipboard] Copy fallback failed:', e);
    }
  }

  if (copied && successToast) {
    showToast(successToast, 'success');
  }

  return copied;
}

/**
 * Reads text from the system clipboard:
 * 1. Tries the OS-Next Desktop Backend Clipboard endpoint (`/api/system/clipboard`)
 * 2. Falls back to standard browser `navigator.clipboard.readText()`
 */
export async function readClipboard(): Promise<string> {
  // 1. Try Desktop Backend Clipboard API first
  try {
    const res = await fetch('/api/system/clipboard');
    if (res.ok) {
      const data = await res.json();
      if (data && typeof data.text === 'string' && data.text) {
        return data.text;
      }
    }
  } catch (_) {
    // Expected when running in pure browser without backend clipboard route
  }

  // 2. Fallback to standard browser navigator.clipboard
  if (typeof navigator !== 'undefined' && navigator.clipboard && navigator.clipboard.readText) {
    try {
      return await navigator.clipboard.readText();
    } catch (_) {}
  }

  return '';
}

/** The message of anything that was thrown: an `Error`, an object with a `message`, or a plain value. */
export function errorMessage(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (e && typeof e === 'object' && 'message' in e && typeof (e as { message: unknown }).message === 'string') return (e as { message: string }).message;
  return String(e);
}
