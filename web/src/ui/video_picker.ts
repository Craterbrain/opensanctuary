/**
 * OpenSanctuary / OS-Next Universal Video Background Picker
 *
 * Dedicated controller for the "Videos" tab in the unified Background Modal
 * (#media-image-picker-modal). Integrates:
 * 1. Video Library: Grid of video files with search, durations, and preview
 * 2. From URL: Direct video file path or web stream URL with live preview
 * 3. yt-dlp Downloader: Inline YouTube/web clip downloader with live progress polling
 */

import { isVideoBackground } from '../core/presentation_helpers.ts';
import { readClipboard } from '../core/ui_utils.ts';
import { buildGridCard } from './grid_card.ts';
import { renderEmptyState } from './filter_bar.ts';

export interface VideoPickerContext {
  showModal: (el: HTMLElement | null) => void;
  closeModal: (el: HTMLElement | null) => void;
  escapeHtml: (str: any) => string;
  showToast?: (msg: string, type?: string) => void;
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
let ytdlpPollingInterval: any = null;

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
        urlStatus.style.color = '#81c784';
      }
    }
  };

  if (btnUrlPreview) btnUrlPreview.addEventListener('click', handleUrlUpdate);
  if (urlInput) urlInput.addEventListener('change', handleUrlUpdate);

  // 4. yt-dlp Online Video Downloader
  const ytdlUrlInput = document.getElementById('video-ytdl-url-input') as HTMLInputElement | null;
  const btnYtdlPaste = document.getElementById('btn-video-ytdl-paste');
  const btnYtdlStart = document.getElementById('btn-video-ytdl-start') as HTMLButtonElement | null;
  const ytdlProgressContainer = document.getElementById('video-ytdl-progress-container');
  const ytdlProgressFill = document.getElementById('video-ytdl-progress-fill');
  const ytdlStatus = document.getElementById('video-ytdl-status');
  const ytdlBrowserSelect = document.getElementById('video-ytdl-browser') as HTMLSelectElement | null;
  const ytdlSponsorblock = document.getElementById('video-ytdl-sponsorblock') as HTMLInputElement | null;

  if (btnYtdlPaste && ytdlUrlInput) {
    btnYtdlPaste.addEventListener('click', async () => {
      try {
        const text = await readClipboard();
        if (text) {
          ytdlUrlInput.value = text.trim();
        }
      } catch (_) {}
    });
  }

  if (btnYtdlStart) {
    btnYtdlStart.addEventListener('click', async () => {
      const url = (ytdlUrlInput?.value || '').trim();
      if (!url) {
        if (ytdlStatus) {
          ytdlStatus.innerHTML = '<span style="color: #ff5252;">⚠️ Please enter or paste a video URL.</span>';
        }
        if (ytdlUrlInput) ytdlUrlInput.focus();
        return;
      }

      const browser = ytdlBrowserSelect ? ytdlBrowserSelect.value : 'auto';
      const sponsorblock = ytdlSponsorblock ? ytdlSponsorblock.checked : true;

      if (ytdlProgressContainer) ytdlProgressContainer.style.display = 'block';
      if (ytdlProgressFill) ytdlProgressFill.style.width = '0%';
      if (btnYtdlStart) btnYtdlStart.disabled = true;

      if (ytdlStatus) {
        ytdlStatus.innerHTML = '<span style="color: #00e5ff;">⏳ Initializing download with yt-dlp...</span>';
      }

      try {
        const res = await fetch('/api/media/ytdlp/download', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            url,
            browser,
            sponsorblock_remove_all: sponsorblock,
            audio_only: false
          })
        });

        if (!res.ok) {
          const errText = await res.text();
          throw new Error(errText || 'Download failed to initialize');
        }

        const initProgress = await res.json();
        const taskId = initProgress.task_id;

        if (ytdlpPollingInterval) clearInterval(ytdlpPollingInterval);
        let failCount = 0;

        ytdlpPollingInterval = setInterval(async () => {
          try {
            const progRes = await fetch(`/api/media/ytdlp/progress/${encodeURIComponent(taskId)}`);
            if (!progRes.ok) {
              failCount++;
              if (failCount >= 5) {
                clearInterval(ytdlpPollingInterval);
                ytdlpPollingInterval = null;
                if (btnYtdlStart) btnYtdlStart.disabled = false;
                if (ytdlStatus) ytdlStatus.innerHTML = '<span style="color: #ff5252;">❌ Download progress connection lost.</span>';
              }
              return;
            }

            failCount = 0;
            const progress = await progRes.json();
            const pct = Math.min(Math.max(progress.percent || 0, 0), 100);

            if (ytdlProgressFill) ytdlProgressFill.style.width = `${pct}%`;

            const speed = progress.speed ? ` • ${progress.speed}` : '';
            const eta = progress.eta ? ` • ETA ${progress.eta}` : '';

            if (ytdlStatus) {
              ytdlStatus.innerHTML = `
                <div style="display: flex; align-items: center; justify-content: space-between; gap: 8px;">
                  <span style="color: #00e5ff;">${currentContext!.escapeHtml(progress.status || 'Downloading...')}${speed}${eta}</span>
                  <span style="font-weight: 700; color: #00e676;">${pct.toFixed(1)}%</span>
                </div>`;
            }

            if (progress.is_complete) {
              clearInterval(ytdlpPollingInterval);
              ytdlpPollingInterval = null;
              if (btnYtdlStart) btnYtdlStart.disabled = false;

              if (progress.error) {
                if (ytdlStatus) {
                  ytdlStatus.innerHTML = `<span style="color: #ff5252;">❌ Download failed: ${currentContext!.escapeHtml(progress.error)}</span>`;
                }
              } else {
                const item = progress.media_item;
                const title = item ? item.name : 'Downloaded Video';
                if (ytdlProgressFill) ytdlProgressFill.style.width = '100%';
                if (ytdlStatus) {
                  ytdlStatus.innerHTML = `<span style="color: #4caf50; font-weight: 600;">✓ Successfully downloaded "${currentContext!.escapeHtml(title)}"!</span>`;
                }
                if (ytdlUrlInput) ytdlUrlInput.value = '';

                // Refresh video list from server
                await fetchAvailableVideos();

                // Select newly downloaded item
                if (item && item.file_path) {
                  selectedVideoPath = item.file_path;
                  selectedVideoName = item.name;
                }

                // Switch back to Library subtab to show newly added video
                setTimeout(() => {
                  setVideoSubtab('library');
                  renderVideoLibraryGrid();
                }, 600);
              }
            }
          } catch (pollErr) {
            console.warn('yt-dlp poll error:', pollErr);
          }
        }, 400);
      } catch (err: any) {
        if (ytdlpPollingInterval) {
          clearInterval(ytdlpPollingInterval);
          ytdlpPollingInterval = null;
        }
        if (btnYtdlStart) btnYtdlStart.disabled = false;
        if (ytdlStatus) {
          ytdlStatus.innerHTML = `<span style="color: #ff5252;">❌ Error: ${currentContext!.escapeHtml(err.message)}</span>`;
        }
      }
    });
  }

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
