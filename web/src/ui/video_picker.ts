/**
 * OpenSanctuary / OS-Next Universal Video Background Picker
 *
 * Dedicated controller for the "Videos" tab in the unified Background Modal
 * (#media-image-picker-modal). Integrates:
 * 1. Video Library: Grid of video files with search, durations, and preview
 * 2. From URL: Direct video file path or web stream URL with live preview
 * 3. Online video: the Video Downloader plugin's page (see src/core/plugin_bridge.ts)
 */

import { isVideoBackground } from '../core/presentation_helpers.ts';
import { readClipboard, type ToastType } from '../core/ui_utils.ts';
import { buildGridCard } from './grid_card.ts';
import { renderEmptyState } from './filter_bar.ts';

export interface VideoPickerContext {
  showModal: (el: HTMLElement | null) => void;
  closeModal: (el: HTMLElement | null) => void;
  escapeHtml: (str: any) => string;
  showToast?: (msg: string, type?: ToastType) => void;
}

export interface VideoMediaItem {
  id: string;
  name: string;
  file_path: string;
  duration_seconds?: number | null;
  loop_playback?: boolean;
  thumbnail_path?: string | null;
}

let currentContext: VideoPickerContext | null = null;
let currentVideos: VideoMediaItem[] = [];
let selectedVideoPath: string | null = null;
let selectedVideoName: string | null = null;
let onSelectSlideCallback: ((filePath: string, cssBg: string, name: string, isVideo: boolean, isLooping: boolean) => void) | null = null;
let onSelectAllCallback: ((filePath: string, cssBg: string, name: string, isVideo: boolean, isLooping: boolean) => void) | null = null;

/** Switches between the three sub-views inside the Videos tab. */
export function setVideoSubtab(tab: 'library' | 'url' | 'ytdl') {
  const btnLib = document.getElementById('btn-video-subtab-library');
  const btnUrl = document.getElementById('btn-video-subtab-url');
  const btnYtdl = document.getElementById('btn-video-subtab-ytdl');

  if (btnLib) btnLib.classList.toggle('active', tab === 'library');
  if (btnUrl) btnUrl.classList.toggle('active', tab === 'url');
  if (btnYtdl) btnYtdl.classList.toggle('active', tab === 'ytdl');

  const viewLib = document.getElementById('video-subview-library');
  const viewUrl = document.getElementById('video-subview-url');
  const viewYtdl = document.getElementById('video-subview-ytdl');
  const searchWrap = document.getElementById('video-library-search-wrap');

  if (viewLib) viewLib.style.display = tab === 'library' ? 'flex' : 'none';
  if (viewUrl) viewUrl.style.display = tab === 'url' ? 'flex' : 'none';
  if (viewYtdl) viewYtdl.style.display = tab === 'ytdl' ? 'flex' : 'none';
  if (searchWrap) searchWrap.style.display = tab === 'library' ? 'flex' : 'none';
}

