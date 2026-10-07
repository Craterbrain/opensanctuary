/**
 * schedule_drop.ts
 *
 * Native OS desktop drag-and-drop ingestion handler for schedule files (.ewsx, .ews, .osj, .json).
 * Renders an intuitive split drop-target over #schedule-panel:
 * - Top half: "Drop to Replace Schedule"
 * - Bottom half: "Drop to Append to Schedule"
 */

import { appStore } from './state';
import { resolveHostSessionToken, hostTokenHeader } from './host_session.ts';

export interface ScheduleDropOptions {
  panelSelector?: string;
  onLoaded?: (snapshot: any, mode: 'replace' | 'append') => void;
  showToast?: (message: string, type?: 'info' | 'success' | 'warning' | 'error') => void;
}

export function isSupportedScheduleFile(filename: string): boolean {
  const lower = filename.toLowerCase();
  return lower.endsWith('.ewsx') || lower.endsWith('.ews') || lower.endsWith('.ewpx') || lower.endsWith('.osz') || lower.endsWith('.osj') || lower.endsWith('.json');
}

export function isBinaryScheduleFile(filename: string): boolean {
  const lower = filename.toLowerCase();
  return lower.endsWith('.ewsx') || lower.endsWith('.ews') || lower.endsWith('.ewpx') || lower.endsWith('.osz');
}

export async function fileToBase64(file: File): Promise<string> {
  const buffer = await file.arrayBuffer();
  const bytes = new Uint8Array(buffer);
  let binaryStr = '';
  const chunkSize = 8192;
  for (let i = 0; i < bytes.length; i += chunkSize) {
    binaryStr += String.fromCharCode.apply(null, Array.from(bytes.subarray(i, i + chunkSize)));
  }
  return btoa(binaryStr);
}

export async function uploadScheduleFile(
  file: File,
  mode: 'replace' | 'append'
): Promise<any> {
  const isBinary = isBinaryScheduleFile(file.name);
  let payload: { file_name: string; file_data_base64?: string; file_text?: string; mode: 'replace' | 'append' };

  if (isBinary) {
    const b64 = await fileToBase64(file);
    payload = {
      file_name: file.name,
      file_data_base64: b64,
      mode
    };
  } else {
    const text = await file.text();
    payload = {
      file_name: file.name,
      file_text: text,
      mode
    };
  }

  await resolveHostSessionToken();
  const res = await fetch('/api/schedule/open', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', ...hostTokenHeader() },
    body: JSON.stringify(payload)
  });

  if (!res.ok) {
    const errText = await res.text();
    throw new Error(errText || `Server responded with ${res.status}`);
  }

  return await res.json();
}

