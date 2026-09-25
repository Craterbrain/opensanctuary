/**
 * OpenSanctuary / OS-Next Unified Context Menu Manager
 *
 * Centralizes context menu positioning, 2D viewport boundary clamping,
 * click-outside dismissal, Escape key routing, and keyboard navigation
 * across both static DOM context menus and dynamically generated menus.
 */

export interface MenuItemOption {
  label: string;
  shortcut?: string;
  icon?: string;
  danger?: boolean;
  disabled?: boolean;
  action?: () => void;
  separator?: boolean;
}

export interface DynamicMenuOptions {
  items: MenuItemOption[];
  x: number;
  y: number;
  minWidth?: number;
  className?: string;
  onClose?: () => void;
}

export class ContextMenuManager {
  private activeMenuEl: HTMLElement | null = null;
  private dynamicContainer: HTMLElement | null = null;
  private onCloseCallback: (() => void) | null = null;
  private isListenersBound: boolean = false;
  private selectedItemIndex: number = -1;

  constructor() {
    this.bindGlobalListeners();
  }

  /**
   * Binds global document and window listeners for outside clicks, Escape, and resize.
   */
  public bindGlobalListeners(): void {
    if (this.isListenersBound || typeof document === 'undefined' || typeof window === 'undefined') return;
    this.isListenersBound = true;

    // Dismiss on click outside
    document.addEventListener('click', (e: MouseEvent) => {
      if (this.activeMenuEl) {
        const target = e.target as Node;
        if (!this.activeMenuEl.contains(target)) {
          this.hideAll();
        }
      }
    }, true);

    // Context menu right-click outside also dismisses
    document.addEventListener('contextmenu', (e: MouseEvent) => {
      if (this.activeMenuEl && !this.activeMenuEl.contains(e.target as Node)) {
        // Will be hidden prior to showing a new one
      }
    });

    // Keyboard navigation and Escape dismiss
    window.addEventListener('keydown', (e: KeyboardEvent) => {
      if (!this.activeMenuEl || this.activeMenuEl.style.display === 'none') return;

      if (e.key === 'Escape') {
        this.hideAll();
        e.stopPropagation();
        e.preventDefault();
        return;
      }

      if (e.key === 'ArrowDown') {
        this.navigateItems(1);
        e.preventDefault();
        e.stopPropagation();
      } else if (e.key === 'ArrowUp') {
        this.navigateItems(-1);
        e.preventDefault();
        e.stopPropagation();
      } else if (e.key === 'Enter') {
        const items = this.getInteractiveItems();
        if (this.selectedItemIndex >= 0 && this.selectedItemIndex < items.length) {
          items[this.selectedItemIndex].click();
          e.preventDefault();
          e.stopPropagation();
        }
      }
    }, true);

    // Dismiss on window resize or blur to prevent orphaned menus
    window.addEventListener('resize', () => this.hideAll());
    window.addEventListener('blur', () => this.hideAll());
  }

  /**
   * Shows an existing DOM context menu element with automated viewport boundary clamping.
   */
  public showElement(menuEl: HTMLElement | null | undefined, x: number, y: number): boolean {
    if (!menuEl) return false;

    // Hide any previously active menu
    this.hideAll();

    menuEl.style.display = 'block';
    this.activeMenuEl = menuEl;
    this.selectedItemIndex = -1;

    // Viewport-aware 2D clamping
    this.positionAndClamp(menuEl, x, y);

    return true;
  }

  /**
   * Dynamically renders and displays a context menu from a list of item definitions.
   */
  public showMenu(opts: DynamicMenuOptions): HTMLElement {
    this.hideAll();

    if (!this.dynamicContainer) {
      if (typeof document !== 'undefined') {
        this.dynamicContainer = document.createElement('div');
        this.dynamicContainer.className = 'dynamic-context-menu context-menu';
        this.dynamicContainer.style.position = 'fixed';
        this.dynamicContainer.style.zIndex = '100000';
        document.body.appendChild(this.dynamicContainer);
      }
    }

    const container = this.dynamicContainer;
    if (!container) return null as unknown as HTMLElement;

    container.className = `dynamic-context-menu context-menu ${opts.className || ''}`.trim();
    container.style.minWidth = `${opts.minWidth || 190}px`;
    container.style.display = 'block';
    container.innerHTML = '';
    this.onCloseCallback = opts.onClose || null;
    this.selectedItemIndex = -1;

    opts.items.forEach((item) => {
      if (item.separator) {
        const sep = document.createElement('div');
        sep.className = 'context-menu-separator';
        container.appendChild(sep);
        return;
      }

      const row = document.createElement('div');
      const isDisabled = !!item.disabled;
      const isDanger = !!item.danger;

      row.className = `context-menu-item ${isDisabled ? 'disabled' : ''} ${isDanger ? 'context-menu-item-danger' : ''}`.trim();
      row.tabIndex = isDisabled ? -1 : 0;

      const leftWrap = document.createElement('span');
      leftWrap.className = 'context-menu-label-wrap';
      if (item.icon) {
        const iconSpan = document.createElement('span');
        iconSpan.className = 'context-menu-item-icon';
        iconSpan.textContent = item.icon;
        leftWrap.appendChild(iconSpan);
      }
      const labelSpan = document.createElement('span');
      labelSpan.textContent = item.label;
      leftWrap.appendChild(labelSpan);
      row.appendChild(leftWrap);

      if (item.shortcut) {
        const scSpan = document.createElement('span');
        scSpan.className = 'context-menu-shortcut';
        scSpan.textContent = item.shortcut;
        row.appendChild(scSpan);
      }

      if (!isDisabled && item.action) {
        row.addEventListener('click', (e) => {
          e.stopPropagation();
          this.hideAll();
          item.action!();
        });
      }

      container.appendChild(row);
    });

    this.activeMenuEl = container;
    this.positionAndClamp(container, opts.x, opts.y);

    return container;
  }

