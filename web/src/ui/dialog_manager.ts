/**
 * OpenSanctuary / OS-Next Unified Dialog & Modal Manager
 *
 * Centralizes modal lifecycle, stacked modal z-index tracking,
 * clean dialog reset hooks, and layered Escape key dismissal.
 */

export type ModalTarget = HTMLElement | string;

export interface ModalOpenOptions {
  focusSelector?: string;
  zIndex?: number;
}

export interface ConfirmDialogOptions {
  title?: string;
  message?: string;
  detail?: string;
  acceptBtnText?: string;
  onAccept?: () => Promise<void> | void;
}

export class DialogManager {
  private openStack: HTMLElement[] = [];
  private resetHooks: Map<string, () => void> = new Map();
  private onConfirmAccept: (() => Promise<void> | void) | null = null;
  private onBeforeEscapeClose: (() => void) | null = null;
  private isEscapeListenerInitialized = false;

  constructor() {
    // Escape routing is coordinated through presentation_helpers / app_ui shortcut matrix,
    // or standalone consumers can call attachEscapeListener().
  }

  /**
   * Resolves an HTMLElement from either an element reference or an element ID string.
   */
  public resolveModal(target: ModalTarget | null | undefined): HTMLElement | null {
    if (!target) return null;
    if (typeof target === 'string') {
      if (typeof document === 'undefined' || typeof document.getElementById !== 'function') return null;
      return document.getElementById(target);
    }
    return target;
  }

  /**
   * Registers a cleanup/reset function to execute whenever the specified modal is closed.
   */
  public registerResetHook(modalId: string, hook: () => void): void {
    this.resetHooks.set(modalId, hook);
  }

  /**
   * Unregisters a cleanup/reset hook for a modal ID.
   */
  public unregisterResetHook(modalId: string): void {
    this.resetHooks.delete(modalId);
  }

  /**
   * Sets an optional callback to run before dismissing the topmost modal on Escape.
   */
  public setBeforeEscapeClose(cb: () => void): void {
    this.onBeforeEscapeClose = cb;
  }

  /**
   * Opens a modal dialog, bringing it to the top of the modal stack.
   */
  public openModal(target: ModalTarget, options: ModalOpenOptions = {}): HTMLElement | null {
    const el = this.resolveModal(target);
    if (!el) return null;

    // Remove from existing position in stack if already present
    const existingIdx = this.openStack.indexOf(el);
    if (existingIdx !== -1) {
      this.openStack.splice(existingIdx, 1);
    }

    const setZ = (target: HTMLElement, val: string) => {
      if (target.style && typeof target.style.setProperty === 'function') {
        target.style.setProperty('z-index', val, 'important');
      }
      if (target.style) {
        target.style.zIndex = val;
      }
    };

    // Set stacking z-index if multiple dialogs are open
    if (options.zIndex !== undefined) {
      setZ(el, String(options.zIndex));
    } else if (el.id === 'confirm-modal') {
      setZ(el, '10000');
    } else if (this.openStack.length > 0) {
      const prevTop = this.openStack[this.openStack.length - 1];
      let prevZ = 2000;
      if (typeof window !== 'undefined' && typeof window.getComputedStyle === 'function' && prevTop instanceof Element) {
        prevZ = parseInt(window.getComputedStyle(prevTop).zIndex, 10) || 2000;
      } else {
        prevZ = parseInt((prevTop && prevTop.style && prevTop.style.zIndex) || '2000', 10) || 2000;
      }

      let currentZ = 0;
      if (typeof window !== 'undefined' && typeof window.getComputedStyle === 'function' && el instanceof Element) {
        currentZ = parseInt(window.getComputedStyle(el).zIndex, 10) || 0;
      } else {
        currentZ = parseInt((el.style && el.style.zIndex) || '0', 10) || 0;
      }

      if (currentZ <= prevZ) {
        setZ(el, String(prevZ + 10));
      }
    }

    el.style.display = 'flex';
    this.openStack.push(el);

    // Optional input autofocus
    if (options.focusSelector) {
      const input = el.querySelector(options.focusSelector) as HTMLElement | null;
      if (input && typeof input.focus === 'function') {
        setTimeout(() => input.focus(), 50);
      }
    }

    return el;
  }

  /**
   * Closes a modal dialog, removing it from the stack and triggering its reset hook.
   */
  public closeModal(target: ModalTarget): boolean {
    const el = this.resolveModal(target);
    if (!el) return false;

    el.style.display = 'none';

    const idx = this.openStack.indexOf(el);
    if (idx !== -1) {
      this.openStack.splice(idx, 1);
    }

    // Execute registered reset hook if defined
    if (el.id && this.resetHooks.has(el.id)) {
      try {
        this.resetHooks.get(el.id)!();
      } catch (err) {
        console.warn(`[DialogManager] Error executing reset hook for #${el.id}:`, err);
      }
    }

    return true;
  }

