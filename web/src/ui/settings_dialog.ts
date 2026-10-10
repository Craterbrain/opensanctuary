/**
 * OpenSanctuary / OS-Next Settings Dialog Controller
 * Searchable, categorized settings rendering driven by settings_schema.ts.
 */

import {
  SETTINGS_SCHEMA,
  SETTINGS_CATEGORIES,
  SettingDef
} from "../core/settings_schema.ts";
import {
  DisplayOutputConfig,
  DISPLAY_CONTENT_SOURCES,
  createDefaultDisplayOutput,
} from "../core/display_config.ts";
import { debounce, showToast } from "../core/ui_utils.ts";
import { api, UpdateCheckResult } from "../core/api_client.ts";
import { escapeHtml } from "../core/presentation_helpers.ts";
import { closeModal, showConfirmDialog } from "./dialog_manager.ts";
import { openCcliReportModal } from "./ccli_report_modal.ts";
import { ICON_SETS, ICON_SET_PREVIEW_ICONS, IconSetDef, iconUrl, resolveTheme } from "../core/icon_sets.ts";

export interface SettingsContext {
  getAppOptions: () => Record<string, any>;
  getInstalledBibles: () => any[];
  getAvailableThemes: () => any[];
  areTranslationsEquivalent: (a: string, b: string) => boolean;
  getDisplayOutputs: () => DisplayOutputConfig[];
  saveDisplayOutputs: (outputs: DisplayOutputConfig[]) => Promise<void> | void;
  /** Saves and applies theme options (`iconSet`, `colorTheme`, `themeLinked`). */
  setThemeOptions: (changes: Record<string, string>) => Promise<void> | void;
  switchToPairingTab: () => void;
  switchToAdbProvisionTab: () => void;
  showFirstTimeSetup: () => void;
}

interface MonitorInfo {
  index: number;
  name: string;
  width: number;
  height: number;
  x: number;
  y: number;
  is_primary: boolean;
}

interface DisplayStatus {
  open: boolean;
  monitor_index: number | null;
  width: number | null;
  height: number | null;
}

/** Settings whose value is a filesystem directory -- get a "Browse…" button next to the text field. */
const DIRECTORY_PICKER_KEYS = new Set(['biblesDirectory', 'songsDirectory', 'mediaCacheDirectory']);

let activeSettingsCategory = 'general';
let settingsSearchQuery = '';
let settingsServerInfo: {
  instance_id?: string;
  version?: string;
  data_dir?: string;
  bibles_dir?: string;
  songs_dir?: string;
  media_dir?: string;
} | null = null;
/** Populated alongside `settingsServerInfo` by `loadSettingsServerInfo()` -- the
 * automated check flow's cached result (docs/update.md), shown inline next to
 * the `appVersion` row. `undefined` until the first load attempt completes. */
let settingsUpdateStatus: UpdateCheckResult | null | undefined = undefined;
let currentContext: SettingsContext | null = null;

let displaysAvailable = false;
let displaysMonitors: MonitorInfo[] = [];
let displaysStatuses: Record<string, DisplayStatus> = {};
let displaysLoaded = false;

export function initSettingsDialog(context: SettingsContext) {
  currentContext = context;
}

function getSettingValue(key: string): string {
  if (key === 'serverInstanceId') return (settingsServerInfo && settingsServerInfo.instance_id) || '(unavailable)';
  if (key === 'appVersion') {
    const version = (settingsServerInfo && settingsServerInfo.version) || '';
    const latest = settingsUpdateStatus?.latest;
    return latest ? `${version}  —  Update available: v${latest.version}` : version;
  }
  if (key === 'dataDirectory') return (settingsServerInfo && settingsServerInfo.data_dir) || '(unavailable)';
  const options = currentContext ? currentContext.getAppOptions() : {};
  const stored = options[key] != null ? String(options[key]) : '';
  // biblesDirectory/songsDirectory/mediaCacheDirectory: pre-fill with the
  // currently-effective resolved path (docs/paths.md) when no explicit
  // override has been saved, so the field shows what's actually in use
  // rather than looking blank/unset.
  if (!stored && key === 'biblesDirectory') return (settingsServerInfo && settingsServerInfo.bibles_dir) || '';
  if (!stored && key === 'songsDirectory') return (settingsServerInfo && settingsServerInfo.songs_dir) || '';
  if (!stored && key === 'mediaCacheDirectory') return (settingsServerInfo && settingsServerInfo.media_dir) || '';
  return stored;
}

function settingMatchesSearch(def: SettingDef, q: string): boolean {
  if (!q) return true;
  return (def.label + ' ' + def.description).toLowerCase().includes(q);
}

export function renderSettingsSidebar() {
  const list = document.getElementById('settings-category-list');
  if (!list) return;
  const q = settingsSearchQuery.trim().toLowerCase();
  list.innerHTML = '';
  SETTINGS_CATEGORIES.forEach(cat => {
    const defs = SETTINGS_SCHEMA.filter(d => d.category === cat.id);
    const hasMatch = !q || defs.some(d => settingMatchesSearch(d, q));
    const btn = document.createElement('button');
    btn.className = 'settings-category-btn'
      + (cat.id === activeSettingsCategory ? ' active' : '')
      + (q && !hasMatch ? ' no-match' : '');
    btn.innerHTML = `<span class="settings-category-icon">${cat.icon}</span><span>${cat.label}</span>`;
    btn.addEventListener('click', () => {
      activeSettingsCategory = cat.id;
      renderSettingsSidebar();
      renderSettingsContent();
    });
    list.appendChild(btn);
  });
}

/** Leaves the search results and shows a category's own panel (the "Open ..." buttons on 'panel' rows). */
function openSettingsCategory(categoryId: string) {
  settingsSearchQuery = '';
  const searchEl = document.getElementById('settings-search') as HTMLInputElement | null;
  if (searchEl) searchEl.value = '';
  activeSettingsCategory = categoryId;
  renderSettingsSidebar();
  renderSettingsContent();
}

export function renderSettingRow(def: SettingDef, highlight: boolean): HTMLElement {
  const row = document.createElement('div');
  row.className = 'settings-row' + (highlight ? ' settings-row--highlight' : '');

  const labelWrap = document.createElement('div');
  labelWrap.className = 'settings-row-label';
  labelWrap.innerHTML = `<label class="form-label">${def.label}</label>`
    + (def.description ? `<div class="settings-row-desc">${def.description}</div>` : '');
  row.appendChild(labelWrap);

  const controlWrap = document.createElement('div');
  controlWrap.className = 'settings-row-control';

  if (def.control === 'readonly') {
    const span = document.createElement('span');
    span.className = 'settings-readonly-value';
    span.textContent = getSettingValue(def.key) || '—';
    controlWrap.appendChild(span);
  } else if (def.control === 'panel') {
    const btn = document.createElement('button');
    btn.className = 'btn';
    btn.id = `setting-panel-${def.key}`;
    btn.textContent = def.actionLabel || 'Open';
    btn.addEventListener('click', () => openSettingsCategory(def.category));
    controlWrap.appendChild(btn);
  } else if (def.control === 'action') {
    const btn = document.createElement('button');
    btn.className = 'btn';
    btn.id = `setting-action-${def.key}`;
    btn.textContent = def.actionLabel || 'Run';
    if (def.key === 'openLiveOutputWindow') {
      btn.addEventListener('click', () => {
        window.open('/live.html', 'os-next-live-output', 'width=1920,height=1080');
      });
    } else if (def.key === 'rerunFirstTimeSetup') {
      btn.addEventListener('click', () => {
        const optionsModalEl = document.getElementById('options-modal');
        if (optionsModalEl) closeModal(optionsModalEl);
        currentContext?.showFirstTimeSetup?.();
      });
    } else if (def.key === 'checkForUpdates') {
      btn.addEventListener('click', () => checkForUpdates(btn));
    } else if (def.key === 'installUpdateFromFile') {
      btn.addEventListener('click', () => installUpdateFromFile(btn));
    } else if (def.key === 'moveDataDirectory') {
      btn.addEventListener('click', () => handleMoveDataDirectory(btn));
    } else if (def.key === 'revealDataDirectory') {
      btn.addEventListener('click', () => handleRevealDataDirectory(btn));
    } else if (def.key === 'ccliReport') {
      btn.addEventListener('click', () => openCcliReportModal());
    }
    controlWrap.appendChild(btn);
  } else if (def.control === 'select') {
    const select = document.createElement('select');
    select.className = 'search-input';
    select.id = `setting-${def.key}`;
    let opts = def.options || [];
    if (def.key === 'defaultBibleVersion') {
      const bibles = currentContext ? currentContext.getInstalledBibles() : [];
      opts = (bibles || []).map(b => ({ value: b.id, label: `${b.abbreviation || b.id} - ${b.name || b.id}` }));
    }
    const currentValue = getSettingValue(def.key);
    opts.forEach(o => {
      const opt = document.createElement('option');
      opt.value = o.value;
      opt.textContent = o.label;
      const isMatch = def.key === 'defaultBibleVersion' && currentContext
        ? currentContext.areTranslationsEquivalent(o.value, currentValue)
        : o.value === currentValue;
      if (isMatch) {
        opt.selected = true;
      }
      select.appendChild(opt);
    });
    controlWrap.appendChild(select);
  } else {
    const input = document.createElement('input');
    input.type = def.control === 'password' ? 'password' : 'text';
    input.className = 'search-input';
    input.id = `setting-${def.key}`;
    input.placeholder = def.placeholder || '';
    input.value = getSettingValue(def.key);
    if (def.control === 'password') input.autocomplete = 'off';

    if (DIRECTORY_PICKER_KEYS.has(def.key)) {
      const group = document.createElement('div');
      group.style.display = 'flex';
      group.style.gap = '8px';
      group.style.alignItems = 'center';
      input.style.flex = '1';
      group.appendChild(input);

      const browseBtn = document.createElement('button');
      browseBtn.className = 'btn';
      browseBtn.type = 'button';
      browseBtn.textContent = 'Browse…';
      browseBtn.addEventListener('click', async () => {
        browseBtn.disabled = true;
        try {
          const result = await api.system.pickFolder();
          if (!result.available) {
            showToast('Folder picker is only available in the desktop app — type the path instead.', 'warning');
          } else if (result.path) {
            input.value = result.path;
          }
        } catch (_) {
          showToast('Could not open the folder picker.', 'error');
        } finally {
          browseBtn.disabled = false;
        }
      });
      group.appendChild(browseBtn);

      controlWrap.appendChild(group);
    } else {
      controlWrap.appendChild(input);
    }
  }

  row.appendChild(controlWrap);
  return row;
}

