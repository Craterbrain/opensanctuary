/**
 * OS-Next Theme Editor Modal Controller
 *
 * The real editor for a Theme's typography/category/default/lower-third fields —
 * distinct from theme_picker.ts, which is just the shared "Background" picker
 * (image/CSS/video) reused here for choosing a theme's `background` value.
 */

import { FONT_FAMILY_GROUPS } from "../editor/types.ts";
import { openThemePicker } from "./theme_picker.ts";
import { applyResolvedBackground } from "../core/presentation_helpers.ts";

export interface ThemeEditorContext {
  refreshThemes: () => Promise<void>;
  showToast: (message: string, type?: string) => void;
  escapeHtml: (str: any) => string;
}

let ctx: ThemeEditorContext | null = null;
let editingBackground = '#102027';

export function initThemeEditor(context: ThemeEditorContext) {
  ctx = context;

  document.getElementById('btn-close-theme-editor')?.addEventListener('click', closeThemeEditor);
  document.getElementById('btn-cancel-theme-editor')?.addEventListener('click', closeThemeEditor);
  document.getElementById('btn-save-theme-editor')?.addEventListener('click', saveThemeEditor);
  document.getElementById('btn-theme-editor-choose-bg')?.addEventListener('click', () => {
    openThemePicker(null, (theme) => {
      editingBackground = theme.bg;
      updateBgPreview();
    });
  });

  const categorySelect = document.getElementById('theme-editor-category') as HTMLSelectElement | null;
  categorySelect?.addEventListener('change', () => updateCategoryConditionalFields());

  const chromaToggle = document.getElementById('theme-editor-chroma-enabled') as HTMLInputElement | null;
  chromaToggle?.addEventListener('change', () => {
    const row = document.getElementById('theme-editor-chroma-color-row');
    if (row) row.hidden = !chromaToggle.checked;
  });
}

function updateBgPreview() {
  const preview = document.getElementById('theme-editor-bg-preview');
  if (preview) applyResolvedBackground(preview as HTMLElement, editingBackground);
}

function updateCategoryConditionalFields() {
  const category = (document.getElementById('theme-editor-category') as HTMLSelectElement | null)?.value || 'song';
  const refRow = document.getElementById('theme-editor-reference-position-row');
  if (refRow) refRow.hidden = category !== 'scripture';
  const layoutNote = document.getElementById('theme-editor-layout-note');
  if (layoutNote) layoutNote.hidden = category !== 'presentation';
}

function populateFontSelect() {
  const select = document.getElementById('theme-editor-font-family') as HTMLSelectElement | null;
  if (!select || select.dataset.populated) return;
  select.dataset.populated = '1';
  FONT_FAMILY_GROUPS.forEach(group => {
    const optgroup = document.createElement('optgroup');
    optgroup.label = group.label;
    group.fonts.forEach(font => {
      const opt = document.createElement('option');
      opt.value = font;
      opt.textContent = font;
      optgroup.appendChild(opt);
    });
    select.appendChild(optgroup);
  });
}

function setVal(id: string, value: any) {
  const el = document.getElementById(id) as HTMLInputElement | HTMLSelectElement | null;
  if (el) el.value = value ?? '';
}
function getVal(id: string): string {
  const el = document.getElementById(id) as HTMLInputElement | HTMLSelectElement | null;
  return el ? el.value : '';
}
function setChecked(id: string, checked: boolean) {
  const el = document.getElementById(id) as HTMLInputElement | null;
  if (el) el.checked = checked;
}
function getChecked(id: string): boolean {
  const el = document.getElementById(id) as HTMLInputElement | null;
  return el ? el.checked : false;
}

export function openThemeEditor(theme: any | null) {
  if (!ctx) return;
  populateFontSelect();
  editingBackground = theme ? (theme.background || '#102027') : '#102027';

  const title = document.getElementById('theme-editor-title');
  if (title) title.textContent = theme ? `Edit Theme: ${ctx.escapeHtml(theme.name)}` : 'New Theme';

  setVal('theme-editor-name', theme ? theme.name : '');
  (document.getElementById('theme-editor-name') as HTMLInputElement | null)!.disabled = !!theme;
  setVal('theme-editor-category', theme ? (theme.category || 'song') : 'song');
  setVal('theme-editor-font-family', theme ? (theme.font_family || 'Inter, sans-serif') : 'Inter, sans-serif');
  setVal('theme-editor-font-size', (theme ? theme.font_size : '48px') || '48px');
  setVal('theme-editor-font-color', (theme ? theme.font_color : '#ffffff') || '#ffffff');
  setVal('theme-editor-alignment', theme ? (theme.alignment || 'center') : 'center');
  setVal('theme-editor-vertical-align', theme ? (theme.vertical_align || 'center') : 'center');
  setVal('theme-editor-text-shadow', theme ? (theme.text_shadow || '2px 2px 8px rgba(0,0,0,0.8)') : '2px 2px 8px rgba(0,0,0,0.8)');
  setVal('theme-editor-line-height', (theme ? theme.line_height : '1.3') || '1.3');
  setVal('theme-editor-letter-spacing', (theme ? theme.letter_spacing : '0px') || '0px');
  setVal('theme-editor-reference-position', theme ? (theme.reference_position || 'inline') : 'inline');
  setChecked('theme-editor-chroma-enabled', theme ? !!theme.chroma_key_enabled : false);
  setVal('theme-editor-chroma-color', (theme ? theme.chroma_key_color : '#00ff00') || '#00ff00');
  setVal('theme-editor-safe-area', String(theme ? (theme.safe_area_percent || 0) : 0));

  const chromaRow = document.getElementById('theme-editor-chroma-color-row');
  if (chromaRow) chromaRow.hidden = !getChecked('theme-editor-chroma-enabled');

  updateCategoryConditionalFields();
  updateBgPreview();

  document.querySelectorAll('.context-menu').forEach(m => { (m as HTMLElement).style.display = 'none'; });
  const modal = document.getElementById('theme-editor-modal');
  if (modal) modal.style.display = 'flex';
}