  /**
   * Closes the topmost visible modal in the stack.
   * If the internal stack is empty, scans the DOM for visible `.modal-backdrop` elements.
   * Returns true if a modal was closed, false otherwise.
   */
  public closeTopmostModal(): boolean {
    if (this.onBeforeEscapeClose) {
      try { this.onBeforeEscapeClose(); } catch (_) {}
    }

    // 1. Check internal stack first
    if (this.openStack.length > 0) {
      const topmost = this.openStack.pop()!;
      this.closeModal(topmost);
      return true;
    }

    // 2. Fallback: inspect DOM for visible modal backdrops (in case opened outside manager)
    if (typeof document !== 'undefined' && typeof document.querySelectorAll === 'function') {
      const visibleBackdrops = Array.from(document.querySelectorAll('.modal-backdrop'))
        .filter((el) => {
          const style = (typeof window !== 'undefined' && window.getComputedStyle)
            ? window.getComputedStyle(el)
            : (el as HTMLElement).style;
          return style.display !== 'none';
        }) as HTMLElement[];

      if (visibleBackdrops.length > 0) {
        const topmost = visibleBackdrops.reduce((top, el) => {
          const topZ = parseInt((typeof window !== 'undefined' ? window.getComputedStyle(top).zIndex : top.style.zIndex) || '0', 10) || 0;
          const elZ = parseInt((typeof window !== 'undefined' ? window.getComputedStyle(el).zIndex : el.style.zIndex) || '0', 10) || 0;
          return elZ >= topZ ? el : top;
        });
        this.closeModal(topmost);
        return true;
      }
    }

    return false;
  }

  /**
   * Closes all currently open modals in reverse order.
   */
  public closeAllModals(): void {
    while (this.openStack.length > 0) {
      const el = this.openStack.pop()!;
      this.closeModal(el);
    }
  }

  /**
   * Returns true if any modal is currently visible.
   */
  public isAnyModalOpen(): boolean {
    if (this.openStack.length > 0) return true;
    if (typeof document !== 'undefined' && typeof document.querySelectorAll === 'function') {
      return Array.from(document.querySelectorAll('.modal-backdrop')).some((el) => {
        const style = (typeof window !== 'undefined' && window.getComputedStyle)
          ? window.getComputedStyle(el)
          : (el as HTMLElement).style;
        return style.display !== 'none';
      });
    }
    return false;
  }

  /**
   * Returns the topmost open modal element or null.
   */
  public getTopmostModal(): HTMLElement | null {
    if (this.openStack.length > 0) {
      return this.openStack[this.openStack.length - 1];
    }
    return null;
  }

  /**
   * Standardized confirmation dialog handler.
   */
  public showConfirmDialog(
    title: string,
    message: string,
    detail: string,
    onAccept: () => Promise<void> | void,
    acceptBtnText: string = '🗑️ Delete Permanently'
  ): HTMLElement | null {
    if (typeof document === 'undefined' || typeof document.getElementById !== 'function') return null;

    const modalEl = document.getElementById('confirm-modal');
    const titleEl = document.getElementById('confirm-modal-title');
    const msgEl = document.getElementById('confirm-modal-message');
    const detailEl = document.getElementById('confirm-modal-detail');
    const acceptBtn = document.getElementById('btn-accept-confirm');

    if (titleEl) titleEl.textContent = title || '⚠️ Confirm Action';
    if (msgEl) msgEl.textContent = message || 'Are you sure you want to proceed?';
    if (detailEl) detailEl.textContent = detail || 'This action cannot be undone.';
    if (acceptBtn) acceptBtn.textContent = acceptBtnText;

    this.onConfirmAccept = onAccept;
    return this.openModal(modalEl || 'confirm-modal', { zIndex: 10000 });
  }

  /**
   * Binds confirm modal buttons if present in DOM.
   */
  public setupConfirmModalBindings(): void {
    if (typeof document === 'undefined' || typeof document.getElementById !== 'function') return;

    const btnClose = document.getElementById('btn-close-confirm');
    const btnCancel = document.getElementById('btn-cancel-confirm');
    const btnAccept = document.getElementById('btn-accept-confirm');

    if (btnClose) btnClose.addEventListener('click', () => this.closeModal('confirm-modal'));
    if (btnCancel) btnCancel.addEventListener('click', () => this.closeModal('confirm-modal'));

    if (btnAccept) {
      btnAccept.addEventListener('click', async () => {
        const cb = this.onConfirmAccept;
        this.closeModal('confirm-modal');
        this.onConfirmAccept = null;
        if (cb) {
          try {
            await cb();
          } catch (err) {
            console.error('[DialogManager] Confirm dialog callback error:', err);
          }
        }
      });
    }
  }

  public attachEscapeListener(): void {
    if (typeof window === 'undefined' || this.isEscapeListenerInitialized) return;
    this.isEscapeListenerInitialized = true;

    // Listen for Escape to dismiss topmost dialog layer
    window.addEventListener('keydown', (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        const handled = this.closeTopmostModal();
        if (handled) {
          e.stopPropagation();
          e.preventDefault();
        }
      }
    });
  }
}

// Global Singleton Instance
export const dialogManager = new DialogManager();

// Export convenient standalone facades matching existing app signatures
export const attachEscapeListener = () => dialogManager.attachEscapeListener();
export const showModal = (target: ModalTarget, options?: ModalOpenOptions) => dialogManager.openModal(target, options);
export const closeModal = (target: ModalTarget) => dialogManager.closeModal(target);
export const closeTopmostModal = () => dialogManager.closeTopmostModal();
export const closeAllModals = () => dialogManager.closeAllModals();
export const isAnyModalOpen = () => dialogManager.isAnyModalOpen();
export const showConfirmDialog = (
  title: string,
  message: string,
  detail: string,
  onAccept: () => Promise<void> | void,
  acceptBtnText?: string
) => dialogManager.showConfirmDialog(title, message, detail, onAccept, acceptBtnText);
export const registerModalReset = (modalId: string, hook: () => void) => dialogManager.registerResetHook(modalId, hook);
