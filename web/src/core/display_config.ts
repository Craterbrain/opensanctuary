/**
 * Configured native "Display" output slots — windowless, always-on-top windows the
 * desktop app can open on a physical monitor, each showing one of the existing
 * live-rendered pages (Live Output FOH, Stage Foldback). Persisted as a single JSON
 * string under the "displayOutputs" key via the existing generic /api/settings
 * key-value store (see app_core.ts's appOptions/saveAppOptions).
 *
 * Only two content sources exist today, but `contentPath` is a plain string (not a
 * union) and resolution is its own variant type so a third source or resolution mode
 * can be added later without reshaping this config or its persisted JSON.
 */

export type DisplayResolutionMode = 'native' | 'fixed';

export interface DisplayOutputConfig {
  id: string;
  label: string;
  /** Path under the web root, e.g. "live.html" or "stage.html". */
  contentPath: string;
  monitorIndex: number;
  resolutionMode: DisplayResolutionMode;
  fixedWidth?: number;
  fixedHeight?: number;
  alwaysOnTop: boolean;
}

export const DISPLAY_CONTENT_SOURCES: { value: string; label: string }[] = [
  { value: 'live.html', label: 'Live Output (Front of House)' },
  { value: 'stage.html', label: 'Stage Foldback' },
];

export function parseDisplayOutputs(raw: string | undefined | null): DisplayOutputConfig[] {
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed : [];
  } catch (_) {
    return [];
  }
}

export function serializeDisplayOutputs(list: DisplayOutputConfig[]): string {
  return JSON.stringify(list);
}

export function createDefaultDisplayOutput(existing: DisplayOutputConfig[]): DisplayOutputConfig {
  return {
    id: `display-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
    label: `Display ${existing.length + 1}`,
    contentPath: 'live.html',
    monitorIndex: existing.length,
    resolutionMode: 'native',
    alwaysOnTop: true,
  };
}
