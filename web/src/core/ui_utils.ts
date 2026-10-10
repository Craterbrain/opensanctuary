/**
 * OpenSanctuary / OS-Next Core UI Utilities
 * Shared presentation, clipboard, toast notification, timing, and debounce helpers.
 */

import { escapeHtml } from './presentation_helpers';

export type ToastType = 'info' | 'success' | 'warning' | 'error';

/**
 * Displays an animated toast notification in the bottom-right of the viewport.
 * Dynamically mounts or reuses #os-toast-container.
 */
export function showToast(message: string, type: ToastType = 'info', duration: number = 3500): HTMLElement | null {
  if (typeof document === 'undefined' || !document.body) return null;

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

  toast.innerHTML = `
    <span class="os-toast-icon">${icon}</span>
    <span class="os-toast-msg">${escapeHtml(message)}</span>
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
  let timer: any = null;

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