export function setupScheduleDesktopDrop(options?: ScheduleDropOptions): () => void {
  const panelSelector = options?.panelSelector || '#schedule-panel';
  const panel = document.querySelector(panelSelector) as HTMLElement | null;
  if (!panel) return () => {};

  const toast = options?.showToast || (() => {});

  // Ensure drop overlay DOM exists inside panel
  let overlay = panel.querySelector('#schedule-drop-overlay') as HTMLElement | null;
  if (!overlay) {
    overlay = document.createElement('div');
    overlay.id = 'schedule-drop-overlay';
    overlay.className = 'schedule-drop-overlay';
    overlay.style.display = 'none';
    overlay.innerHTML = `
      <div class="schedule-drop-zone schedule-drop-zone-replace" id="schedule-drop-zone-replace" data-mode="replace">
        <div class="schedule-drop-zone-icon">🔄</div>
        <div class="schedule-drop-zone-title">Drop to Replace Schedule</div>
        <div class="schedule-drop-zone-desc">Replaces active service schedule</div>
      </div>
      <div class="schedule-drop-zone schedule-drop-zone-append" id="schedule-drop-zone-append" data-mode="append">
        <div class="schedule-drop-zone-icon">➕</div>
        <div class="schedule-drop-zone-title">Drop to Append to Schedule</div>
        <div class="schedule-drop-zone-desc">Adds onto end of current schedule</div>
      </div>
    `;
    panel.appendChild(overlay);
  }

  const zoneReplace = overlay.querySelector('#schedule-drop-zone-replace') as HTMLElement | null;
  const zoneAppend = overlay.querySelector('#schedule-drop-zone-append') as HTMLElement | null;

  let dragCounter = 0;

  const showOverlay = () => {
    if (overlay) overlay.style.display = 'flex';
  };

  const hideOverlay = () => {
    dragCounter = 0;
    if (overlay) overlay.style.display = 'none';
    if (zoneReplace) zoneReplace.classList.remove('active-zone');
    if (zoneAppend) zoneAppend.classList.remove('active-zone');
  };

  const onDragEnter = (e: DragEvent) => {
    if (!e.dataTransfer || !Array.from(e.dataTransfer.types).includes('Files')) return;
    dragCounter++;
    showOverlay();
    e.preventDefault();
  };

  const onDragOver = (e: DragEvent) => {
    if (!e.dataTransfer || !Array.from(e.dataTransfer.types).includes('Files')) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'copy';
    showOverlay();

    if (overlay) {
      const rect = overlay.getBoundingClientRect();
      const midY = rect.top + rect.height / 2;
      if (e.clientY < midY) {
        zoneReplace?.classList.add('active-zone');
        zoneAppend?.classList.remove('active-zone');
      } else {
        zoneAppend?.classList.add('active-zone');
        zoneReplace?.classList.remove('active-zone');
      }
    }
  };

  const onDragLeave = (e: DragEvent) => {
    if (!e.dataTransfer || !Array.from(e.dataTransfer.types).includes('Files')) return;
    dragCounter--;
    if (dragCounter <= 0) {
      hideOverlay();
    }
  };

  const onDrop = async (e: DragEvent) => {
    if (!e.dataTransfer || !Array.from(e.dataTransfer.types).includes('Files')) return;
    e.preventDefault();
    e.stopPropagation();

    let mode: 'replace' | 'append' = 'replace';
    if (overlay) {
      const rect = overlay.getBoundingClientRect();
      const midY = rect.top + rect.height / 2;
      mode = (e.clientY >= midY) ? 'append' : 'replace';
    }

    hideOverlay();

    const files = e.dataTransfer.files;
    if (!files || files.length === 0) return;
    const file = files[0];

    if (!isSupportedScheduleFile(file.name)) {
      toast(`Unsupported schedule format: "${file.name}". Expected .ewsx, .ews, .ewpx, .osz, or .osj file.`, 'warning');
      return;
    }

    toast(`Reading ${file.name}...`, 'info');

    try {
      const snapshot = await uploadScheduleFile(file, mode);
      appStore.setSnapshot(snapshot);
      if (options?.onLoaded) {
        options.onLoaded(snapshot, mode);
      }
      if (mode === 'append') {
        toast(`✓ Successfully appended items from '${file.name}' to schedule`, 'success');
      } else {
        toast(`✓ Successfully loaded schedule '${snapshot?.schedule?.title || file.name}'`, 'success');
      }
    } catch (err: any) {
      toast(`Error loading schedule: ${err.message || err}`, 'error');
    }
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Escape' && overlay?.style.display !== 'none') {
      hideOverlay();
    }
  };

  panel.addEventListener('dragenter', onDragEnter);
  panel.addEventListener('dragover', onDragOver);
  panel.addEventListener('dragleave', onDragLeave);
  panel.addEventListener('drop', onDrop);
  window.addEventListener('keydown', onKeyDown);

  return () => {
    panel.removeEventListener('dragenter', onDragEnter);
    panel.removeEventListener('dragover', onDragOver);
    panel.removeEventListener('dragleave', onDragLeave);
    panel.removeEventListener('drop', onDrop);
    window.removeEventListener('keydown', onKeyDown);
    if (overlay && overlay.parentNode) {
      overlay.parentNode.removeChild(overlay);
    }
  };
}
