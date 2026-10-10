/**
 * OpenSanctuary / OS-Next Universal Filter Bar, Search Input & Empty State System
 *
 * Provides standardized search inputs with debouncing and clear ("×") buttons,
 * category pill tab switchers with accessible ARIA semantics, and visually
 * consistent empty states across media, video, theme, song, and library pickers.
 */

import { debounce } from '../core/ui_utils.ts';
import { escapeHtml } from '../core/presentation_helpers.ts';

export interface SearchInputOptions {
  placeholder?: string;
  initialValue?: string;
  debounceMs?: number;
  ariaLabel?: string;
  className?: string;
  inputClassName?: string;
  onSearch: (query: string) => void;
  onClear?: () => void;
}

export interface SearchInputHandle {
  container: HTMLElement;
  input: HTMLInputElement;
  clearBtn: HTMLButtonElement;
  setValue: (value: string, triggerSearch?: boolean) => void;
  getValue: () => string;
  clear: () => void;
  focus: () => void;
}

export interface CategoryItem {
  id: string;
  label: string;
  icon?: string;
  count?: number;
}

export interface CategoryPillsOptions {
  categories: CategoryItem[];
  activeId: string;
  onChange: (categoryId: string) => void;
  className?: string;
}

export interface CategoryPillsHandle {
  container: HTMLElement;
  setActive: (id: string, triggerChange?: boolean) => void;
  getActive: () => string;
}

export interface EmptyStateOptions {
  icon?: string;
  title: string;
  description?: string;
  actionLabel?: string;
  onAction?: () => void;
  className?: string;
}

/**
 * Creates a standardized search input with built-in debounce and clear ("×") button.
 */
export function createSearchInput(options: SearchInputOptions): SearchInputHandle {
  if (typeof document === 'undefined' || typeof document.createElement !== 'function') {
    return {
      container: null as any,
      input: null as any,
      clearBtn: null as any,
      setValue: () => {},
      getValue: () => '',
      clear: () => {},
      focus: () => {}
    };
  }

  const container = document.createElement('div');
  container.className = `ui-search-input-wrap ${options.className || ''}`.trim();

  const input = document.createElement('input');
  input.type = 'text';
  input.className = `search-input ${options.inputClassName || ''}`.trim();
  input.placeholder = options.placeholder || '🔍 Search...';
  input.value = options.initialValue || '';
  if (options.ariaLabel) {
    input.setAttribute('aria-label', options.ariaLabel);
  }

  const clearBtn = document.createElement('button');
  clearBtn.type = 'button';
  clearBtn.className = 'ui-search-clear-btn';
  clearBtn.innerHTML = '{icon:close}';
  clearBtn.title = 'Clear search';
  clearBtn.setAttribute('aria-label', 'Clear search');

  const updateClearVisibility = () => {
    if (input.value.trim().length > 0) {
      clearBtn.classList.add('visible');
    } else {
      clearBtn.classList.remove('visible');
    }
  };

  const debouncedSearch = debounce((val: string) => {
    options.onSearch(val.trim());
  }, options.debounceMs ?? 150);

  input.addEventListener('input', () => {
    updateClearVisibility();
    debouncedSearch(input.value);
  });

  input.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
      if (input.value) {
        e.stopPropagation();
        handleClear();
      }
    }
  });

  const handleClear = () => {
    input.value = '';
    updateClearVisibility();
    if (input && typeof input.focus === 'function') {
      input.focus();
    }
    options.onSearch('');
    if (options.onClear) {
      options.onClear();
    }
  };

  clearBtn.addEventListener('click', handleClear);

  container.appendChild(input);
  container.appendChild(clearBtn);
  updateClearVisibility();

  return {
    container,
    input,
    clearBtn,
    setValue: (val: string, triggerSearch = true) => {
      input.value = val;
      updateClearVisibility();
      if (triggerSearch) {
        options.onSearch(val.trim());
      }
    },
    getValue: () => input.value.trim(),
    clear: handleClear,
    focus: () => {
      if (input && typeof input.focus === 'function') {
        input.focus();
      }
    }
  };
}

/**
 * Creates an accessible category pill tab switcher bar.
 */
export function createCategoryPills(options: CategoryPillsOptions): CategoryPillsHandle {
  if (typeof document === 'undefined' || typeof document.createElement !== 'function') {
    return {
      container: null as any,
      setActive: () => {},
      getActive: () => options.activeId
    };
  }

  const container = document.createElement('div');
  container.className = `import-mode-tabs ${options.className || ''}`.trim();
  container.setAttribute('role', 'tablist');

  let currentActiveId = options.activeId;

  options.categories.forEach(cat => {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'import-tab-btn';
    btn.dataset.cat = cat.id;
    btn.setAttribute('role', 'tab');
    btn.setAttribute('aria-selected', cat.id === currentActiveId ? 'true' : 'false');
    if (cat.id === currentActiveId) {
      btn.classList.add('active');
    }

    let text = cat.label;
    if (cat.icon) {
      text = `${cat.icon} ${text}`;
    }
    if (typeof cat.count === 'number') {
      text = `${text} (${cat.count})`;
    }
    btn.textContent = text;

    btn.addEventListener('click', () => {
      setActive(cat.id, true);
    });

    container.appendChild(btn);
  });

  const setActive = (id: string, triggerChange = true) => {
    currentActiveId = id;
    container.querySelectorAll('.import-tab-btn').forEach(b => {
      const isAct = (b as HTMLElement).dataset.cat === id;
      b.classList.toggle('active', isAct);
      b.setAttribute('aria-selected', isAct ? 'true' : 'false');
    });
    if (triggerChange) {
      options.onChange(id);
    }
  };

  return {
    container,
    setActive,
    getActive: () => currentActiveId
  };
}

/**
 * Renders a standardized, accessible empty state message card.
 */
export function renderEmptyState(container: HTMLElement, options: EmptyStateOptions): HTMLElement {
  if (typeof document === 'undefined' || typeof document.createElement !== 'function') {
    return null as any;
  }

  const emptyEl = document.createElement('div');
  emptyEl.className = `ui-empty-state ${options.className || ''}`.trim();
  emptyEl.setAttribute('role', 'status');

  const iconHtml = options.icon ? `<div class="empty-state-icon">${escapeHtml(options.icon)}</div>` : '';
  const titleHtml = `<div class="empty-state-title">${escapeHtml(options.title)}</div>`;
  const descHtml = options.description ? `<div class="empty-state-desc">${escapeHtml(options.description)}</div>` : '';

  emptyEl.innerHTML = `${iconHtml}${titleHtml}${descHtml}`;

  if (options.actionLabel && options.onAction) {
    const actionBtn = document.createElement('button');
    actionBtn.type = 'button';
    actionBtn.className = 'btn-empty-action';
    actionBtn.textContent = options.actionLabel;
    actionBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      options.onAction!();
    });
    emptyEl.appendChild(actionBtn);
  }

  container.innerHTML = '';
  container.appendChild(emptyEl);
  return emptyEl;
}