export function initVideoPicker(context: VideoPickerContext) {
  currentContext = context;

  // 1. Subtab button click listeners
  const btnLib = document.getElementById('btn-video-subtab-library');
  const btnUrl = document.getElementById('btn-video-subtab-url');
  const btnYtdl = document.getElementById('btn-video-subtab-ytdl');

  if (btnLib) btnLib.addEventListener('click', () => setVideoSubtab('library'));
  if (btnUrl) btnUrl.addEventListener('click', () => setVideoSubtab('url'));
  if (btnYtdl) btnYtdl.addEventListener('click', () => setVideoSubtab('ytdl'));

  // 2. Search filter in Library view
  const searchInput = document.getElementById('video-picker-search') as HTMLInputElement | null;
  if (searchInput) {
    searchInput.addEventListener('input', () => {
      renderVideoLibraryGrid();
    });
  }

  // 3. Direct URL preview & paste
  const urlInput = document.getElementById('video-url-input') as HTMLInputElement | null;
  const btnUrlPaste = document.getElementById('btn-video-url-paste');
  const btnUrlPreview = document.getElementById('btn-video-url-preview');
  const urlPreviewContainer = document.getElementById('video-url-preview-container');
  const urlPreviewVideo = document.getElementById('video-url-preview-element') as HTMLVideoElement | null;
  const urlStatus = document.getElementById('video-url-status');

  if (btnUrlPaste && urlInput) {
    btnUrlPaste.addEventListener('click', async () => {
      try {
        const text = await readClipboard();
        if (text) {
          urlInput.value = text.trim();
          urlInput.dispatchEvent(new Event('input'));
        }
      } catch (_) {}
    });
  }

  const handleUrlUpdate = () => {
    const val = (urlInput?.value || '').trim();
    if (!val) {
      if (urlPreviewContainer) urlPreviewContainer.style.display = 'none';
      if (urlStatus) urlStatus.textContent = '';
      return;
    }
    selectedVideoPath = val;
    selectedVideoName = val.split('/').pop() || 'Custom Video URL';

    if (urlPreviewContainer && urlPreviewVideo) {
      urlPreviewContainer.style.display = 'flex';
      urlPreviewVideo.src = val;
      urlPreviewVideo.load();
      if (urlStatus) {
        urlStatus.textContent = `Selected: "${selectedVideoName}"`;
        urlStatus.style.color = 'var(--os-success-text)';
      }
    }
  };

  if (btnUrlPreview) btnUrlPreview.addEventListener('click', handleUrlUpdate);
  if (urlInput) urlInput.addEventListener('change', handleUrlUpdate);

  // 4. Online video download: the Video Downloader plugin puts its page in #plugin-picker-pane;
  //    until it is turned on, the pane shows a pointer to Settings > Plugins.
  document.getElementById('btn-picker-enable-ytdlp')?.addEventListener('click', () => {
    window.dispatchEvent(new CustomEvent('os-open-plugins-settings'));
  });
  // A finished download lands in the library: pick it up without reopening the dialog.
  window.addEventListener('os-library-changed', (ev) => {
    if ((ev as CustomEvent).detail?.category === 'media') void fetchAvailableVideos().then(() => renderVideoLibraryGrid());
  });

  // 5. Apply Buttons
  const btnApplySlide = document.getElementById('btn-apply-video-slide');
  if (btnApplySlide) {
    btnApplySlide.addEventListener('click', () => {
      applyCurrentVideoSelection(false);
    });
  }

  const btnApplyAll = document.getElementById('btn-apply-video-all');
  if (btnApplyAll) {
    btnApplyAll.addEventListener('click', () => {
      applyCurrentVideoSelection(true);
    });
  }
}

function applyCurrentVideoSelection(applyToAll: boolean) {
  if (!selectedVideoPath) {
    if (currentContext?.showToast) currentContext.showToast('Please select a video or enter a URL first.', 'warning');
    return;
  }

  const loopCheckbox = document.getElementById('video-picker-loop') as HTMLInputElement | null;
  const isLooping = loopCheckbox ? loopCheckbox.checked : true;
  const name = selectedVideoName || selectedVideoPath.split('/').pop() || 'Video Background';

  if (applyToAll && onSelectAllCallback) {
    onSelectAllCallback(selectedVideoPath, selectedVideoPath, name, true, isLooping);
  } else if (onSelectSlideCallback) {
    onSelectSlideCallback(selectedVideoPath, selectedVideoPath, name, true, isLooping);
  }

  const modal = document.getElementById('media-image-picker-modal');
  if (modal && currentContext) {
    currentContext.closeModal(modal);
  }
}

export async function fetchAvailableVideos(): Promise<VideoMediaItem[]> {
  try {
    const res = await fetch('/api/media');
    if (res.ok) {
      const data = await res.json();
      if (Array.isArray(data)) {
        currentVideos = data
          .filter(m => {
            const mType = (m.media_type || '').toLowerCase();
            const path = (m.file_path || '').toLowerCase();
            return mType === 'video' || isVideoBackground(path);
          })
          .map(m => ({
            id: m.id,
            name: m.name || m.file_path.split('/').pop() || 'Untitled Video',
            file_path: m.file_path,
            duration_seconds: m.duration_seconds,
            loop_playback: m.loop_playback !== false,
            thumbnail_path: m.thumbnail_path
          }));
      }
    }
  } catch (err) {
    console.warn('Failed to load videos from /api/media:', err);
  }
  return currentVideos;
}