function closeThemeEditor() {
  const modal = document.getElementById('theme-editor-modal');
  if (modal) modal.style.display = 'none';
}

async function saveThemeEditor() {
  if (!ctx) return;
  const name = getVal('theme-editor-name').trim();
  if (!name) {
    ctx.showToast('A theme needs a name.', 'warning');
    return;
  }

  const theme = {
    name,
    background: editingBackground,
    font_family: getVal('theme-editor-font-family') || 'Inter, sans-serif',
    font_size: getVal('theme-editor-font-size') || '48px',
    font_color: getVal('theme-editor-font-color') || '#ffffff',
    alignment: getVal('theme-editor-alignment') || 'center',
    text_shadow: getVal('theme-editor-text-shadow') || 'none',
    line_height: getVal('theme-editor-line-height') || '1.3',
    letter_spacing: getVal('theme-editor-letter-spacing') || '0px',
    opacity: '1.0',
    margin_top: '5%',
    margin_bottom: '5%',
    margin_left: '5%',
    margin_right: '5%',
    vertical_align: getVal('theme-editor-vertical-align') || 'center',
    category: getVal('theme-editor-category') || 'song',
    is_default: false, // set-default is a separate action (context menu), never silently claimed here
    reference_position: getVal('theme-editor-reference-position') || 'inline',
    chroma_key_enabled: getChecked('theme-editor-chroma-enabled'),
    chroma_key_color: getVal('theme-editor-chroma-color') || '#00ff00',
    safe_area_percent: parseInt(getVal('theme-editor-safe-area'), 10) || 0,
  };

  try {
    await fetch('/api/themes', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(theme),
    });
    await ctx.refreshThemes();
    ctx.showToast(`✓ Saved theme "${name}"`, 'success');
    closeThemeEditor();
  } catch (e) {
    ctx.showToast('Failed to save theme.', 'error');
  }
}

export async function setThemeAsDefault(theme: any) {
  if (!ctx || !theme) return;
  try {
    await fetch('/api/themes/default', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ category: theme.category || 'song', name: theme.name }),
    });
    await ctx.refreshThemes();
    ctx.showToast(`✓ "${theme.name}" is now the default ${theme.category || 'song'} theme`, 'success');
  } catch (e) {
    ctx.showToast('Failed to set default theme.', 'error');
  }
}

export async function duplicateTheme(theme: any) {
  if (!ctx || !theme) return;
  let newName = `${theme.name} Copy`;
  let n = 2;
  const existingNames = new Set((await (async () => {
    try {
      const res = await fetch('/api/themes');
      const list = res.ok ? await res.json() : [];
      return list.map((t: any) => t.name.toLowerCase());
    } catch (_) { return []; }
  })()));
  while (existingNames.has(newName.toLowerCase())) {
    newName = `${theme.name} Copy ${n}`;
    n++;
  }
  const duplicate = { ...theme, name: newName, is_default: false };
  try {
    await fetch('/api/themes', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(duplicate),
    });
    await ctx.refreshThemes();
    ctx.showToast(`✓ Duplicated as "${newName}"`, 'success');
  } catch (e) {
    ctx.showToast('Failed to duplicate theme.', 'error');
  }
}

export async function deleteThemeItem(theme: any) {
  if (!ctx || !theme) return;
  if (!confirm(`Delete theme "${theme.name}"? This cannot be undone.`)) return;
  try {
    await fetch(`/api/themes/${encodeURIComponent(theme.name)}`, { method: 'DELETE' });
    await ctx.refreshThemes();
    ctx.showToast(`Deleted theme "${theme.name}"`, 'info');
  } catch (e) {
    ctx.showToast('Failed to delete theme.', 'error');
  }
}
