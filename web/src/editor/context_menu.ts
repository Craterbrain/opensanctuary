export interface ContextMenuState {
  hasSelection: boolean;
  selectedCount: number;
  hasClipboard: boolean;
  isGroup: boolean;
  isSingleImage: boolean;
}

import { contextMenuManager } from '../ui/context_menu_manager';

export interface ContextMenuCallbacks {
  onCut: () => void;
  onCopy: () => void;
  onPaste: () => void;
  onDuplicate: () => void;
  onDelete: () => void;
  onBringForward: () => void;
  onSendBackward: () => void;
  onBringToFront: () => void;
  onSendToBack: () => void;
  onGroup: () => void;
  onUngroup: () => void;
  onSelectAll: () => void;
  onCropImage: () => void;
}

export class EditorContextMenu {
  private menuEl: HTMLElement;
  private callbacks: ContextMenuCallbacks;
  private isVisible: boolean = false;

  constructor(callbacks: ContextMenuCallbacks) {
    this.callbacks = callbacks;
    this.menuEl = document.createElement('div');
    this.menuEl.className = 'editor-context-menu context-menu';
    this.menuEl.style.position = 'fixed';
    this.menuEl.style.zIndex = '100000';
    this.menuEl.style.display = 'none';
    this.menuEl.style.minWidth = '180px';
    this.menuEl.style.background = '#1e1e24';
    this.menuEl.style.border = '1px solid #333338';
    this.menuEl.style.borderRadius = '6px';
    this.menuEl.style.boxShadow = '0 8px 24px rgba(0,0,0,0.6)';
    this.menuEl.style.padding = '4px 0';
    this.menuEl.style.fontSize = '12px';
    this.menuEl.style.userSelect = 'none';

    document.body.appendChild(this.menuEl);

    // Click outside listener
    window.addEventListener('click', (e) => {
      if (this.isVisible && !this.menuEl.contains(e.target as Node)) {
        this.hide();
      }
    });

    window.addEventListener('keydown', (e) => {
      if (this.isVisible && e.key === 'Escape') {
        this.hide();
      }
    });
  }

  public show(x: number, y: number, state: ContextMenuState): void {
    this.menuEl.innerHTML = '';

    // Group 1: Clipboard
    this.addItem('Cut', 'Ctrl+X', state.hasSelection, () => this.callbacks.onCut());
    this.addItem('Copy', 'Ctrl+C', state.hasSelection, () => this.callbacks.onCopy());
    this.addItem('Paste', 'Ctrl+V', state.hasClipboard, () => this.callbacks.onPaste());
    this.addItem('Duplicate', 'Ctrl+D', state.hasSelection, () => this.callbacks.onDuplicate());
    this.addItem('Delete', 'Del', state.hasSelection, () => this.callbacks.onDelete());

    this.addSeparator();

    // Group 2: Ordering
    this.addItem('Bring Forward', 'Ctrl+]', state.hasSelection, () => this.callbacks.onBringForward());
    this.addItem('Send Backward', 'Ctrl+[', state.hasSelection, () => this.callbacks.onSendBackward());
    this.addItem('Bring to Front', 'Ctrl+Shift+]', state.hasSelection, () => this.callbacks.onBringToFront());
    this.addItem('Send to Back', 'Ctrl+Shift+[', state.hasSelection, () => this.callbacks.onSendToBack());

    if (state.isSingleImage) {
      this.addSeparator();
      this.addItem('Crop Image…', '', true, () => this.callbacks.onCropImage());
    }

    this.addSeparator();

    // Group 3: Grouping
    if (state.selectedCount > 1) {
      this.addItem('Group', 'Ctrl+G', true, () => this.callbacks.onGroup());
    }
    if (state.isGroup) {
      this.addItem('Ungroup', 'Ctrl+Shift+G', true, () => this.callbacks.onUngroup());
    }

    this.addItem('Select All', 'Ctrl+A', true, () => this.callbacks.onSelectAll());

    contextMenuManager.showElement(this.menuEl, x, y);
    this.isVisible = true;
  }

  public hide(): void {
    if (this.menuEl) {
      this.menuEl.style.display = 'none';
    }
    contextMenuManager.hideAll();
    this.isVisible = false;
  }

  private addItem(label: string, shortcut: string, enabled: boolean, onClick: () => void): void {
    const item = document.createElement('div');
    item.className = 'editor-context-item';
    item.style.display = 'flex';
    item.style.justifyContent = 'space-between';
    item.style.alignItems = 'center';
    item.style.padding = '6px 12px';
    item.style.cursor = enabled ? 'pointer' : 'default';
    item.style.opacity = enabled ? '1.0' : '0.4';
    item.style.color = enabled ? '#e4e4e7' : '#71717a';

    if (enabled) {
      item.addEventListener('mouseenter', () => {
        item.style.background = '#27272a';
      });
      item.addEventListener('mouseleave', () => {
        item.style.background = 'transparent';
      });
      item.addEventListener('click', (e) => {
        e.stopPropagation();
        this.hide();
        onClick();
      });
    }

    const labelSpan = document.createElement('span');
    labelSpan.textContent = label;

    const shortcutSpan = document.createElement('span');
    shortcutSpan.style.fontSize = '10px';
    shortcutSpan.style.color = '#a1a1aa';
    shortcutSpan.style.marginLeft = '16px';
    shortcutSpan.textContent = shortcut;

    item.appendChild(labelSpan);
    item.appendChild(shortcutSpan);
    this.menuEl.appendChild(item);
  }

  private addSeparator(): void {
    const sep = document.createElement('div');
    sep.style.height = '1px';
    sep.style.background = '#27272a';
    sep.style.margin = '4px 0';
    this.menuEl.appendChild(sep);
  }
}