/** Settings > About > "Check for Updates" (docs/update.md) -- forces a real,
 * right-now check (the background task in main.rs already does this
 * periodically; this is for "did that just happen" / "check again right
 * now" on demand). Offers to download-and-install immediately when a newer
 * release is found, going straight through the same verified download path
 * a Linux `.deb` install uses -- no separate "download" button. */
export async function checkForUpdates(btn: HTMLButtonElement): Promise<void> {
  btn.disabled = true;
  try {
    const result = await api.updates.check();
    settingsUpdateStatus = result.status;
    renderSettingsContent();

    if (result.status.error) {
      showToast(`Could not check for updates: ${result.status.error}`, 'error');
      return;
    }
    const latest = result.status.latest;
    if (!latest) {
      const note = result.throttled ? ' (already checked moments ago)' : '';
      showToast(`You're up to date (v${result.status.current_version})${note}.`, 'success');
      return;
    }
    showConfirmDialog(
      'Update Available',
      `v${latest.version} is available (you're on v${result.status.current_version}). Download and install now?`,
      'This downloads the verified installer and hands it to your system\'s own installer -- OpenSanctuary never installs anything with elevated privileges itself.',
      async () => { await downloadAndInstallUpdate(); },
      'Download & Install'
    );
  } catch (_) {
    showToast('Could not check for updates.', 'error');
  } finally {
    btn.disabled = false;
  }
}

export async function downloadAndInstallUpdate(): Promise<void> {
  try {
    const result = await api.updates.downloadAndInstall();
    if (!result.ok) {
      showToast(`Could not install update: ${result.error || 'unknown error'}`, 'error');
      return;
    }
    showToast(`Opened your system's installer for v${result.version}. Restart OpenSanctuary once it finishes.`, 'success');
  } catch (_) {
    showToast('Could not download the update.', 'error');
  }
}

/** Settings > About > "Install Update from File" (docs/update.md) -- the manual
 * fallback path: pick a release you already downloaded yourself. Useful on
 * platforms the automated download-and-install path doesn't cover yet
 * (Windows/macOS), or for installing without waiting for a check. */
async function installUpdateFromFile(btn: HTMLButtonElement): Promise<void> {
  btn.disabled = true;
  try {
    const picked = await api.system.pickFile();
    if (!picked.available) {
      showToast('Picking a file is only available in the desktop app.', 'warning');
      return;
    }
    if (!picked.path) return; // canceled
    await applyLocalUpdate(picked.path, false);
  } catch (_) {
    showToast('Could not open the file picker.', 'error');
  } finally {
    btn.disabled = false;
  }
}

async function applyLocalUpdate(path: string, force: boolean): Promise<void> {
  try {
    const result = await api.updates.applyLocal({ path, force });
    const status = result.checksum.status;

    if (result.needs_confirmation) {
      const detail = status === 'mismatch'
        ? `The file's checksum doesn't match what checksums.txt expects (expected ${result.checksum.expected}, got ${result.checksum.actual}). This could mean a corrupted or tampered download.`
        : `checksums.txt.minisig didn't verify: ${result.checksum.reason}. The checksum file itself isn't trustworthy.`;
      showConfirmDialog(
        'Checksum Verification Failed',
        'This file did not pass verification. Install it anyway?',
        detail,
        async () => { await applyLocalUpdate(path, true); },
        'Install Anyway'
      );
      return;
    }

    if (!result.ok) {
      showToast(`Could not open installer: ${result.error || 'unknown error'}`, 'error');
      return;
    }

    const note = status === 'verified_and_signed' ? ' (signature verified ✓)'
      : status === 'checksum_matched_unsigned' ? ' (checksum matched)'
      : status === 'no_entry_for_file' ? ' (no matching entry in checksums.txt)'
      : '';
    showToast(`Opened your system's installer for the update${note}. Restart OpenSanctuary once it finishes.`, 'success');
  } catch (_) {
    showToast('Could not apply the update.', 'error');
  }
}

/**
 * Settings > Storage > "Move Data Directory" (docs/paths.md) -- relocates data directory
 * to a new destination folder with SHA-256 verification and optional source cleanup.
 */
async function handleRevealDataDirectory(btn: HTMLButtonElement): Promise<void> {
  btn.disabled = true;
  try {
    const res = await api.system.revealDataDir();
    if (!res.ok) {
      showToast(res.error || 'Could not open the file manager.', 'error');
    }
  } catch (_) {
    showToast('Could not open the file manager.', 'error');
  } finally {
    btn.disabled = false;
  }
}

async function handleMoveDataDirectory(btn: HTMLButtonElement): Promise<void> {
  btn.disabled = true;
  try {
    let targetPath: string | null = null;
    const picked = await api.system.pickFolder();
    if (picked.available && picked.path) {
      targetPath = picked.path;
    } else if (!picked.available) {
      targetPath = window.prompt('Enter destination directory path to move data into:');
    }
    if (!targetPath || !targetPath.trim()) {
      return;
    }
    targetPath = targetPath.trim();

    // Ask if original directory should be deleted after SHA verify
    const deleteSource = window.confirm(
      `Delete original data directory after copy and SHA-256 verification?\n\n` +
      `• Click 'OK' to delete the original directory once verified.\n` +
      `• Click 'Cancel' to keep the original directory intact as a backup.`
    );

    const detailText = deleteSource
      ? `All files will be copied to "${targetPath}" and verified via SHA-256 checksums. Upon successful verification, the original directory files will be deleted. An application restart is required to load from the new location.`
      : `All files will be copied to "${targetPath}" and verified via SHA-256 checksums. The original directory will be kept as a backup. An application restart is required to load from the new location.`;

    const acceptText = deleteSource ? 'Relocate & Delete Original' : 'Relocate & Keep Backup';

    showConfirmDialog(
      'Move Data Directory',
      `Move data directory to "${targetPath}"?`,
      detailText,
      async () => {
        btn.disabled = true;
        const originalText = btn.textContent;
        btn.textContent = 'Relocating…';
        showToast('Copying files and verifying SHA-256 checksums…', 'info');
        try {
          const res = await api.system.moveDataDir(targetPath!, deleteSource);
          if (res.ok) {
            const cleanupMsg = res.source_deleted ? 'Original directory cleaned up.' : 'Original directory kept as backup.';
            showToast(`Data directory moved (${res.files_copied} files verified). ${cleanupMsg} Please restart OpenSanctuary.`, 'success');
            await loadSettingsServerInfo();
            renderSettingsContent();
          } else {
            showToast('Failed to move data directory.', 'error');
          }
        } catch (err: any) {
          showToast(`Failed to move data directory: ${err?.message || err}`, 'error');
        } finally {
          btn.disabled = false;
          btn.textContent = originalText;
        }
      },
      acceptText
    );
  } catch (err: any) {
    showToast(`Could not initiate folder selection: ${err?.message || err}`, 'error');
  } finally {
    btn.disabled = false;
  }
}

export function renderSettingsContent() {
  const content = document.getElementById('settings-content');
  if (!content) return;
  const q = settingsSearchQuery.trim().toLowerCase();

  if (!q && activeSettingsCategory === 'display') {
    renderDisplaySettings(content);
    return;
  }
  if (!q && activeSettingsCategory === 'theme') {
    renderThemeSettings(content);
    return;
  }
  if (!q && activeSettingsCategory === 'network') {
    renderNetworkSettings(content);
    return;
  }
  if (!q && activeSettingsCategory === 'paired-devices') {
    renderPairedDevicesSettings(content);
    return;
  }

  content.innerHTML = '';

  const defsToShow = q
    ? SETTINGS_SCHEMA.filter(d => settingMatchesSearch(d, q))
    : SETTINGS_SCHEMA.filter(d => d.category === activeSettingsCategory);

  if (defsToShow.length === 0) {
    const empty = document.createElement('div');
    empty.className = 'settings-empty';
    empty.textContent = `No settings match "${settingsSearchQuery}".`;
    content.appendChild(empty);
    return;
  }

  let lastCategory: string | null = null;
  defsToShow.forEach(def => {
    if (q && def.category !== lastCategory) {
      lastCategory = def.category;
      const catDef = SETTINGS_CATEGORIES.find(c => c.id === def.category);
      const header = document.createElement('div');
      header.className = 'settings-group-header';
      header.textContent = catDef ? `${catDef.icon} ${catDef.label}` : def.category;
      content.appendChild(header);
    }
    content.appendChild(renderSettingRow(def, !!q));
  });
}

export function onSettingsSearchInput(value: string) {
  settingsSearchQuery = value;
  renderSettingsSidebar();
  renderSettingsContent();
}

export async function loadSettingsServerInfo() {
  try {
    settingsServerInfo = await api.system.serverInfo();
  } catch (_) { /* About panel just shows "(unavailable)" */ }
  try {
    const result = await api.updates.status();
    settingsUpdateStatus = result.status;
  } catch (_) {
    // Not fatal -- the appVersion row just omits the "Update available"
    // suffix (e.g. not console-authenticated yet, or offline).
  }
  return settingsServerInfo;
}

export function resetOptionsModal() {
  activeSettingsCategory = 'general';
  settingsSearchQuery = '';
  const searchEl = document.getElementById('settings-search') as HTMLInputElement | null;
  if (searchEl) searchEl.value = '';
  displaysLoaded = false;
  networkLoaded = false;
  pairedDevicesLoaded = false;
  renderSettingsSidebar();
  renderSettingsContent();
  loadSettingsServerInfo().then(() => {
    if (activeSettingsCategory === 'about') renderSettingsContent();
  });
}

// ============================================================================
// DISPLAY PANEL — bespoke (not the generic single-value row renderer above):
// live monitor detection + per-output-slot Open/Close against the native
// desktop app's tao/wry windows. See src/webview/mod.rs (DisplayManagerHandle)
// and the /api/displays* routes in src/api/routes.rs.
// ============================================================================

async function fetchDisplaysState() {
  try {
    const data = await api.displays.list();
    displaysAvailable = !!data?.available;
    displaysMonitors = data?.monitors || [];
    displaysStatuses = data?.statuses || {};
  } catch (_) {
    displaysAvailable = false;
  }
  displaysLoaded = true;
}

function monitorLabel(m: MonitorInfo): string {
  return `${m.name}${m.is_primary ? ' (Primary)' : ''} — ${m.width}×${m.height}`;
}

function getOutputs(): DisplayOutputConfig[] {
  return currentContext ? currentContext.getDisplayOutputs() : [];
}

async function persistOutputs(outputs: DisplayOutputConfig[]) {
  if (currentContext) await currentContext.saveDisplayOutputs(outputs);
}

type ThemeCardKind = 'both' | 'icons' | 'colors';

