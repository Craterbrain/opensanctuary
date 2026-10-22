/**
 * "Available, not turned on yet" hints for installed plugins, built only from their own manifests.
 *
 * The console never names a particular plugin. Each one declares where it appears (`ui.placements`
 * in plugin.json, or a floating panel if it asks for `ui.panel` and declares nothing), and until the
 * operator approves it the console shows a small hint in exactly those places -- a muted entry in
 * that library tab's sidebar, a note in the video picker, a card in the Store -- each opening its
 * review card in Settings > Plugins. Once approved, the plugin's own page takes the hint's place.
 *
 * A plugin that isn't installed leaves no trace (v0.2.0 shipped without its plugins, and hard-coded
 * pointers to them led nowhere), and one the operator denied or revoked isn't advertised again.
 */
import { api, type PluginInfo } from "./api_client.ts";

type Placement = { slot: string; label?: string; tab?: string };

let installed: PluginInfo[] = [];

/** Where a plugin will appear once approved: what its manifest declares, else a floating panel if it may show UI. */
export function placementsFor(p: PluginInfo): Placement[] {
  if (p.placements?.length) return p.placements;
  const perms = p.state === 'active' ? p.granted : p.requested;
  return perms.some((g) => g.id === 'ui.panel') ? [{ slot: 'dock' }] : [];
}

/** Installed, waiting for its first review, and approvable: worth pointing the operator at. */
export function isOffered(p: PluginInfo): boolean {
  return p.state === 'pending' && !p.error;
}

/** Opens Settings > Plugins, scrolled to this plugin's review card. */
export function openPluginReview(pluginId?: string): void {
  window.dispatchEvent(new CustomEvent('os-open-plugins-settings', { detail: { pluginId } }));
}

/** Offered plugins that would add this library category (`library.write` with it in `categories`). */
export function pluginsAddingTo(plugins: PluginInfo[], category: string): PluginInfo[] {
  return plugins.filter((p) => isOffered(p) && p.requested.some((r) => r.id === 'library.write' && (r.categories ?? []).includes(category)));
}

/** Re-reads the installed plugins and updates every hint. Console pages only (the list needs the host token). */
export async function refreshPluginHints(): Promise<void> {
  try {
    applyPluginHints(await api.plugins.list());
  } catch {
    // Not a console page, or the server is restarting: leave the hints as they are.
  }
}

/** Updates every hint from an already-fetched plugin list (Settings > Plugins passes the one it loaded). */
export function applyPluginHints(plugins: PluginInfo[]): void {
  installed = plugins;
  const offered = plugins.filter(isOffered);
  const libraryHints = offered.flatMap((p) => placementsFor(p)
    .filter((pl) => pl.slot === 'library-source')
    .map((pl) => ({ id: p.id, tab: pl.tab || 'songs', label: pl.label || p.name, name: p.name })));
  // Loaded on use, not imported: library_panel.ts reads the DOM at module load, and this module is
  // reachable from settings code that unit tests import without one.
  void import("../ui/library_panel.ts").then((m) => m.setLibrarySourceHints(libraryHints));
  syncVideoPickerTab();
  renderStoreCards(offered);
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

function reviewButton(p: PluginInfo, cls = 'btn btn-primary'): HTMLButtonElement {
  const b = el('button', cls, `Review ${p.name}`);
  b.type = 'button';
  b.dataset.pluginId = p.id;
  b.addEventListener('click', () => openPluginReview(p.id));
  return b;
}

/**
 * The video background picker's plugin tab: shown only if some installed plugin puts a page there,
 * named after it, and holding either the running plugins' pages or a hint per plugin not yet turned on.
 * Called again by plugin_bridge.ts whenever a plugin's page is added to or removed from it.
 */
export function syncVideoPickerTab(): void {
  const pane = document.getElementById('plugin-picker-pane');
  const hint = document.getElementById('plugin-picker-hint');
  const tab = document.getElementById('btn-video-subtab-ytdl');
  if (!pane || !hint || !tab) return;
  const runningIds = Array.from(pane.querySelectorAll<HTMLElement>('.plugin-source--picker')).map((w) => w.dataset.pluginId ?? '');
  const offered = installed.filter((p) => isOffered(p) && !runningIds.includes(p.id) && placementsFor(p).some((pl) => pl.slot === 'video-picker'));

  hint.replaceChildren();
  for (const p of offered) {
    const row = el('div', 'plugin-offer');
    row.append(el('p', '', `${p.name} can add video here. It's a plugin that isn't turned on yet.`), reviewButton(p));
    hint.appendChild(row);
  }
  hint.hidden = runningIds.length > 0 || offered.length === 0;

  const names = [...runningIds.map((id) => installed.find((p) => p.id === id)?.name ?? id), ...offered.map((p) => p.name)];
  tab.hidden = names.length === 0;
  tab.textContent = names.length === 1 ? `📥 ${names[0]}` : '📥 From plugins';
  // Don't leave the picker showing a tab that just disappeared.
  if (tab.hidden && tab.classList.contains('active')) document.getElementById('btn-video-subtab-library')?.click();
}

/** The Store's plugin cards: one per installed plugin not turned on yet, in its own words. */
function renderStoreCards(offered: PluginInfo[]): void {
  const box = document.getElementById('store-plugin-cards');
  if (!box) return;
  box.replaceChildren(...offered.map((p) => {
    const card = el('div', 'store-card');
    card.dataset.pluginId = p.id;
    card.append(
      el('h4', '', p.name),
      el('p', '', p.description || 'An installed plugin that isn\'t turned on yet.'),
      reviewButton(p),
    );
    return card;
  }));
}
