/**
 * Ribbon buttons added by sandboxed plugins (`ui.toolbar`, docs/PLUGIN_PERMISSIONS.md).
 *
 * The labels, icons and tooltips come from a plugin's manifest, which the operator reviewed and which
 * can't change at runtime. They are untrusted text: inserted with textContent only, never as markup, and
 * the icon is only ever a *name* resolved against the app's own icon files. Plugin buttons live in their
 * own "Plugins" group, carry a puzzle badge and the plugin's name in their tooltip, and never sit among
 * the built-in controls, so a plugin can't pass one off as Go Live, Blackout or Clear.
 */
import { iconUrl, getCurrentIconSet } from "./icon_sets.ts";

export interface ToolbarEntry {
  pluginId: string;
  pluginName: string;
  id: string;
  label: string;
  icon: string;
  tooltip: string;
}

/** More than this many buttons across all plugins collapse into a "Plugins" menu. */
export const MAX_VISIBLE_TOOLBAR_BUTTONS = 6;
/** At most one click event per button in this many milliseconds. */
export const TOOLBAR_DEBOUNCE_MS = 500;

const entries: ToolbarEntry[] = [];
const state = new Map<string, { disabled: boolean; pressed: boolean }>();
const lastClick = new Map<string, number>();
let onClick: ((pluginId: string, itemId: string) => void) | null = null;
let menuOpen = false;

const keyOf = (pluginId: string, id: string) => `${pluginId}\u0000${id}`;

function cluster(): HTMLElement | null {
  return document.getElementById('ribbon-plugins');
}

function makeButton(e: ToolbarEntry, inMenu: boolean): HTMLButtonElement {
  const st = state.get(keyOf(e.pluginId, e.id)) ?? { disabled: false, pressed: false };
  const b = document.createElement('button');
  b.type = 'button';
  b.className = inMenu ? 'ribbon-plugins-menu-item' : 'ribbon-btn ribbon-btn--plugin';
  b.dataset.pluginId = e.pluginId;
  b.dataset.itemId = e.id;
  const described = `${e.pluginName} (plugin): ${e.tooltip || e.label}`;
  b.title = described;
  b.setAttribute('aria-label', described);
  b.disabled = st.disabled;
  b.classList.toggle('active', st.pressed);
  b.setAttribute('aria-pressed', String(st.pressed));
  const icon = document.createElement('img');
  icon.dataset.icon = e.icon;
  icon.src = iconUrl(getCurrentIconSet(), e.icon);
  icon.alt = '';
  icon.draggable = false;
  const label = document.createElement('span');
  label.className = 'label';
  label.textContent = inMenu ? `${e.pluginName}: ${e.label}` : e.label;
  if (inMenu) {
    b.append(icon, label);
  } else {
    const wrap = document.createElement('span');
    wrap.className = 'icon';
    wrap.appendChild(icon);
    const badge = document.createElement('span');
    badge.className = 'plugin-badge';
    badge.setAttribute('aria-hidden', 'true');
    badge.dataset.badge = 'plugin';
    wrap.appendChild(badge);
    b.append(wrap, label);
  }
  b.addEventListener('click', (ev) => {
    ev.stopPropagation();
    const k = keyOf(e.pluginId, e.id);
    const now = Date.now();
    // Enter is the Go Live shortcut: a plugin button must never keep focus and capture it.
    b.blur();
    if (now - (lastClick.get(k) ?? -Infinity) < TOOLBAR_DEBOUNCE_MS) return;
    lastClick.set(k, now);
    menuOpen = false;
    render();
    onClick?.(e.pluginId, e.id);
  });
  return b;
}

function render(): void {
  const root = cluster();
  if (!root) return;
  root.replaceChildren();
  root.hidden = entries.length === 0;
  if (!entries.length) return;
  const caption = document.createElement('span');
  caption.className = 'ribbon-plugins-caption';
  caption.textContent = 'Plugins';
  root.appendChild(caption);
  entries.slice(0, MAX_VISIBLE_TOOLBAR_BUTTONS).forEach((e) => root.appendChild(makeButton(e, false)));
  const extra = entries.slice(MAX_VISIBLE_TOOLBAR_BUTTONS);
  if (extra.length) {
    const wrap = document.createElement('div');
    wrap.className = 'ribbon-plugins-overflow';
    const toggle = document.createElement('button');
    toggle.type = 'button';
    toggle.className = 'ribbon-btn ribbon-btn--plugin ribbon-plugins-more';
    toggle.setAttribute('aria-haspopup', 'true');
    toggle.setAttribute('aria-expanded', String(menuOpen));
    toggle.title = 'More plugin buttons';
    const label = document.createElement('span');
    label.className = 'label';
    label.textContent = `More ▾ (${extra.length})`;
    toggle.appendChild(label);
    toggle.addEventListener('click', (ev) => {
      ev.stopPropagation();
      toggle.blur();
      menuOpen = !menuOpen;
      render();
    });
    wrap.appendChild(toggle);
    if (menuOpen) {
      const menu = document.createElement('div');
      menu.className = 'ribbon-plugins-menu';
      extra.forEach((e) => menu.appendChild(makeButton(e, true)));
      wrap.appendChild(menu);
    }
    root.appendChild(wrap);
  }
}

if (typeof document !== 'undefined') {
  document.addEventListener('click', () => {
    if (menuOpen) { menuOpen = false; render(); }
  });
}

/** Adds a plugin's buttons (in approval order, then id) and re-renders the group. */
export function addToolbarItems(
  plugin: { id: string; name: string; toolbar?: Array<{ id: string; label: string; icon: string; tooltip?: string }> },
  click: (pluginId: string, itemId: string) => void,
): void {
  onClick = click;
  removeToolbarItems(plugin.id, false);
  for (const t of plugin.toolbar ?? []) {
    entries.push({ pluginId: plugin.id, pluginName: plugin.name, id: t.id, label: t.label, icon: t.icon, tooltip: t.tooltip ?? '' });
  }
  render();
}

/** Removes a plugin's buttons (revoke, deny, a failed token) and hides the group when it is empty. */
export function removeToolbarItems(pluginId: string, redraw = true): void {
  for (let i = entries.length - 1; i >= 0; i--) {
    const entry = entries[i];
    if (entry && entry.pluginId === pluginId) {
      state.delete(keyOf(pluginId, entry.id));
      lastClick.delete(keyOf(pluginId, entry.id));
      entries.splice(i, 1);
    }
  }
  if (redraw) render();
}

/** `ui.toolbar.update`: only on/off and pressed, never the label, icon or tooltip. Returns false for an unknown button. */
export function updateToolbarItem(pluginId: string, itemId: string, patch: { disabled?: unknown; pressed?: unknown }): boolean {
  if (!entries.some((e) => e.pluginId === pluginId && e.id === itemId)) return false;
  const k = keyOf(pluginId, itemId);
  const cur = state.get(k) ?? { disabled: false, pressed: false };
  if (typeof patch.disabled === 'boolean') cur.disabled = patch.disabled;
  if (typeof patch.pressed === 'boolean') cur.pressed = patch.pressed;
  state.set(k, cur);
  render();
  return true;
}

/** For tests: forget everything. */
export function resetToolbarForTests(): void {
  entries.length = 0;
  state.clear();
  lastClick.clear();
  menuOpen = false;
  onClick = null;
}