/**
 * One pickable card. 'icons' previews the set's icons, 'colors' mocks the interface
 * accents, 'both' (the cohesive theme) shows the two together.
 */
function buildThemeCard(set: IconSetDef, kind: ThemeCardKind, selectedId: string, onPick: () => void): HTMLElement {
  const card = document.createElement('button');
  card.type = 'button';
  card.className = 'icon-set-card' + (set.id === selectedId ? ' selected' : '');
  card.id = `theme-card-${kind}-${set.id}`;
  card.setAttribute('role', 'radio');
  card.setAttribute('aria-checked', String(set.id === selectedId));

  if (kind !== 'colors') {
    const preview = document.createElement('div');
    preview.className = 'icon-set-preview';
    ICON_SET_PREVIEW_ICONS.forEach(name => {
      const img = document.createElement('img');
      img.src = iconUrl(set.id, name);
      img.alt = '';
      img.draggable = false;
      preview.appendChild(img);
    });
    card.appendChild(preview);
  }

  const c = set.colors;
  if (kind !== 'icons') {
    const mock = document.createElement('div');
    mock.className = 'theme-color-mock';
    mock.innerHTML = `<span class="theme-color-mock-btn" style="background:${c.primary}">Aa</span>`
      + `<span class="theme-color-mock-bar" style="background:${c.highlight}"></span>`
      + `<span class="theme-color-mock-bar" style="background:${c.deep}"></span>`
      + `<span class="theme-color-mock-bar" style="background:${c.bright}"></span>`;
    card.appendChild(mock);
  }

  const dots = (kind === 'icons' ? set.tones : [c.primary, c.highlight, c.deep])
    .map(col => `<span class="icon-set-dot" style="background:${col}"></span>`).join('');
  const meta = document.createElement('div');
  meta.className = 'icon-set-meta';
  meta.innerHTML = `<span class="icon-set-name">${escapeHtml(set.label)}</span><span class="icon-set-swatch">${dots}</span>`;
  card.appendChild(meta);

  card.addEventListener('click', onPick);
  return card;
}

/**
 * Settings > Theme: icon set and interface color theme. By default they are one
 * cohesive theme (one pick sets both); turning "Match Icons and Colors" off shows
 * two independent pickers. Choices apply immediately (the app behind the dialog
 * changes) and are saved with the other app options.
 */
function renderThemeSettings(content: HTMLElement) {
  content.innerHTML = '';
  content.classList.remove('display-panel');
  const theme = resolveTheme(currentContext?.getAppOptions() ?? {});
  const rerender = () => renderThemeSettings(content);
  const change = async (changes: Record<string, string>) => {
    await currentContext?.setThemeOptions(changes);
    rerender();
  };

  const linkRow = document.createElement('div');
  linkRow.className = 'settings-row';
  linkRow.innerHTML = '<div class="settings-row-label"><label class="form-label" for="theme-linked-toggle">Match Icons and Colors</label>'
    + '<div class="settings-row-desc">One cohesive theme for the icons and the interface colors. Turn off to choose them independently.</div></div>';
  const linkControl = document.createElement('div');
  linkControl.className = 'settings-row-control';
  const toggle = document.createElement('input');
  toggle.type = 'checkbox';
  toggle.id = 'theme-linked-toggle';
  toggle.checked = theme.linked;
  toggle.addEventListener('change', () => {
    // Linking adopts the icon set's colors; unlinking keeps what's showing now.
    void change(toggle.checked
      ? { themeLinked: 'true', colorTheme: theme.iconSet }
      : { themeLinked: 'false', colorTheme: theme.colorTheme });
  });
  linkControl.appendChild(toggle);
  linkRow.appendChild(linkControl);
  content.appendChild(linkRow);

  const section = (title: string, desc: string, kind: ThemeCardKind, selectedId: string, pick: (id: string) => Record<string, string>) => {
    const header = document.createElement('div');
    header.className = 'settings-group-header';
    header.textContent = title;
    content.appendChild(header);
    const d = document.createElement('div');
    d.className = 'settings-row-desc';
    d.textContent = desc;
    content.appendChild(d);
    const grid = document.createElement('div');
    grid.className = 'icon-set-grid';
    grid.setAttribute('role', 'radiogroup');
    grid.setAttribute('aria-label', title);
    ICON_SETS.forEach(set => grid.appendChild(buildThemeCard(set, kind, selectedId, () => void change(pick(set.id)))));
    content.appendChild(grid);
  };

  if (theme.linked) {
    section('Theme', 'Icons and interface colors together. Takes effect immediately.', 'both', theme.iconSet,
      id => ({ iconSet: id, colorTheme: id }));
  } else {
    section('Icon Set', 'Color of the toolbar and menu icons.', 'icons', theme.iconSet, id => ({ iconSet: id }));
    section('Color Theme', 'Accent colors across the interface: selection, buttons and highlights.', 'colors', theme.colorTheme,
      id => ({ colorTheme: id }));
  }
}

function renderDisplaySettings(content: HTMLElement) {
  content.innerHTML = '';
  content.classList.add('display-panel');

  if (!displaysLoaded) {
    const loading = document.createElement('div');
    loading.className = 'settings-empty';
    loading.textContent = 'Checking display availability…';
    content.appendChild(loading);
    fetchDisplaysState().then(() => {
      if (activeSettingsCategory === 'display') renderDisplaySettings(content);
    });
    return;
  }

  if (!displaysAvailable) {
    const banner = document.createElement('div');
    banner.className = 'display-unavailable-banner';
    banner.innerHTML = `⚠ Native display windows require the OS-Next desktop app (not the headless/browser-only server). Launch the desktop app to detect monitors and open display outputs here.`;
    content.appendChild(banner);
    return;
  }

  const toolbar = document.createElement('div');
  toolbar.className = 'display-toolbar';
  const refreshBtn = document.createElement('button');
  refreshBtn.className = 'btn';
  refreshBtn.textContent = '↻ Refresh Displays';
  refreshBtn.addEventListener('click', async () => {
    try { await api.displays.refresh(); } catch (_) { /* ignore */ }
    await new Promise(r => setTimeout(r, 250));
    await fetchDisplaysState();
    renderDisplaySettings(content);
  });
  toolbar.appendChild(refreshBtn);
  content.appendChild(toolbar);

  const monitorsHeader = document.createElement('div');
  monitorsHeader.className = 'settings-group-header';
  monitorsHeader.textContent = `🖵 Detected Monitors (${displaysMonitors.length})`;
  content.appendChild(monitorsHeader);

  const monitorsGrid = document.createElement('div');
  monitorsGrid.className = 'display-monitors-grid';
  displaysMonitors.forEach(m => {
    const card = document.createElement('div');
    card.className = 'display-monitor-card';
    card.innerHTML = `<div class="display-monitor-name">${escapeHtml(m.name)}${m.is_primary ? ' <span class="display-monitor-primary-tag">Primary</span>' : ''}</div>`
      + `<div class="display-monitor-res">${m.width} × ${m.height}</div>`;
    monitorsGrid.appendChild(card);
  });
  if (displaysMonitors.length === 0) {
    monitorsGrid.innerHTML = '<div class="settings-empty">No monitors detected.</div>';
  }
  content.appendChild(monitorsGrid);

  const outputsHeader = document.createElement('div');
  outputsHeader.className = 'settings-group-header';
  outputsHeader.textContent = 'Configured Display Outputs';
  content.appendChild(outputsHeader);

  const outputs = getOutputs();
  const outputsList = document.createElement('div');
  outputsList.className = 'display-outputs-list';
  outputs.forEach(out => {
    outputsList.appendChild(renderDisplayOutputRow(out, outputs, content));
  });
  if (outputs.length === 0) {
    const empty = document.createElement('div');
    empty.className = 'settings-empty';
    empty.textContent = 'No display outputs configured yet.';
    outputsList.appendChild(empty);
  }
  content.appendChild(outputsList);

  const addBtn = document.createElement('button');
  addBtn.className = 'btn display-add-btn';
  addBtn.textContent = '+ Add Display';
  addBtn.addEventListener('click', async () => {
    const updated = [...getOutputs(), createDefaultDisplayOutput(getOutputs())];
    await persistOutputs(updated);
    renderDisplaySettings(content);
  });
  content.appendChild(addBtn);
}