  /**
   * Calculates clamped left and top coordinates ensuring the menu stays inside the viewport.
   */
  public positionAndClamp(el: HTMLElement, x: number, y: number): void {
    const margin = 8;
    const winW = (typeof window !== 'undefined' && window.innerWidth) ? window.innerWidth : 1440;
    const winH = (typeof window !== 'undefined' && window.innerHeight) ? window.innerHeight : 900;

    const elW = el.offsetWidth || el.getBoundingClientRect?.()?.width || 200;
    const elH = el.offsetHeight || el.getBoundingClientRect?.()?.height || 220;

    const posX = Math.min(Math.max(margin, x), Math.max(margin, winW - elW - margin));
    const posY = Math.min(Math.max(margin, y), Math.max(margin, winH - elH - margin));

    el.style.left = `${posX}px`;
    el.style.top = `${posY}px`;
  }

  /**
   * Hides all open context menus and fires any registered close callback.
   */
  public hideAll(): void {
    if (this.activeMenuEl) {
      this.activeMenuEl.style.display = 'none';
      this.activeMenuEl = null;
    }

    if (this.dynamicContainer) {
      this.dynamicContainer.style.display = 'none';
    }

    if (typeof document !== 'undefined' && typeof document.querySelectorAll === 'function') {
      document.querySelectorAll('.context-menu').forEach((m) => {
        (m as HTMLElement).style.display = 'none';
      });
    }

    if (this.onCloseCallback) {
      const cb = this.onCloseCallback;
      this.onCloseCallback = null;
      try { cb(); } catch (_) {}
    }

    this.selectedItemIndex = -1;
  }

  /**
   * Returns true if any context menu is currently visible.
   */
  public isAnyMenuOpen(): boolean {
    if (this.activeMenuEl && this.activeMenuEl.style.display !== 'none') return true;
    if (typeof document !== 'undefined' && typeof document.querySelectorAll === 'function') {
      return Array.from(document.querySelectorAll('.context-menu')).some(
        m => (m as HTMLElement).style.display !== 'none'
      );
    }
    return false;
  }

  /**
   * Returns the currently active menu element, if any.
   */
  public getActiveMenu(): HTMLElement | null {
    return this.activeMenuEl;
  }

  private getInteractiveItems(): HTMLElement[] {
    if (!this.activeMenuEl) return [];
    const items = Array.from(this.activeMenuEl.querySelectorAll('.context-menu-item, .editor-context-item')) as HTMLElement[];
    return items.filter(el => !el.classList.contains('disabled') && el.style.opacity !== '0.4');
  }

  private navigateItems(direction: number): void {
    const items = this.getInteractiveItems();
    if (items.length === 0) return;

    items.forEach(el => el.classList.remove('keyboard-focused'));

    this.selectedItemIndex += direction;
    if (this.selectedItemIndex < 0) {
      this.selectedItemIndex = items.length - 1;
    } else if (this.selectedItemIndex >= items.length) {
      this.selectedItemIndex = 0;
    }

    const current = items[this.selectedItemIndex];
    if (current) {
      current.classList.add('keyboard-focused');
      if (typeof current.focus === 'function') current.focus();
    }
  }
}

// Global Singleton Instance
export const contextMenuManager = new ContextMenuManager();

// Standalone facades matching existing app signatures
export const showContextMenu = (menuEl: HTMLElement | null | undefined, x: number, y: number) =>
  contextMenuManager.showElement(menuEl, x, y);

export const hideAllContextMenus = () =>
  contextMenuManager.hideAll();