export function getAvailableVideos(): VideoMediaItem[] {
  return currentVideos;
}

export async function prepareVideoPickerGrid(opts: {
  currentVideo?: string | null;
  onSelectSlide?: ((filePath: string, cssBg: string, name: string, isVideo: boolean, isLooping: boolean) => void) | null;
  onSelectAll?: ((filePath: string, cssBg: string, name: string, isVideo: boolean, isLooping: boolean) => void) | null;
  isStudio?: boolean;
} = {}) {
  onSelectSlideCallback = opts.onSelectSlide || null;
  onSelectAllCallback = opts.onSelectAll || null;

  const btnAll = document.getElementById('btn-apply-video-all');
  if (btnAll) {
    btnAll.style.display = opts.onSelectAll ? 'inline-flex' : 'none';
  }

  const btnSlide = document.getElementById('btn-apply-video-slide');
  if (btnSlide) {
    btnSlide.textContent = opts.isStudio ? 'Apply to Active Slide' : 'Apply Video Background';
  }

  await fetchAvailableVideos();

  if (opts.currentVideo) {
    selectedVideoPath = opts.currentVideo;
    const found = currentVideos.find(v => v.file_path === opts.currentVideo);
    selectedVideoName = found ? found.name : opts.currentVideo.split('/').pop() || 'Video';
  } else if (currentVideos.length > 0 && !selectedVideoPath) {
    selectedVideoPath = currentVideos[0].file_path;
    selectedVideoName = currentVideos[0].name;
  }

  setVideoSubtab('library');
  renderVideoLibraryGrid();
}

export function renderVideoLibraryGrid() {
  const gridEl = document.getElementById('media-video-picker-grid');
  if (!gridEl) return;
  gridEl.innerHTML = '';

  const searchInput = document.getElementById('video-picker-search') as HTMLInputElement | null;
  const query = (searchInput?.value || '').toLowerCase().trim();

  const filtered = currentVideos.filter(v => {
    if (query && !v.name.toLowerCase().includes(query) && !v.file_path.toLowerCase().includes(query)) {
      return false;
    }
    return true;
  });

  const emptyEl = document.getElementById('media-video-picker-empty');
  if (emptyEl) {
    emptyEl.hidden = filtered.length > 0;
    if (filtered.length === 0) {
      renderEmptyState(emptyEl, {
        icon: '🎬',
        title: 'No Videos Found',
        description: query ? `No video backgrounds matching "${query}".` : 'No video backgrounds available in the library.',
        actionLabel: query ? 'Clear Search' : undefined,
        onAction: query ? () => {
          const sInput = document.getElementById('video-picker-search') as HTMLInputElement | null;
          if (sInput) sInput.value = '';
          renderVideoLibraryGrid();
        } : undefined
      });
    }
  }
  if (filtered.length === 0) return;

  filtered.forEach(vid => {
    const isSelected = selectedVideoPath === vid.file_path;
    const card = buildGridCard({
      id: vid.file_path,
      title: vid.name,
      thumbnail: {
        type: 'video',
        value: vid.file_path,
        durationSec: vid.duration_seconds
      },
      isSelected,
      className: 'media-video-card',
      thumbClassName: 'media-video-thumb',
      titleClassName: 'media-video-info',
      onClick: () => {
        selectedVideoPath = vid.file_path;
        selectedVideoName = vid.name;
        gridEl.querySelectorAll('.media-video-card').forEach(c => c.classList.remove('selected'));
        card.classList.add('selected');
      },
      onDoubleClick: () => {
        selectedVideoPath = vid.file_path;
        selectedVideoName = vid.name;
        applyCurrentVideoSelection(false);
      }
    });

    if (card) {
      gridEl.appendChild(card);
    }
  });
}