function renderDisplayOutputRow(out: DisplayOutputConfig, allOutputs: DisplayOutputConfig[], content: HTMLElement): HTMLElement {
  const row = document.createElement('div');
  row.className = 'display-output-row';

  const status = displaysStatuses[out.id];
  const isOpen = !!(status && status.open);

  const updateField = async (patch: Partial<DisplayOutputConfig>) => {
    const updated = allOutputs.map(o => o.id === out.id ? { ...o, ...patch } : o);
    await persistOutputs(updated);
  };

  const labelInput = document.createElement('input');
  labelInput.type = 'text';
  labelInput.className = 'search-input display-output-label-input';
  labelInput.value = out.label;
  labelInput.addEventListener('change', () => updateField({ label: labelInput.value.trim() || out.label }));

  const contentSelect = document.createElement('select');
  contentSelect.className = 'search-input';
  DISPLAY_CONTENT_SOURCES.forEach(src => {
    const opt = document.createElement('option');
    opt.value = src.value;
    opt.textContent = src.label;
    if (src.value === out.contentPath) opt.selected = true;
    contentSelect.appendChild(opt);
  });
  contentSelect.addEventListener('change', () => updateField({ contentPath: contentSelect.value }));

  const monitorSelect = document.createElement('select');
  monitorSelect.className = 'search-input';
  displaysMonitors.forEach(m => {
    const opt = document.createElement('option');
    opt.value = String(m.index);
    opt.textContent = monitorLabel(m);
    if (m.index === out.monitorIndex) opt.selected = true;
    monitorSelect.appendChild(opt);
  });
  monitorSelect.addEventListener('change', () => updateField({ monitorIndex: parseInt(monitorSelect.value, 10) }));

  const resolutionSelect = document.createElement('select');
  resolutionSelect.className = 'search-input';
  [['native', 'Fill Monitor (Native)'], ['fixed', 'Fixed Resolution']].forEach(([value, label]) => {
    const opt = document.createElement('option');
    opt.value = value;
    opt.textContent = label;
    if (value === out.resolutionMode) opt.selected = true;
    resolutionSelect.appendChild(opt);
  });
  resolutionSelect.addEventListener('change', () => updateField({ resolutionMode: resolutionSelect.value as 'native' | 'fixed' }));

  const widthInput = document.createElement('input');
  widthInput.type = 'number';
  widthInput.className = 'search-input display-output-dim-input';
  widthInput.placeholder = 'Width';
  widthInput.value = out.fixedWidth ? String(out.fixedWidth) : '1920';
  widthInput.addEventListener('change', () => updateField({ fixedWidth: parseInt(widthInput.value, 10) || 1920 }));

  const heightInput = document.createElement('input');
  heightInput.type = 'number';
  heightInput.className = 'search-input display-output-dim-input';
  heightInput.placeholder = 'Height';
  heightInput.value = out.fixedHeight ? String(out.fixedHeight) : '1080';
  heightInput.addEventListener('change', () => updateField({ fixedHeight: parseInt(heightInput.value, 10) || 1080 }));

  const dimsWrap = document.createElement('div');
  dimsWrap.className = 'display-output-dims';
  dimsWrap.style.display = out.resolutionMode === 'fixed' ? 'flex' : 'none';
  dimsWrap.appendChild(widthInput);
  dimsWrap.appendChild(heightInput);
  resolutionSelect.addEventListener('change', () => {
    dimsWrap.style.display = resolutionSelect.value === 'fixed' ? 'flex' : 'none';
  });

  const aotLabel = document.createElement('label');
  aotLabel.className = 'display-output-checkbox';
  const aotCheckbox = document.createElement('input');
  aotCheckbox.type = 'checkbox';
  aotCheckbox.checked = out.alwaysOnTop;
  aotCheckbox.addEventListener('change', () => updateField({ alwaysOnTop: aotCheckbox.checked }));
  aotLabel.appendChild(aotCheckbox);
  aotLabel.appendChild(document.createTextNode('Always on Top'));

  const statusBadge = document.createElement('span');
  statusBadge.className = 'display-output-status ' + (isOpen ? 'open' : 'closed');
  statusBadge.textContent = isOpen && status ? `● Open (${status.width}×${status.height})` : '○ Closed';

  const openCloseBtn = document.createElement('button');
  openCloseBtn.className = 'btn' + (isOpen ? '' : ' btn-primary');
  openCloseBtn.textContent = isOpen ? 'Close' : 'Open';
  openCloseBtn.addEventListener('click', async () => {
    try {
      if (isOpen) {
        await api.displays.close({ id: out.id });
      } else {
        const resolution = out.resolutionMode === 'fixed'
          ? { kind: 'fixed', width: out.fixedWidth || 1920, height: out.fixedHeight || 1080 }
          : { kind: 'native' };
        await api.displays.open({
          id: out.id,
          content_path: out.contentPath,
          monitor_index: out.monitorIndex,
          resolution,
          always_on_top: out.alwaysOnTop,
        });
      }
    } catch (_) { /* status will just show unchanged on failure */ }
    await new Promise(r => setTimeout(r, 250));
    await fetchDisplaysState();
    renderDisplaySettings(content);
  });

  const removeBtn = document.createElement('button');
  removeBtn.className = 'btn display-remove-btn';
  removeBtn.textContent = '✕';
  removeBtn.title = 'Remove this display output';
  removeBtn.addEventListener('click', async () => {
    if (isOpen) {
      try {
        await api.displays.close({ id: out.id });
      } catch (_) { /* ignore */ }
    }
    const updated = allOutputs.filter(o => o.id !== out.id);
    await persistOutputs(updated);
    renderDisplaySettings(content);
  });

  const fieldsRow = document.createElement('div');
  fieldsRow.className = 'display-output-fields';
  fieldsRow.appendChild(labelInput);
  fieldsRow.appendChild(contentSelect);
  fieldsRow.appendChild(monitorSelect);
  fieldsRow.appendChild(resolutionSelect);
  fieldsRow.appendChild(dimsWrap);
  fieldsRow.appendChild(aotLabel);

  const actionsRow = document.createElement('div');
  actionsRow.className = 'display-output-actions';
  actionsRow.appendChild(statusBadge);
  actionsRow.appendChild(openCloseBtn);
  actionsRow.appendChild(removeBtn);

  row.appendChild(fieldsRow);
  row.appendChild(actionsRow);
  return row;
}

// ============================================================================
// NETWORK PANEL — bespoke controller for Network configuration:
// - Host name setting & DHCP Option 12 broadcast trigger
// - Live RFC 1035 sanitization & LAN conflict detection
// - HTTP / Remote port setting & port conflict checker
// - Dedicated virtual adapter (macvlan/ipvlan/VMAdapter) toggle with Polkit/UAC feedback
// - Multi-interface enumeration and broadcast selection
// ============================================================================

export interface NetworkInterfaceItem {
  name: string;
  display_name: string;
  interface_type: string;
  ipv4: string | null;
  netmask: string | null;
  broadcast: string | null;
  mac_address: string | null;
  is_loopback: boolean;
  is_virtual: boolean;
  is_up: boolean;
  enabled: boolean;
}

export interface NetworkState {
  interfaces: NetworkInterfaceItem[];
  broadcast_all: boolean;
  hostname: string;
  port: number;
  dedicated_active: boolean;
  dedicated_adapter: NetworkInterfaceItem | null;
  lan_ip?: string;
  remote_url?: string;
  mdns_url?: string;
  is_dedicated?: boolean;
  http_port?: number;
  https_port?: number | null;
  https_enabled?: boolean;
  pairing_url?: string;
  mdns_pairing_url?: string;
  public_https_url?: string | null;
}

let networkLoaded = false;
let networkState: NetworkState | null = null;
let networkSettingsDirty = false;
let networkElevationInProgress = false;
let networkElevationError: string | null = null;

async function fetchNetworkState() {
  try {
    const [ifaceData, infoData] = await Promise.all([
      api.network.interfaces(),
      api.network.info(),
    ]);
    if (ifaceData && infoData) {
      networkState = {
        interfaces: ifaceData.interfaces || [],
        broadcast_all: ifaceData.broadcast_all !== false,
        hostname: ifaceData.hostname || 'opensanctuary',
        port: ifaceData.port || 8080,
        dedicated_active: !!ifaceData.dedicated_active,
        dedicated_adapter: ifaceData.dedicated_adapter || null,
        lan_ip: infoData.lan_ip,
        remote_url: infoData.remote_url,
        mdns_url: infoData.mdns_url,
        is_dedicated: !!infoData.is_dedicated,
        http_port: infoData.http_port || ifaceData.http_port || infoData.port || 8080,
        https_port: infoData.https_port ?? ifaceData.https_port ?? null,
        https_enabled: !!(infoData.https_enabled ?? ifaceData.https_enabled),
        pairing_url: infoData.pairing_url,
        mdns_pairing_url: infoData.mdns_pairing_url,
        public_https_url: infoData.public_https_url ?? null,
      };
    }
  } catch (err) {
    console.warn('Failed to fetch network state:', err);
  }
  networkLoaded = true;
}

/**
 * POST /api/settings now holds back `networkHostname`/`networkPort` when they
 * collide with something already on the network, reporting it in a
 * `_conflicts` field rather than saving silently. This surfaces that to the
 * operator and, if they explicitly confirm, resubmits the exact same
 * payload with `_confirmOverrides` so the server saves it anyway. Returns
 * false only if a conflict was reported and the operator declined to
 * override it.
 */
async function resolveSettingsConflicts(
  result: Record<string, any>,
  originalPayload: Record<string, string>
): Promise<boolean> {
  const conflicts = result && result._conflicts;
  if (!conflicts || typeof conflicts !== 'object' || Object.keys(conflicts).length === 0) {
    return true;
  }

  const lines = Object.entries(conflicts).map(([key, info]: [string, any]) => `${key}: ${info?.message || 'conflict detected'}`);
  const proceed = confirm(
    `${lines.join('\n')}\n\nSave anyway? Anything already using this on the network may be affected.`
  );
  if (!proceed) {
    return false;
  }

  await api.settings.save({
    ...originalPayload,
    _confirmOverrides: Object.keys(conflicts).join(','),
  });
  return true;
}

export async function saveNetworkSettings(): Promise<boolean> {
  const hostInput = document.getElementById('network-hostname-input') as HTMLInputElement | null;
  const portInput = document.getElementById('network-port-input') as HTMLInputElement | null;
  const publicUrlInput = document.getElementById('network-public-https-url-input') as HTMLInputElement | null;
  const broadcastAllInput = document.getElementById('network-broadcast-all-toggle') as HTMLInputElement | null;
  const checkboxes = document.querySelectorAll<HTMLInputElement>('.network-iface-checkbox');
  const cspSelect = document.getElementById('setting-securityCspMode') as HTMLSelectElement | null;
  const corsSelect = document.getElementById('setting-securityCorsMode') as HTMLSelectElement | null;
  const frameSelect = document.getElementById('setting-securityFrameOptions') as HTMLSelectElement | null;

  // Guard: If network settings were never loaded, or no inputs are mounted and settings aren't dirty,
  // do not overwrite existing backend settings with defaults.
  if (!hostInput && !portInput && !publicUrlInput && !broadcastAllInput && !cspSelect && !corsSelect && !frameSelect && checkboxes.length === 0 && !networkSettingsDirty) {
    return true;
  }

  // If inputs are not currently mounted in DOM, but settings are dirty, use cached networkState
  if (!hostInput && !portInput && networkState && networkSettingsDirty) {
    try {
      const dirtyPayload: Record<string, string> = {
        networkHostname: networkState.hostname,
        networkPort: String(networkState.port),
        networkBroadcastAll: networkState.broadcast_all ? 'true' : 'false',
        publicHttpsUrl: networkState.public_https_url || '',
      };
      if (cspSelect) dirtyPayload.securityCspMode = cspSelect.value;
      if (corsSelect) dirtyPayload.securityCorsMode = corsSelect.value;
      if (frameSelect) dirtyPayload.securityFrameOptions = frameSelect.value;
      let result = await api.settings.save(dirtyPayload);
      if (!(await resolveSettingsConflicts(result, dirtyPayload))) {
        return false;
      }
      networkSettingsDirty = false;
      return true;
    } catch (_) {
      return false;
    }
  }

  const hostname = hostInput ? hostInput.value.trim() : (networkState?.hostname || 'opensanctuary');
  const port = portInput ? portInput.value.trim() : String(networkState?.port || 8080);
  const publicHttpsUrl = publicUrlInput ? publicUrlInput.value.trim() : (networkState?.public_https_url || '');
  const broadcastAll = broadcastAllInput ? broadcastAllInput.checked : (networkState?.broadcast_all ?? true);

  // Collect enabled interfaces
  const enabledInterfaces: string[] = [];
  checkboxes.forEach(cb => {
    if (cb.checked && cb.dataset.iface) {
      enabledInterfaces.push(cb.dataset.iface);
    }
  });

  try {
    // 1. Save core and security settings
    const savePayload: Record<string, string> = {
      networkHostname: hostname,
      networkPort: port,
      networkBroadcastAll: broadcastAll ? 'true' : 'false',
      publicHttpsUrl,
    };
    if (cspSelect) savePayload.securityCspMode = cspSelect.value;
    if (corsSelect) savePayload.securityCorsMode = corsSelect.value;
    if (frameSelect) savePayload.securityFrameOptions = frameSelect.value;

    const saveResult = await api.settings.save(savePayload);
    if (!(await resolveSettingsConflicts(saveResult, savePayload))) {
      return false;
    }

    if (currentContext) {
      const opts = currentContext.getAppOptions();
      if (cspSelect) opts.securityCspMode = cspSelect.value;
      if (corsSelect) opts.securityCorsMode = corsSelect.value;
      if (frameSelect) opts.securityFrameOptions = frameSelect.value;
    }

    // 2. Save interface preferences
    if (checkboxes.length > 0) {
      await api.network.saveInterfaces({
        enabled_interfaces: enabledInterfaces,
        broadcast_all: broadcastAll,
      });
    }

    if (networkState) {
      networkState.hostname = hostname;
      networkState.port = parseInt(port, 10) || 8080;
      networkState.broadcast_all = broadcastAll;
      networkState.public_https_url = publicHttpsUrl || null;
    }

    networkSettingsDirty = false;
    return true;
  } catch (err) {
    console.warn('Failed to save network settings:', err);
    return false;
  }
}

function renderNetworkSettings(content: HTMLElement) {
  content.innerHTML = '';
  content.classList.add('network-panel');

  if (!networkLoaded) {
    const loading = document.createElement('div');
    loading.className = 'settings-empty';
    loading.textContent = 'Discovering network interfaces and configuration…';
    content.appendChild(loading);
    fetchNetworkState().then(() => {
      if (activeSettingsCategory === 'network') renderNetworkSettings(content);
    });
    return;
  }

  if (!networkState) {
    const empty = document.createElement('div');
    empty.className = 'settings-empty';
    empty.textContent = 'Unable to load network interfaces.';
    content.appendChild(empty);
    return;
  }

  // 1. Overview Card
  const overviewCard = document.createElement('div');
  overviewCard.className = 'network-overview-card';

  const ipStat = document.createElement('div');
  ipStat.className = 'network-overview-stat';
  ipStat.innerHTML = `
    <div class="network-overview-stat-label">Active LAN IP</div>
    <div class="network-overview-stat-value">
      ${escapeHtml(networkState.lan_ip || '127.0.0.1')}
      ${networkState.is_dedicated ? ' <span class="network-badge network-badge--success">Dedicated IP</span>' : ''}
    </div>
  `;
  overviewCard.appendChild(ipStat);

  const portStat = document.createElement('div');
  portStat.className = 'network-overview-stat';
  portStat.innerHTML = `
    <div class="network-overview-stat-label">Active Port</div>
    <div class="network-overview-stat-value">${escapeHtml(networkState.port)}</div>
  `;
  overviewCard.appendChild(portStat);

  const hostnameStat = document.createElement('div');
  hostnameStat.className = 'network-overview-stat';
  hostnameStat.innerHTML = `
    <div class="network-overview-stat-label">Host Name</div>
    <div class="network-overview-stat-value">${escapeHtml(networkState.hostname)}.local</div>
  `;
  overviewCard.appendChild(hostnameStat);

  const adapterStat = document.createElement('div');
  adapterStat.className = 'network-overview-stat';
  adapterStat.innerHTML = `
    <div class="network-overview-stat-label">Dedicated MAC</div>
    <div class="network-overview-stat-value">
      ${networkState.dedicated_active
        ? `<span class="network-badge network-badge--success">Active (${escapeHtml(networkState.dedicated_adapter?.mac_address || 'Virtual')})</span>`
        : '<span class="network-badge network-badge--info">Disabled (Host System)</span>'}
    </div>
  `;
  overviewCard.appendChild(adapterStat);

  const httpsStat = document.createElement('div');
  httpsStat.className = 'network-overview-stat';
  httpsStat.innerHTML = `
    <div class="network-overview-stat-label">HTTPS (Secure Plane)</div>
    <div class="network-overview-stat-value">
      ${networkState.https_enabled && networkState.https_port
        ? `<span class="network-badge network-badge--success">Active (Port ${escapeHtml(networkState.https_port)})</span>`
        : '<span class="network-badge network-badge--info">Disabled</span>'}
    </div>
  `;
  overviewCard.appendChild(httpsStat);

  content.appendChild(overviewCard);

  // Elevation alert / prompt banner
  if (networkElevationInProgress) {
    const elevBanner = document.createElement('div');
    elevBanner.className = 'network-badge network-badge--warning';
    elevBanner.style.padding = '12px 14px';
    elevBanner.style.width = '100%';
    elevBanner.style.boxSizing = 'border-box';
    elevBanner.innerHTML = '🔐 <strong>Elevation Required:</strong> Please respond to the administrator prompt (Polkit on Linux / UAC on Windows) to configure the virtual network adapter...';
    content.appendChild(elevBanner);
  }

  if (networkElevationError) {
    const errBanner = document.createElement('div');
    errBanner.className = 'network-badge network-badge--error';
    errBanner.style.padding = '10px 12px';
    errBanner.style.width = '100%';
    errBanner.style.boxSizing = 'border-box';
    errBanner.innerHTML = `⚠️ <strong>Error:</strong> ${escapeHtml(networkElevationError)}`;
    content.appendChild(errBanner);
  }

  // 2. Hostname & DHCP Option 12 Card
  const hostnameCard = document.createElement('div');
  hostnameCard.className = 'network-card';

  const hostTitle = document.createElement('div');
  hostTitle.className = 'network-card-title';
  hostTitle.innerHTML = '<span>🏷️</span> Network Host Name & DHCP Option 12';
  hostnameCard.appendChild(hostTitle);

  const hostDesc = document.createElement('div');
  hostDesc.className = 'network-card-desc';
  hostDesc.textContent = 'The host name sent in DHCP Option 12 broadcast packets and advertised via mDNS (_opensanctuary._tcp.local) for auto-discovery.';
  hostnameCard.appendChild(hostDesc);

  const hostInputRow = document.createElement('div');
  hostInputRow.className = 'network-input-row';

  const hostInput = document.createElement('input');
  hostInput.type = 'text';
  hostInput.className = 'search-input';
  hostInput.id = 'network-hostname-input';
  hostInput.style.maxWidth = '260px';
  hostInput.value = networkState.hostname;

  const hostBadge = document.createElement('span');
  hostBadge.className = 'network-badge network-badge--info';
  hostBadge.id = 'network-hostname-badge';
  hostBadge.textContent = 'Checking LAN…';

  const bcastBtn = document.createElement('button');
  bcastBtn.className = 'btn';
  bcastBtn.id = 'btn-broadcast-option12';
  bcastBtn.innerHTML = '📡 Broadcast Option 12 Now';

  const bcastStatus = document.createElement('span');
  bcastStatus.id = 'network-broadcast-status';
  bcastStatus.className = 'network-badge network-badge--info';
  bcastStatus.style.display = 'none';

  bcastBtn.addEventListener('click', async () => {
    bcastBtn.disabled = true;
    bcastBtn.textContent = 'Broadcasting…';
    try {
      const data = await api.network.broadcastOption12(hostInput.value.trim());
      bcastStatus.style.display = 'inline-flex';
      if (data.success) {
        bcastStatus.className = 'network-badge network-badge--success';
        bcastStatus.textContent = `✓ Sent Option 12 to ${data.total_targets || 0} interface(s)`;
      } else {
        bcastStatus.className = 'network-badge network-badge--error';
        bcastStatus.textContent = 'Broadcast failed';
      }
    } catch (_) {
      bcastStatus.style.display = 'inline-flex';
      bcastStatus.className = 'network-badge network-badge--error';
      bcastStatus.textContent = 'Network error during broadcast';
    } finally {
      bcastBtn.disabled = false;
      bcastBtn.innerHTML = '📡 Broadcast Option 12 Now';
    }
  });

  const probeHostname = debounce(async (name: string) => {
    try {
      const data = await api.network.checkHostname(name);
      if (data.conflict_detected) {
        hostBadge.className = 'network-badge network-badge--warning';
        hostBadge.innerHTML = `⚠️ In use by ${escapeHtml(data.conflicting_ip || 'another device')}`;
        if (data.suggested_hostname) {
          const sugBtn = document.createElement('button');
          sugBtn.className = 'btn btn-xs';
          sugBtn.style.marginLeft = '6px';
          sugBtn.textContent = `Use "${data.suggested_hostname}"`;
          sugBtn.addEventListener('click', () => {
            hostInput.value = data.suggested_hostname;
            checkHostnameDebounced(data.suggested_hostname);
          });
          hostBadge.appendChild(sugBtn);
        }
      } else {
        hostBadge.className = 'network-badge network-badge--success';
        hostBadge.textContent = `✓ Available on LAN (${data.sanitized_hostname}.local)`;
      }
    } catch (_) {
      hostBadge.textContent = 'Could not verify LAN';
    }
  }, 350);

  const checkHostnameDebounced = (name: string) => {
    hostBadge.className = 'network-badge network-badge--info';
    hostBadge.textContent = 'Probing LAN…';
    probeHostname(name);
  };

  hostInput.addEventListener('input', () => {
    networkSettingsDirty = true;
    checkHostnameDebounced(hostInput.value);
  });
  checkHostnameDebounced(hostInput.value);

  hostInputRow.appendChild(hostInput);
  hostInputRow.appendChild(hostBadge);
  hostInputRow.appendChild(bcastBtn);
  hostInputRow.appendChild(bcastStatus);
  hostnameCard.appendChild(hostInputRow);
  content.appendChild(hostnameCard);

  // 2b. Public HTTPS URL card (docs/TUNNELS.md -- Caddy reverse proxy with a
  // real Let's Encrypt cert). Optional: an empty value just means "not set,
  // keep using the self-signed cert everywhere," so the badge stays quiet
  // until there's actually something to check.
  const publicUrlCard = document.createElement('div');
  publicUrlCard.className = 'network-card';

  const publicUrlTitle = document.createElement('div');
  publicUrlTitle.className = 'network-card-title';
  publicUrlTitle.innerHTML = '<span>🌐</span> Public HTTPS URL';
  publicUrlCard.appendChild(publicUrlTitle);

  const publicUrlDesc = document.createElement('div');
  publicUrlDesc.className = 'network-card-desc';
  publicUrlDesc.textContent = 'A trusted public domain (e.g. a Caddy reverse proxy with a real certificate) to prefer over the self-signed certificate in the Remote Console banner/QR, TV/mobile pairing, and /api/network/info. Leave blank to keep using the self-signed cert.';
  publicUrlCard.appendChild(publicUrlDesc);

  const publicUrlInputRow = document.createElement('div');
  publicUrlInputRow.className = 'network-input-row';

  const publicUrlInputEl = document.createElement('input');
  publicUrlInputEl.type = 'text';
  publicUrlInputEl.className = 'search-input';
  publicUrlInputEl.id = 'network-public-https-url-input';
  publicUrlInputEl.style.maxWidth = '320px';
  publicUrlInputEl.placeholder = 'https://connect.yourchurch.org';
  publicUrlInputEl.value = networkState.public_https_url || '';

  const publicUrlBadge = document.createElement('span');
  publicUrlBadge.className = 'network-badge network-badge--info';
  publicUrlBadge.id = 'network-public-https-url-badge';
  publicUrlBadge.style.display = 'none';

  const probePublicUrl = debounce(async (url: string) => {
    const trimmed = url.trim();
    if (!trimmed) {
      publicUrlBadge.style.display = 'none';
      return;
    }
    try {
      const data = await api.network.checkPublicUrl(trimmed);
      publicUrlBadge.style.display = 'inline-flex';
      if (data.reachable) {
        publicUrlBadge.className = 'network-badge network-badge--success';
        publicUrlBadge.textContent = `✓ ${data.message || 'Reachable'}`;
      } else {
        publicUrlBadge.className = 'network-badge network-badge--warning';
        publicUrlBadge.textContent = `⚠️ ${data.message || 'Not reachable yet'}`;
      }
    } catch (_) {
      publicUrlBadge.style.display = 'inline-flex';
      publicUrlBadge.className = 'network-badge network-badge--info';
      publicUrlBadge.textContent = 'Could not verify';
    }
  }, 500);

  publicUrlInputEl.addEventListener('input', () => {
    networkSettingsDirty = true;
    publicUrlBadge.className = 'network-badge network-badge--info';
    publicUrlBadge.style.display = publicUrlInputEl.value.trim() ? 'inline-flex' : 'none';
    publicUrlBadge.textContent = 'Checking…';
    probePublicUrl(publicUrlInputEl.value);
  });
  if (publicUrlInputEl.value.trim()) {
    probePublicUrl(publicUrlInputEl.value);
  }

  publicUrlInputRow.appendChild(publicUrlInputEl);
  publicUrlInputRow.appendChild(publicUrlBadge);
  publicUrlCard.appendChild(publicUrlInputRow);
  content.appendChild(publicUrlCard);

  // 3. HTTP & Remote Port Card
  const portCard = document.createElement('div');
  portCard.className = 'network-card';

  const portTitle = document.createElement('div');
  portTitle.className = 'network-card-title';
  portTitle.innerHTML = '<span>🔌</span> HTTP Server & Remote Control Port';
  portCard.appendChild(portTitle);

  const portDesc = document.createElement('div');
  portDesc.className = 'network-card-desc';
  portDesc.textContent = 'TCP port number for operator console, live stream, and mobile remote control. (Takes effect on server restart)';
  portCard.appendChild(portDesc);

  const portInputRow = document.createElement('div');
  portInputRow.className = 'network-input-row';

  const portInput = document.createElement('input');
  portInput.type = 'number';
  portInput.className = 'search-input';
  portInput.id = 'network-port-input';
  portInput.style.maxWidth = '140px';
  portInput.min = '1024';
  portInput.max = '65535';
  portInput.value = String(networkState.port);

  const portBadge = document.createElement('span');
  portBadge.className = 'network-badge network-badge--info';
  portBadge.id = 'network-port-badge';
  portBadge.textContent = 'Checking port…';

  const probePort = debounce(async (p: number) => {
    try {
      const data = await api.network.checkPort(p);
      if (data.in_use_by_current) {
        portBadge.className = 'network-badge network-badge--success';
        portBadge.textContent = '✓ Current active server port';
      } else if (data.available) {
        portBadge.className = 'network-badge network-badge--success';
        portBadge.textContent = `✓ Port ${p} is available`;
      } else {
        portBadge.className = 'network-badge network-badge--error';
        portBadge.innerHTML = `❌ Port ${p} is in use`;
        if (data.suggested_alternative) {
          const sugBtn = document.createElement('button');
          sugBtn.className = 'btn btn-xs';
          sugBtn.style.marginLeft = '6px';
          sugBtn.textContent = `Use ${data.suggested_alternative}`;
          sugBtn.addEventListener('click', () => {
            portInput.value = String(data.suggested_alternative);
            checkPortDebounced(String(data.suggested_alternative));
          });
          portBadge.appendChild(sugBtn);
        }
      }
    } catch (_) {
      portBadge.textContent = 'Could not verify port';
    }
  }, 350);

  const checkPortDebounced = (pVal: string) => {
    const p = parseInt(pVal, 10);
    if (!p || p < 1 || p > 65535) {
      probePort.cancel();
      portBadge.className = 'network-badge network-badge--error';
      portBadge.textContent = 'Invalid port number (1-65535)';
      return;
    }
    portBadge.className = 'network-badge network-badge--info';
    portBadge.textContent = 'Probing port…';
    probePort(p);
  };

  portInput.addEventListener('input', () => {
    networkSettingsDirty = true;
    checkPortDebounced(portInput.value);
  });
  checkPortDebounced(portInput.value);

  portInputRow.appendChild(portInput);
  portInputRow.appendChild(portBadge);
  portCard.appendChild(portInputRow);
  content.appendChild(portCard);

  // 3b. HTTPS & TLS Configuration Card
  const tlsCard = document.createElement('div');
  tlsCard.className = 'network-card';

  const tlsTitle = document.createElement('div');
  tlsTitle.className = 'network-card-title';
  tlsTitle.innerHTML = '<span>🔒</span> HTTPS & TLS Security Plane';
  tlsCard.appendChild(tlsTitle);

  const tlsDesc = document.createElement('div');
  tlsDesc.className = 'network-card-desc';
  tlsDesc.textContent = 'Dedicated encrypted HTTPS listener for console administration and camera QR pairing (required by iOS Safari and Android Chrome for camera scanning). AV output displays (/live.html, /stage.html) and Timecode sync remain on cleartext HTTP (zero-auth).';
  tlsCard.appendChild(tlsDesc);

  const tlsRow = document.createElement('div');
  tlsRow.className = 'network-input-row';
  tlsRow.style.alignItems = 'center';
  tlsRow.style.gap = '12px';

  const regenBtn = document.createElement('button');
  regenBtn.type = 'button';
  regenBtn.className = 'btn';
  regenBtn.id = 'btn-regenerate-tls';
  regenBtn.innerHTML = '🔄 Regenerate TLS Certificate';

  const tlsStatus = document.createElement('span');
  tlsStatus.id = 'network-tls-status';
  tlsStatus.className = 'network-badge network-badge--info';
  tlsStatus.textContent = networkState.https_enabled
    ? `✓ TLS active with SANs for ${networkState.hostname}.local & LAN IPs`
    : 'TLS disabled';

  regenBtn.addEventListener('click', async () => {
    regenBtn.disabled = true;
    tlsStatus.className = 'network-badge network-badge--info';
    tlsStatus.textContent = 'Generating new certificate & SANs…';
    try {
      const res = await api.network.regenerateTls();
      if (res.success) {
        tlsStatus.className = 'network-badge network-badge--success';
        tlsStatus.textContent = '✓ Certificate regenerated! Restart server to load new cert.';
      } else {
        tlsStatus.className = 'network-badge network-badge--error';
        tlsStatus.textContent = '❌ Failed to regenerate certificate';
      }
    } catch (err: any) {
      tlsStatus.className = 'network-badge network-badge--error';
      tlsStatus.textContent = `❌ ${err.message || 'Error regenerating certificate'}`;
    } finally {
      regenBtn.disabled = false;
    }
  });

  tlsRow.appendChild(regenBtn);
  tlsRow.appendChild(tlsStatus);
  tlsCard.appendChild(tlsRow);
  content.appendChild(tlsCard);

  // 4. Dedicated MAC & IP (Virtual Adapter) Card
  const macCard = document.createElement('div');
  macCard.className = 'network-card';

  const macTitle = document.createElement('div');
  macTitle.className = 'network-card-title';
  macTitle.innerHTML = '<span>🌐</span> Dedicated MAC & IP Address (Virtual Adapter)';
  macCard.appendChild(macTitle);

  const macDesc = document.createElement('div');
  macDesc.className = 'network-card-desc';
  macDesc.textContent = 'Allocates an isolated virtual adapter (macvlan bridge on Ethernet, ipvlan L2 on Wi-Fi, or VMNetworkAdapter on Windows) to acquire an independent MAC address and DHCP IP lease. Requests Polkit/UAC elevation only when toggled.';
  macCard.appendChild(macDesc);

  const macActionRow = document.createElement('div');
  macActionRow.className = 'network-input-row';

  const toggleMacBtn = document.createElement('button');
  toggleMacBtn.className = 'btn ' + (networkState.dedicated_active ? 'btn-danger' : 'btn-primary');
  toggleMacBtn.id = 'btn-toggle-dedicated-mac';
  toggleMacBtn.textContent = networkState.dedicated_active ? '✕ Remove Dedicated Adapter' : '✨ Acquire Dedicated MAC & IP';
  toggleMacBtn.disabled = networkElevationInProgress;

  toggleMacBtn.addEventListener('click', async () => {
    networkElevationInProgress = true;
    networkElevationError = null;
    renderNetworkSettings(content);

    try {
      const enabling = !networkState?.dedicated_active;
      const hostname = hostInput.value.trim();
      let data = await api.network.toggleDedicatedMac(enabling, hostname);
      if (!data.success && data.conflict) {
        const proceed = confirm(
          `${data.message}\n\nCreate the adapter anyway? Anything already using this hostname on the network may be affected.`
        );
        if (proceed) {
          data = await api.network.toggleDedicatedMac(enabling, hostname, true);
        }
      }
      if (!data.success) {
        networkElevationError = data.message || 'Failed to toggle dedicated adapter.';
      }
    } catch (e: any) {
      networkElevationError = e?.message || 'Network error communicating with server.';
    } finally {
      networkElevationInProgress = false;
      await fetchNetworkState();
      renderNetworkSettings(content);
    }
  });

  const macStatusPill = document.createElement('span');
  if (networkState.dedicated_active) {
    macStatusPill.className = 'network-badge network-badge--success';
    macStatusPill.innerHTML = `✓ Active • IP: ${escapeHtml(networkState.dedicated_adapter?.ipv4 || 'Leased')} • MAC: ${escapeHtml(networkState.dedicated_adapter?.mac_address || '—')}`;
  } else {
    macStatusPill.className = 'network-badge network-badge--info';
    macStatusPill.textContent = 'Inactive (Sharing Host IP & MAC)';
  }

  macActionRow.appendChild(toggleMacBtn);
  macActionRow.appendChild(macStatusPill);
  macCard.appendChild(macActionRow);
  content.appendChild(macCard);

  // 5. Multi-Interface Broadcast & Network Connections Found Card
  const ifacesCard = document.createElement('div');
  ifacesCard.className = 'network-card';

  const ifacesHeader = document.createElement('div');
  ifacesHeader.className = 'network-card-title';
  ifacesHeader.innerHTML = `<span>📶</span> Network Connections Found (${networkState.interfaces.length})`;
  ifacesCard.appendChild(ifacesHeader);

  const broadcastAllRow = document.createElement('div');
  broadcastAllRow.className = 'network-input-row';
  broadcastAllRow.style.marginBottom = '6px';

  const bcastAllCheckbox = document.createElement('input');
  bcastAllCheckbox.type = 'checkbox';
  bcastAllCheckbox.id = 'network-broadcast-all-toggle';
  bcastAllCheckbox.checked = networkState.broadcast_all;
  bcastAllCheckbox.addEventListener('change', () => {
    networkSettingsDirty = true;
  });

  const bcastAllLabel = document.createElement('label');
  bcastAllLabel.htmlFor = 'network-broadcast-all-toggle';
  bcastAllLabel.style.fontSize = '12.5px';
  bcastAllLabel.style.cursor = 'pointer';
  bcastAllLabel.textContent = 'Broadcast across all active network interfaces simultaneously';

  broadcastAllRow.appendChild(bcastAllCheckbox);
  broadcastAllRow.appendChild(bcastAllLabel);
  ifacesCard.appendChild(broadcastAllRow);

  const ifacesGrid = document.createElement('div');
  ifacesGrid.className = 'network-interfaces-grid';

  networkState.interfaces.forEach(iface => {
    const iCard = document.createElement('div');
    iCard.className = 'network-iface-card';

    const header = document.createElement('div');
    header.className = 'network-iface-header';

    const icon = iface.interface_type === 'Ethernet' ? '🖧'
      : iface.interface_type === 'Wireless' ? '📶'
      : iface.interface_type === 'Virtual' ? '🔀'
      : '{icon:network}';

    const title = document.createElement('div');
    title.className = 'network-iface-title';
    title.innerHTML = `<span>${icon}</span> <span>${escapeHtml(iface.display_name)}</span>`;

    const statusBadge = document.createElement('span');
    statusBadge.className = 'network-badge ' + (iface.is_up ? 'network-badge--success' : 'network-badge--info');
    statusBadge.textContent = iface.is_up ? 'UP' : 'DOWN';

    header.appendChild(title);
    header.appendChild(statusBadge);
    iCard.appendChild(header);

    const meta = document.createElement('div');
    meta.className = 'network-iface-meta';
    meta.innerHTML = `
      <div>IP : ${escapeHtml(iface.ipv4 || 'None')}</div>
      <div>MAC: ${escapeHtml(iface.mac_address || '—')}</div>
      ${iface.netmask ? `<div>Mask: ${escapeHtml(iface.netmask)}</div>` : ''}
    `;
    iCard.appendChild(meta);

    const toggleRow = document.createElement('label');
    toggleRow.style.display = 'flex';
    toggleRow.style.alignItems = 'center';
    toggleRow.style.gap = '6px';
    toggleRow.style.fontSize = '11.5px';
    toggleRow.style.cursor = 'pointer';
    toggleRow.style.marginTop = '4px';

    const cb = document.createElement('input');
    cb.type = 'checkbox';
    cb.className = 'network-iface-checkbox';
    cb.dataset.iface = iface.name;
    cb.checked = iface.enabled;
    cb.addEventListener('change', () => {
      networkSettingsDirty = true;
    });

    toggleRow.appendChild(cb);
    toggleRow.appendChild(document.createTextNode('Enable connection'));
    iCard.appendChild(toggleRow);

    ifacesGrid.appendChild(iCard);
  });

  if (networkState.interfaces.length === 0) {
    ifacesGrid.innerHTML = '<div class="settings-empty">No active network interfaces detected.</div>';
  }

  ifacesCard.appendChild(ifacesGrid);
  content.appendChild(ifacesCard);

  // 6. Advanced Network & Security Policies Card
  const secCard = document.createElement('div');
  secCard.className = 'network-card';
  secCard.id = 'network-advanced-card';

  const secTitle = document.createElement('div');
  secTitle.className = 'network-card-title';
  secTitle.innerHTML = '<span>🛡️</span> Advanced Network & Security Policies';
  secCard.appendChild(secTitle);

  const secDesc = document.createElement('div');
  secDesc.className = 'network-card-desc';
  secDesc.textContent = 'Defense-in-depth security policies protecting church production operations from unauthorized cross-origin requests, malicious website interactions, and UI redress / clickjacking.';
  secCard.appendChild(secDesc);

  // Policy 1: CSP Mode
  const cspRow = document.createElement('div');
  cspRow.className = 'network-input-row';
  cspRow.style.flexDirection = 'column';
  cspRow.style.alignItems = 'flex-start';
  cspRow.style.gap = '4px';

  const cspLabel = document.createElement('label');
  cspLabel.htmlFor = 'setting-securityCspMode';
  cspLabel.style.fontSize = '12px';
  cspLabel.style.fontWeight = '600';
  cspLabel.textContent = 'Content Security Policy (CSP)';

  const cspSelect = document.createElement('select');
  cspSelect.className = 'search-input';
  cspSelect.id = 'setting-securityCspMode';
  cspSelect.style.maxWidth = '380px';
  cspSelect.innerHTML = `
    <option value="balanced">Balanced Protection (Recommended — Restricts external scripts, allows sanctuary media)</option>
    <option value="strict">Strict Hardening (Enforces HTTPS & Self Only — Disallows cleartext HTTP media)</option>
    <option value="disabled">Disabled (No CSP header)</option>
  `;
  const currentCsp = getSettingValue('securityCspMode') || 'balanced';
  cspSelect.value = currentCsp;
  cspSelect.addEventListener('change', () => { networkSettingsDirty = true; });

  const cspHelp = document.createElement('div');
  cspHelp.className = 'network-card-desc';
  cspHelp.style.fontSize = '11px';
  cspHelp.textContent = 'Guards the operator console and presentation viewers against cross-site scripting (XSS) and rogue script execution.';

  cspRow.appendChild(cspLabel);
  cspRow.appendChild(cspSelect);
  cspRow.appendChild(cspHelp);
  secCard.appendChild(cspRow);

  // Policy 2: CORS Network Boundary Mode
  const corsRow = document.createElement('div');
  corsRow.className = 'network-input-row';
  corsRow.style.flexDirection = 'column';
  corsRow.style.alignItems = 'flex-start';
  corsRow.style.gap = '4px';

  const corsLabel = document.createElement('label');
  corsLabel.htmlFor = 'setting-securityCorsMode';
  corsLabel.style.fontSize = '12px';
  corsLabel.style.fontWeight = '600';
  corsLabel.textContent = 'CORS Network Boundary Policy';

  const corsSelect = document.createElement('select');
  corsSelect.className = 'search-input';
  corsSelect.id = 'setting-securityCorsMode';
  corsSelect.style.maxWidth = '380px';
  corsSelect.innerHTML = `
    <option value="permissive">Permissive (Default — Allow all LAN clients, Roku, and Android TV displays)</option>
    <option value="restricted">Restricted (Sanctuary Subnet & Localhost Only — Rejects public web origins)</option>
  `;
  const currentCors = getSettingValue('securityCorsMode') || 'permissive';
  corsSelect.value = currentCors;
  corsSelect.addEventListener('change', () => { networkSettingsDirty = true; });

  const corsHelp = document.createElement('div');
  corsHelp.className = 'network-card-desc';
  corsHelp.style.fontSize = '11px';
  corsHelp.textContent = 'Restricted mode prevents malicious external websites visited in an operator browser from issuing background API requests to this console.';

  corsRow.appendChild(corsLabel);
  corsRow.appendChild(corsSelect);
  corsRow.appendChild(corsHelp);
  secCard.appendChild(corsRow);

  // Policy 3: Frame Embedding Policy (X-Frame-Options)
  const frameRow = document.createElement('div');
  frameRow.className = 'network-input-row';
  frameRow.style.flexDirection = 'column';
  frameRow.style.alignItems = 'flex-start';
  frameRow.style.gap = '4px';

  const frameLabel = document.createElement('label');
  frameLabel.htmlFor = 'setting-securityFrameOptions';
  frameLabel.style.fontSize = '12px';
  frameLabel.style.fontWeight = '600';
  frameLabel.textContent = 'Frame Embedding Policy (X-Frame-Options)';

  const frameSelect = document.createElement('select');
  frameSelect.className = 'search-input';
  frameSelect.id = 'setting-securityFrameOptions';
  frameSelect.style.maxWidth = '380px';
  frameSelect.innerHTML = `
    <option value="sameorigin">Same-Origin Only (Recommended — Prevents external iframe clickjacking)</option>
    <option value="deny">Deny All Framing (Blocks all iframes)</option>
    <option value="disabled">Allow All (Permits OBS Studio browser sources & embeds)</option>
  `;
  const currentFrame = getSettingValue('securityFrameOptions') || 'sameorigin';
  frameSelect.value = currentFrame;
  frameSelect.addEventListener('change', () => { networkSettingsDirty = true; });

  const frameHelp = document.createElement('div');
  frameHelp.className = 'network-card-desc';
  frameHelp.style.fontSize = '11px';
  frameHelp.textContent = 'Controls whether third-party web pages can embed your console or live outputs inside an iframe.';

  frameRow.appendChild(frameLabel);
  frameRow.appendChild(frameSelect);
  frameRow.appendChild(frameHelp);
  secCard.appendChild(frameRow);

  content.appendChild(secCard);

  // 7. Save Button in Panel
  const saveRow = document.createElement('div');
  saveRow.style.display = 'flex';
  saveRow.style.justifyContent = 'flex-end';
  saveRow.style.gap = '10px';
  saveRow.style.marginTop = '8px';

  const saveBtn = document.createElement('button');
  saveBtn.className = 'btn btn-primary';
  saveBtn.id = 'btn-save-network-settings';
  saveBtn.innerHTML = '💾 Save Network Settings';

  const saveFeedback = document.createElement('span');
  saveFeedback.className = 'network-badge network-badge--success';
  saveFeedback.style.display = 'none';

  saveBtn.addEventListener('click', async () => {
    saveBtn.disabled = true;
    saveBtn.textContent = 'Saving…';
    const ok = await saveNetworkSettings();
    saveFeedback.style.display = 'inline-flex';
    if (ok) {
      saveFeedback.className = 'network-badge network-badge--success';
      saveFeedback.textContent = '✓ Settings Saved';
    } else {
      saveFeedback.className = 'network-badge network-badge--error';
      saveFeedback.textContent = 'Failed to save settings';
    }
    setTimeout(() => {
      saveFeedback.style.display = 'none';
    }, 3000);
    saveBtn.disabled = false;
    saveBtn.innerHTML = '💾 Save Network Settings';
  });

  saveRow.appendChild(saveFeedback);
  saveRow.appendChild(saveBtn);
  content.appendChild(saveRow);
}

// --- Paired Devices Settings Tab ---

let pairedDevicesLoaded = false;
let pairedDevicesList: Array<{
  id: string;
  name: string;
  platform: string;
  paired_at: number;
  last_seen: number;
  token: string;
}> = [];

async function fetchPairedDevicesState() {
  try {
    pairedDevicesList = await api.pairing.getDevices();
    pairedDevicesLoaded = true;
  } catch (err) {
    console.error('Failed to fetch paired devices:', err);
    pairedDevicesLoaded = true;
  }
}

function formatRelativeTime(epochMs: number): string {
  const diff = Date.now() - epochMs;
  if (diff < 60000) return 'Just now';
  const mins = Math.floor(diff / 60000);
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

export function renderPairedDevicesSettings(content: HTMLElement) {
  content.innerHTML = '';
  content.classList.add('network-panel');

  if (!pairedDevicesLoaded) {
    const loading = document.createElement('div');
    loading.className = 'settings-empty';
    loading.textContent = 'Loading paired display devices…';
    content.appendChild(loading);
    fetchPairedDevicesState().then(() => {
      if (content.isConnected !== false && activeSettingsCategory === 'paired-devices') {
        renderPairedDevicesSettings(content);
      }
    });
    return;
  }

  // 1. Header Overview Card
  const overviewCard = document.createElement('div');
  overviewCard.className = 'network-overview-card';
  overviewCard.style.display = 'flex';
  overviewCard.style.justifyContent = 'space-between';
  overviewCard.style.alignItems = 'center';

  const statGroup = document.createElement('div');
  statGroup.style.display = 'flex';
  statGroup.style.gap = '24px';

  const countStat = document.createElement('div');
  countStat.className = 'network-overview-stat';
  countStat.innerHTML = `
    <div class="network-overview-stat-label">Active TV Displays</div>
    <div class="network-overview-stat-value">
      ${pairedDevicesList.length} ${pairedDevicesList.length === 1 ? 'Device' : 'Devices'}
    </div>
  `;
  statGroup.appendChild(countStat);

  const securityStat = document.createElement('div');
  securityStat.className = 'network-overview-stat';
  securityStat.innerHTML = `
    <div class="network-overview-stat-label">Hardware Pinning</div>
    <div class="network-overview-stat-value">
      <span class="network-badge network-badge--success">Enforced</span>
    </div>
  `;
  statGroup.appendChild(securityStat);
  overviewCard.appendChild(statGroup);

  // Pair New Device Button
  const pairBtn = document.createElement('button');
  pairBtn.className = 'btn btn-primary';
  pairBtn.id = 'btn-paired-devices-pair-tv';
  pairBtn.innerHTML = '➕ Pair TV App';
  pairBtn.addEventListener('click', () => {
    const remoteModal = document.getElementById('remote-modal');
    if (remoteModal) {
      currentContext?.switchToPairingTab?.();
      remoteModal.classList.add('active');
    }
  });
  overviewCard.appendChild(pairBtn);

  // Install via ADB Button -- sideload without the Play Store (docs/CLIENT_PAIRING.md).
  const adbProvisionBtn = document.createElement('button');
  adbProvisionBtn.className = 'btn btn-secondary';
  adbProvisionBtn.id = 'btn-paired-devices-adb-provision';
  adbProvisionBtn.innerHTML = '📲 Install via ADB';
  adbProvisionBtn.style.marginLeft = '8px';
  adbProvisionBtn.addEventListener('click', () => {
    const remoteModal = document.getElementById('remote-modal');
    if (remoteModal) {
      currentContext?.switchToAdbProvisionTab?.();
      remoteModal.classList.add('active');
    }
  });
  overviewCard.appendChild(adbProvisionBtn);

  content.appendChild(overviewCard);

  // 2. Paired Devices List or Empty State
  const sectionTitle = document.createElement('div');
  sectionTitle.className = 'settings-group-header';
  sectionTitle.style.marginTop = '16px';
  sectionTitle.textContent = 'Registered TV & Sanctuary Displays';
  content.appendChild(sectionTitle);

  if (pairedDevicesList.length === 0) {
    const emptyCard = document.createElement('div');
    emptyCard.className = 'settings-empty';
    emptyCard.style.padding = '36px 20px';
    emptyCard.style.textAlign = 'center';
    emptyCard.innerHTML = `
      <div style="font-size: 36px; margin-bottom: 8px;">📺</div>
      <div style="font-weight: 600; font-size: 15px; margin-bottom: 6px; color: var(--text-main);">No Display Clients Paired</div>
      <div style="font-size: 13px; color: var(--text-muted); max-width: 440px; margin: 0 auto 16px; line-height: 1.5;">
        Pair Android TV or Roku sanctuary screens to prevent display hijacking on shared church networks.
      </div>
    `;
    const emptyPairBtn = document.createElement('button');
    emptyPairBtn.className = 'btn btn-primary';
    emptyPairBtn.textContent = 'Pair New TV Display';
    emptyPairBtn.addEventListener('click', () => {
      const remoteModal = document.getElementById('remote-modal');
      if (remoteModal) {
        currentContext?.switchToPairingTab?.();
        remoteModal.classList.add('active');
      }
    });
    emptyCard.appendChild(emptyPairBtn);
    content.appendChild(emptyCard);
  } else {
    const listWrap = document.createElement('div');
    listWrap.className = 'network-card';
    listWrap.style.padding = '0';
    listWrap.style.overflow = 'hidden';

    pairedDevicesList.forEach((device, idx) => {
      const row = document.createElement('div');
      row.className = 'paired-device-row';
      row.style.display = 'flex';
      row.style.alignItems = 'center';
      row.style.justifyContent = 'space-between';
      row.style.padding = '14px 18px';
      row.style.borderBottom = idx < pairedDevicesList.length - 1 ? '1px solid var(--border-color)' : 'none';

      const left = document.createElement('div');
      left.style.display = 'flex';
      left.style.alignItems = 'center';
      left.style.gap = '14px';

      const icon = document.createElement('div');
      icon.style.fontSize = '24px';
      icon.textContent = device.platform.toLowerCase().includes('roku') ? '🟣' : '📺';
      left.appendChild(icon);

      const info = document.createElement('div');
      const isOnline = (Date.now() - device.last_seen) < 120000;
      const statusBadge = isOnline
        ? '<span class="network-badge network-badge--success" style="font-size: 10px;">Online</span>'
        : `<span class="network-badge network-badge--info" style="font-size: 10px;">Seen ${formatRelativeTime(device.last_seen)}</span>`;

      info.innerHTML = `
        <div style="font-weight: 600; font-size: 14px; color: var(--text-main); display: flex; align-items: center; gap: 8px;">
          ${escapeHtml(device.name)}
          <span class="network-badge" style="background: rgba(255,255,255,0.06); font-size: 10px;">${escapeHtml(device.platform)}</span>
          ${statusBadge}
        </div>
        <div style="font-size: 11.5px; color: var(--text-muted); font-family: monospace; margin-top: 2px;">
          ID: ${escapeHtml(device.id)} • Paired: ${new Date(device.paired_at).toLocaleDateString()}
        </div>
      `;
      left.appendChild(info);
      row.appendChild(left);

      const right = document.createElement('div');
      right.style.display = 'flex';
      right.style.alignItems = 'center';
      right.style.gap = '8px';

      const unpairBtn = document.createElement('button');
      unpairBtn.className = 'btn btn-secondary btn-unpair-device';
      unpairBtn.setAttribute('data-device-id', device.id);
      unpairBtn.style.color = '#ff5252';
      unpairBtn.style.borderColor = 'rgba(255, 82, 82, 0.3)';
      unpairBtn.textContent = 'Revoke / Unpair';
      unpairBtn.addEventListener('click', async () => {
        if (!confirm(`Are you sure you want to unpair "${device.name}"? The TV will be disconnected until re-paired.`)) {
          return;
        }
        unpairBtn.disabled = true;
        unpairBtn.textContent = 'Unpairing…';
        try {
          await api.pairing.unpairDevice(device.id);
          await fetchPairedDevicesState();
          renderPairedDevicesSettings(content);
        } catch (err) {
          alert('Failed to unpair device: ' + err);
          unpairBtn.disabled = false;
          unpairBtn.textContent = 'Revoke / Unpair';
        }
      });
      right.appendChild(unpairBtn);
      row.appendChild(right);

      listWrap.appendChild(row);
    });

    content.appendChild(listWrap);
  }

  // 3. Information Banner
  const infoBanner = document.createElement('div');
  infoBanner.className = 'network-card';
  infoBanner.style.marginTop = '16px';
  infoBanner.style.background = 'rgba(0, 229, 255, 0.04)';
  infoBanner.style.border = '1px solid rgba(0, 229, 255, 0.2)';
  infoBanner.innerHTML = `
    <div style="font-size: 12.5px; color: var(--text-main); line-height: 1.5;">
      <strong>🔒 Zero-Auth Web Display Guarantee:</strong> Regular browser displays (<code>/live.html</code>, <code>/stage.html</code>) and volunteer foldback screens do not require authentication or pairing. Pairing is specifically enforced on TV apps to prevent accidental hijacked broadcasts across shared sanctuary networks.
    </div>
  `;
  content.appendChild(infoBanner);
}

// Called from app_ui.ts's ADB-provisioning wizard once a device finishes
// self-authorizing, so the Paired Devices tab reflects it without the
// operator needing to close and reopen Settings.
export async function refreshPairedDevicesAfterAdbProvision() {
  const content = document.getElementById('settings-content');
  if (!content || activeSettingsCategory !== 'paired-devices') return;
  await fetchPairedDevicesState();
  renderPairedDevicesSettings(content);
}


