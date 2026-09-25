// Bundled entry point for live.html — the actual projector/FOH output. This
// was previously an inline, un-bundled <script> baked directly into live.html
// (verbatim logic preserved below); it's now a real ES module so it can share
// the canvas editor's element renderer with the in-app Live preview, making
// what an operator designs (positioned text/images/shapes/tables) actually
// appear on the real output instead of only a flattened text string.
import { computeCanvasProjection } from './core/canvas_projection';
import { renderSlideElements } from './core/slide_render';
import { autoFitLyrics as autoFitLyricsCore } from './core/autofit';
import {
  applyResolvedBackground,
  parseParallelSlide,
  extractImageUrl,
  isVideoBackground,
  isAudioMedia,
  resolveSlideAt,
  resolveSlideBackground,
  resolveThemeAt,
  applyThemeTypography,
  escapeHtml,
  ThemeDefinition,
  DEFAULT_THEMES
} from './core/presentation_helpers';
import { clock } from './core/timesync';
import { createEngineWebSocket, EngineWebSocketClient } from './core/ws_client';

export class NtpClock {
  now(): number {
    return clock.now();
  }
}
export const ntpClock = new NtpClock();

// Themes list for typography/background-by-name resolution (see resolveThemeAt) —
// fetched once at startup and periodically refreshed, mirroring how the operator
// console caches it (app_core.ts refreshAvailableThemes).
let availableThemes: ThemeDefinition[] = DEFAULT_THEMES;
async function refreshLiveThemes() {
  try {
    const res = await fetch('/api/themes');
    if (res.ok) {
      const dbThemes = await res.json();
      if (Array.isArray(dbThemes) && dbThemes.length > 0) {
        availableThemes = dbThemes.map((t: any) => ({
          name: t.name,
          bg: t.background || 'linear-gradient(135deg, #0f2027, #203a43, #2c5364)',
          font: t.font_family,
          color: t.font_color,
          category: t.category || 'song',
          isDefault: !!t.is_default,
          fontSize: t.font_size,
          textShadow: t.text_shadow,
          alignment: t.alignment,
          lineHeight: t.line_height,
          letterSpacing: t.letter_spacing,
          marginTop: t.margin_top,
          marginBottom: t.margin_bottom,
          marginLeft: t.margin_left,
          marginRight: t.margin_right,
          referencePosition: t.reference_position || 'inline',
          chromaKeyEnabled: !!t.chroma_key_enabled,
          chromaKeyColor: t.chroma_key_color,
          safeAreaPercent: t.safe_area_percent || 0,
        }));
      }
    }
  } catch (_) { /* keep DEFAULT_THEMES fallback */ }
}
refreshLiveThemes();
setInterval(refreshLiveThemes, 20000);

// --- TIME SYNC & VIDEO SYNCPLAY ALGORITHM ---
clock.startPeriodicSync(15000);

let mediaSyncChannel: BroadcastChannel | null = null;
let liveWs: WebSocket | null = null;
let liveWsClient: EngineWebSocketClient | null = null;

class TimecodeSyncEngine {
  videoEl: HTMLVideoElement;
  audioEl: HTMLAudioElement;
  masterState: any = null;
  syncTimer: any = null;
  scheduledStartRaf: number | null = null;
  _scheduledTimeout: any = null;
  _scheduledTargetMs: number | undefined;
  _playStartedAt: number | undefined;
  _lastReportedReadyVersion: number | undefined;

  constructor(videoEl: HTMLVideoElement, audioEl: HTMLAudioElement) {
    this.videoEl = videoEl;
    this.audioEl = audioEl;
    this.startSyncClock();
  }

  setMasterState(mediaPlayback: any, isDedicatedMedia: boolean) {
    if (!mediaPlayback) {
      this.masterState = null;
      if (this.scheduledStartRaf) cancelAnimationFrame(this.scheduledStartRaf);
      if (this.videoEl && !this.videoEl.paused) this.videoEl.pause();
      if (this.audioEl && !this.audioEl.paused) this.audioEl.pause();
      return;
    }

    this.masterState = {
      isPlaying: !!mediaPlayback.is_playing,
      baseTime: Number(mediaPlayback.current_time) || 0,
      timestamp: Number(mediaPlayback.timestamp_ms) || clock.now(),
      startAtEpochMs: mediaPlayback.start_at_epoch_ms ? Number(mediaPlayback.start_at_epoch_ms) : null,
      prerollTargetPts: mediaPlayback.preroll_target_pts !== undefined && mediaPlayback.preroll_target_pts !== null ? Number(mediaPlayback.preroll_target_pts) : null,
      syncVersion: mediaPlayback.sync_version || 0,
      isLooping: !isDedicatedMedia || !!mediaPlayback.is_looping,
      isMuted: !isDedicatedMedia || (mediaPlayback.is_muted !== undefined ? !!mediaPlayback.is_muted : false),
      volume: mediaPlayback.volume !== undefined ? Number(mediaPlayback.volume) : 1.0,
    };

    this.syncNow(true);
  }

  calculateTargetTime(el: HTMLMediaElement) {
    if (!this.masterState) return 0;
    if (!this.masterState.isPlaying || this.masterState.prerollTargetPts !== null) {
      return (this.masterState.prerollTargetPts !== null) ? this.masterState.prerollTargetPts : this.masterState.baseTime;
    }

    const now = clock.now();
    const startAnchor = this.masterState.startAtEpochMs || this.masterState.timestamp;

    if (this.masterState.startAtEpochMs && now < this.masterState.startAtEpochMs) {
      return this.masterState.baseTime;
    }

    const elapsedSec = Math.max(0, (now - startAnchor) / 1000.0);
    let target = this.masterState.baseTime + elapsedSec;

    if (this.masterState.isLooping && el && el.duration > 0 && !isNaN(el.duration)) {
      target = target % el.duration;
    }
    return target;
  }

  syncElement(el: any, isVideo: boolean, forceHardAlign: boolean) {
    if (!el || el.style.display === 'none' || !this.masterState) return;

    if (el.readyState < 2) {
      if (!el._initListenerAttached) {
        el._initListenerAttached = true;
        const onInitReady = () => {
          el._initListenerAttached = false;
          this.syncElement(el, isVideo, true);
        };
        el.addEventListener('loadeddata', onInitReady, { once: true });
        el.addEventListener('canplay', onInitReady, { once: true });
      }
      return;
    }

    if (el.seeking) return;

    const isPlaying = this.masterState.isPlaying;
    const now = clock.now();
    const isFutureScheduled = this.masterState.startAtEpochMs && now < this.masterState.startAtEpochMs;
    const isPreroll = this.masterState.prerollTargetPts !== null;
    const targetTime = this.calculateTargetTime(el);

    el.loop = this.masterState.isLooping;
    el.muted = this.masterState.isMuted;
    if (el.volume !== this.masterState.volume) {
      try { el.volume = this.masterState.volume; } catch (_) {}
    }

    if (isPreroll) {
      if (!el.paused) {
        el.pause();
        el._isPlayPromisePending = false;
      }
      if (Math.abs(el.currentTime - targetTime) > 0.02) {
        try { el.currentTime = targetTime; } catch (_) {}
      }
      el.playbackRate = 1.0;

      if (el.readyState >= 3 && !el.seeking) {
        this.reportBufferReady(targetTime, this.masterState.syncVersion);
      } else {
        const onSeeked = () => {
          el.removeEventListener('seeked', onSeeked);
          el.removeEventListener('canplay', onSeeked);
          this.reportBufferReady(targetTime, this.masterState.syncVersion);
        };
        el.addEventListener('seeked', onSeeked, { once: true });
        el.addEventListener('canplay', onSeeked, { once: true });
      }
      return;
    }

    if (!isPlaying || isFutureScheduled) {
      if (!el.paused) {
        el.pause();
        el._isPlayPromisePending = false;
      }
      if (Math.abs(el.currentTime - targetTime) > 0.02) {
        try { el.currentTime = targetTime; } catch (_) {}
      }
      el.playbackRate = 1.0;

      if (isFutureScheduled) {
        this.armScheduledStart(el, this.masterState.startAtEpochMs);
      }
      return;
    }

    if (el.paused && !el._isPlayPromisePending) {
      el._isPlayPromisePending = true;
      const playPromise = el.play();
      if (playPromise !== undefined) {
        playPromise
          .then(() => { el._isPlayPromisePending = false; })
          .catch(() => {
            el._isPlayPromisePending = false;
            if (!el.muted) {
              el.muted = true;
              el.play().catch(() => {});
            }
          });
      }
    }

    const delta = el.currentTime - targetTime;
    const absDelta = Math.abs(delta);

    if (forceHardAlign || absDelta > 0.35) {
      try { el.currentTime = targetTime; } catch (_) {}
      el.playbackRate = 1.0;
    } else if (absDelta > 0.015) {
      if (delta > 0) {
        el.playbackRate = Math.max(0.98, 1.0 - (delta * 0.4));
      } else {
        el.playbackRate = Math.min(1.02, 1.0 + (-delta * 0.4));
      }
    } else if (absDelta < 0.005) {
      el.playbackRate = 1.0;
    }
  }

  armScheduledStart(el: any, targetEpochMs: number) {
    if (this._scheduledTargetMs === targetEpochMs && (this.scheduledStartRaf || this._scheduledTimeout)) {
      return;
    }
    this._scheduledTargetMs = targetEpochMs;

    if (this.scheduledStartRaf) cancelAnimationFrame(this.scheduledStartRaf);
    if (this._scheduledTimeout) clearTimeout(this._scheduledTimeout);

    const executePlay = () => {
      if (this.scheduledStartRaf) { cancelAnimationFrame(this.scheduledStartRaf); this.scheduledStartRaf = null; }
      if (this._scheduledTimeout) { clearTimeout(this._scheduledTimeout); this._scheduledTimeout = null; }
      if (el.paused) {
        this._playStartedAt = performance.now();
        el.play().catch(() => {
          el.muted = true;
          el.play().catch(() => {});
        });
      }
    };

    const checkSchedule = () => {
      const now = clock.now();
      if (now >= targetEpochMs) {
        executePlay();
      } else {
        this.scheduledStartRaf = requestAnimationFrame(checkSchedule);
      }
    };

    this.scheduledStartRaf = requestAnimationFrame(checkSchedule);
    const delayMs = Math.max(0, targetEpochMs - clock.now());
    this._scheduledTimeout = setTimeout(executePlay, delayMs);
  }

  syncNow(forceHardAlign: boolean) {
    if (!forceHardAlign && this._playStartedAt && (performance.now() - this._playStartedAt) < 500) {
      return;
    }
    if (this.videoEl && this.videoEl.style.display !== 'none') {
      this.syncElement(this.videoEl, true, forceHardAlign);
    }
    if (this.audioEl && this.audioEl.dataset.src) {
      this.syncElement(this.audioEl, false, forceHardAlign);
    }
  }

  startSyncClock() {
    if (this.syncTimer) clearInterval(this.syncTimer);
    this.syncTimer = setInterval(() => {
      this.syncNow(false);
    }, 250);
  }

  reportBufferReady(targetPts: number, syncVersion: number) {
    if (this._lastReportedReadyVersion === syncVersion) return;
    this._lastReportedReadyVersion = syncVersion;

    if (mediaSyncChannel) {
      mediaSyncChannel.postMessage({
        type: 'MEDIA_BUFFER_READY',
        targetPts: targetPts,
        syncVersion: syncVersion
      });
    }

    if (liveWs && liveWs.readyState === WebSocket.OPEN) {
      liveWs.send(JSON.stringify({
        MediaBufferReady: {
          client_id: 'foh_projection_screen',
          target_pts: targetPts,
          sync_version: syncVersion
        }
      }));
    }
  }
}

function isCameraFeed(bg: string | null | undefined): boolean {
  if (!bg || typeof bg !== 'string') return false;
  const lower = bg.toLowerCase();
  return lower.startsWith('webcam:') || lower.startsWith('camera:') || lower.startsWith('device:') || lower.startsWith('feed:');
}

const lyricsContainerEl = document.getElementById('lyrics-container')!;
const lyricsEl = document.getElementById('lyrics')!;
const lyricsElementsEl = document.getElementById('lyrics-elements') as HTMLElement | null;
const referenceLabelEl = document.getElementById('reference-label') as HTMLElement | null;
const footerTitleEl = document.getElementById('footer-title')!;
const footerAuthorEl = document.getElementById('footer-author')!;
const blackEl = document.getElementById('overlay-black')!;
const logoEl = document.getElementById('overlay-logo')!;
const alertEl = document.getElementById('alert-banner')!;
const screenEl = document.getElementById('screen')!;
const videoBgEl = document.getElementById('screen-video-bg') as HTMLVideoElement;
const imageBgEl = document.getElementById('screen-image-bg')!;
const audioBgEl = document.getElementById('screen-audio-bg') as HTMLAudioElement;
const webStreamEl = document.getElementById('screen-web-stream') as HTMLIFrameElement;
let lastWebStreamUrl: string | null = null;

videoBgEl.muted = true; // Force muted to prevent audio phasing across devices

const timecodeSyncer = new TimecodeSyncEngine(videoBgEl, audioBgEl);

if (videoBgEl) {
  videoBgEl.addEventListener('loadedmetadata', () => timecodeSyncer.syncNow(true));
  videoBgEl.addEventListener('canplay', () => timecodeSyncer.syncNow(false));
}

try {
  mediaSyncChannel = new BroadcastChannel('opensanctuary_media_sync');
  mediaSyncChannel.postMessage({ action: 'output_ready' });
  window.addEventListener('beforeunload', () => {
    mediaSyncChannel?.postMessage({ action: 'output_closed' });
  });
  mediaSyncChannel.onmessage = (event) => {
    if (event.data && event.data.type === 'MEDIA_SYNC') {
      const d = event.data;
      if (d.action === 'close') {
        fadeToBlack(0.5);
        timecodeSyncer.setMasterState(null, false);
        setTimeout(() => {
          if (videoBgEl) { videoBgEl.pause(); videoBgEl.currentTime = 0; }
          if (audioBgEl) { audioBgEl.pause(); audioBgEl.currentTime = 0; }
        }, 500);
        return;
      }
      if (d.action === 'stop') {
        timecodeSyncer.setMasterState(null, false);
        if (videoBgEl) { videoBgEl.pause(); videoBgEl.currentTime = 0; }
        if (audioBgEl) { audioBgEl.pause(); audioBgEl.currentTime = 0; }
        return;
      }
      timecodeSyncer.setMasterState({
        is_playing: d.isPlaying,
        current_time: d.currentTime,
        timestamp_ms: d.timestamp || Date.now(),
        start_at_epoch_ms: d.start_at_epoch_ms || null,
        isLooping: d.isLooping,
        is_muted: videoBgEl ? videoBgEl.muted : false,
        volume: 1.0
      }, !videoBgEl.loop);
    }
  };
} catch (e) {
  console.warn('BroadcastChannel not available in live display:', e);
}

window.addEventListener('click', () => {
  if (videoBgEl && videoBgEl.muted && !videoBgEl.loop) {
    videoBgEl.muted = false;
    if (videoBgEl.paused) videoBgEl.play().catch(() => {});
  }
  if (audioBgEl && audioBgEl.muted) {
    audioBgEl.muted = false;
    if (audioBgEl.paused) audioBgEl.play().catch(() => {});
  }
});
window.addEventListener('keydown', () => {
  if (videoBgEl && videoBgEl.muted && !videoBgEl.loop) {
    videoBgEl.muted = false;
    if (videoBgEl.paused) videoBgEl.play().catch(() => {});
  }
  if (audioBgEl && audioBgEl.muted) {
    audioBgEl.muted = false;
    if (audioBgEl.paused) audioBgEl.play().catch(() => {});
  }
});

// Thin wrapper around the canonical autoFitLyrics (core/autofit.ts) bound to
// this page's fixed lyrics container/target — this used to be its own,
// separately-maintained copy of the same binary-search algorithm, which had
// drifted out of sync with the canonical one (missing a width-measurement
// fix for wrapped text; see that file's autoFitLyrics doc comment).
function autoFitLyrics() {
  autoFitLyricsCore(lyricsContainerEl, lyricsEl as HTMLElement);
}

window.addEventListener('resize', autoFitLyrics);

let isDisconnectBlackout = false;
let isStateBlackout = false;

function fadeToBlack(durationSec = 0.4) {
  if (!blackEl) return;
  blackEl.style.display = 'block';
  blackEl.style.pointerEvents = 'auto';
  blackEl.style.transition = `opacity ${durationSec}s ease-in-out`;
  requestAnimationFrame(() => {
    blackEl.style.opacity = '1';
  });
}

function fadeInFromBlack(durationSec = 0.4) {
  if (!blackEl) return;
  blackEl.style.transition = `opacity ${durationSec}s ease-in-out`;
  blackEl.style.opacity = '0';
  blackEl.style.pointerEvents = 'none';
  setTimeout(() => {
    if (!isDisconnectBlackout && !isStateBlackout) {
      blackEl.style.display = 'none';
    }
  }, durationSec * 1000 + 40);
}

function renderSnapshot(snapshot: any) {
  try {
    const state = snapshot.state;
    if (!state) return;

    isDisconnectBlackout = false;
    isStateBlackout = !!state.is_blackout;

    if (isStateBlackout) {
      fadeToBlack(0.4);
    } else {
      fadeInFromBlack(0.4);
    }

    logoEl.style.display = state.is_logo_override ? 'flex' : 'none';

    if (state.alert_message) {
      alertEl.textContent = state.alert_message;
      alertEl.style.display = 'block';
    } else {
      alertEl.style.display = 'none';
    }

    if (state.web_stream && state.web_stream.url) {
      const wsMode = state.web_stream.mode || 'background';
      webStreamEl.className = `ws-mode-${wsMode}`;
      webStreamEl.style.display = 'block';
      if (lastWebStreamUrl !== state.web_stream.url) {
        lastWebStreamUrl = state.web_stream.url;
        webStreamEl.src = state.web_stream.url;
      }
    } else if (lastWebStreamUrl !== null) {
      lastWebStreamUrl = null;
      webStreamEl.style.display = 'none';
      webStreamEl.src = 'about:blank';
    }

    const resolvedTheme = resolveThemeAt(state.live_item, availableThemes, state.global_theme);
    applyThemeTypography(lyricsEl as HTMLElement, resolvedTheme);
    const isScriptureTheme = !!(resolvedTheme && resolvedTheme.category === 'scripture');
    const referencePosition = (resolvedTheme && resolvedTheme.referencePosition) || 'inline';

    let currentBg = 'linear-gradient(135deg, #0f2027, #203a43, #2c5364)';
    let slide: any = null;
    if (referenceLabelEl) referenceLabelEl.style.display = 'none';
    if (state.live_item && state.live_item.slides && state.live_item.slides.length > 0) {
      slide = resolveSlideAt(state.live_item, state.live_slide_index) || state.live_item.slides[0];
      const hasPositionedElements = !!(slide.elements && slide.elements.length > 0);
      const showPinnedReference = isScriptureTheme && !!slide.reference_label && (referencePosition === 'top' || referencePosition === 'bottom');
      const inlinePrefix = (isScriptureTheme && slide.reference_label && referencePosition === 'inline') ? `${slide.reference_label}  ` : '';

      if (referenceLabelEl && showPinnedReference) {
        referenceLabelEl.textContent = slide.reference_label;
        referenceLabelEl.style.display = 'block';
        if (referencePosition === 'top') {
          referenceLabelEl.style.top = '3vh';
          referenceLabelEl.style.bottom = '';
          referenceLabelEl.style.fontSize = '2.2vw';
        } else {
          referenceLabelEl.style.bottom = '1.4vh';
          referenceLabelEl.style.top = '';
          referenceLabelEl.style.fontSize = '2.2vw';
        }
      }

      if (state.is_clear_text) {
        lyricsEl.innerHTML = '';
        if (lyricsElementsEl) lyricsElementsEl.style.display = 'none';
      } else if (hasPositionedElements && lyricsElementsEl) {
        // Render the slide's real positioned elements (text runs, images,
        // shapes, lines, tables) instead of the flattened single-textbox
        // fallback below — this is what makes the canvas editor's free-form
        // layouts actually show up on the projector output.
        lyricsEl.innerHTML = '';
        lyricsElementsEl.style.display = 'block';
        const projection = computeCanvasProjection(screenEl.clientWidth, screenEl.clientHeight, { authoredAspect: 16 / 9 });
        renderSlideElements(lyricsElementsEl, slide.elements, projection);
      } else if (slide.text && parseParallelSlide(slide.text).isParallel) {
        if (lyricsElementsEl) lyricsElementsEl.style.display = 'none';
        const parsed = parseParallelSlide(slide.text);
        lyricsEl.innerHTML = `
          <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 3.5vw; width: 100%; text-align: left; align-items: start;">
            <div style="background: rgba(0,0,0,0.38); padding: 1.8vw 2.2vw; border-radius: 0.8vw; border-left: 0.4vw solid #ffa726;">
              <div style="font-size: 0.8em; line-height: 1.4; white-space: pre-line;">${escapeHtml(parsed.leftText)}</div>
            </div>
            <div style="background: rgba(0,0,0,0.38); padding: 1.8vw 2.2vw; border-radius: 0.8vw; border-left: 0.4vw solid #00e5ff;">
              <div style="font-size: 0.8em; line-height: 1.4; white-space: pre-line;">${escapeHtml(parsed.rightText)}</div>
            </div>
          </div>
        `;
      } else {
        if (lyricsElementsEl) lyricsElementsEl.style.display = 'none';
        lyricsEl.textContent = inlinePrefix ? `${inlinePrefix}${slide.text}` : slide.text;
      }
      footerTitleEl.textContent = state.is_clear_text ? '' : state.live_item.title;
      footerAuthorEl.textContent = state.is_clear_text ? '' : (slide.footer || '');

      if (!hasPositionedElements) {
        try { autoFitLyrics(); } catch (e) { console.warn('autoFit error:', e); }
      }
      currentBg = resolveSlideBackground(slide, state.live_item, availableThemes, null, state.global_theme);
    } else {
      lyricsEl.textContent = '';
      if (lyricsElementsEl) { lyricsElementsEl.style.display = 'none'; lyricsElementsEl.innerHTML = ''; }
      footerTitleEl.textContent = '';
      footerAuthorEl.textContent = '';
      currentBg = resolveSlideBackground(null, null, availableThemes, null, state.global_theme);
    }

    // Lower-third / streaming: a chroma-key theme forces a flat keyable color
    // regardless of any image/gradient/video background otherwise configured, and
    // a safe-area percent constrains the lyrics band to the bottom N% of the screen
    // instead of full-bleed (see src/core/models.rs Theme::chroma_key_enabled/
    // safe_area_percent).
    if (resolvedTheme && resolvedTheme.chromaKeyEnabled && resolvedTheme.chromaKeyColor) {
      currentBg = resolvedTheme.chromaKeyColor;
    }
    const safeAreaPercent = resolvedTheme ? (resolvedTheme.safeAreaPercent || 0) : 0;
    if (safeAreaPercent > 0) {
      lyricsContainerEl.style.flex = 'none';
      lyricsContainerEl.style.height = `${safeAreaPercent}%`;
      lyricsContainerEl.style.marginTop = 'auto';
    } else {
      lyricsContainerEl.style.flex = '';
      lyricsContainerEl.style.height = '';
      lyricsContainerEl.style.marginTop = '';
    }

    const isDedicatedMedia = state.live_item && state.live_item.item_type === 'Media';

    if (isVideoBackground(currentBg)) {
      if (videoBgEl) {
        const isMuted = !isDedicatedMedia || (state.media_playback ? (state.media_playback.is_muted || false) : false);
        videoBgEl.muted = isMuted;
        videoBgEl.loop = !isDedicatedMedia || (state.media_playback ? (state.media_playback.is_looping || false) : false);
        videoBgEl.style.objectFit = isDedicatedMedia ? 'contain' : 'cover';
        if (state.media_playback && state.media_playback.volume !== undefined) {
          videoBgEl.volume = state.media_playback.volume;
        }

        const encodedUrl = encodeURI(currentBg);
        if (videoBgEl.dataset.src !== currentBg) {
          videoBgEl.dataset.src = currentBg;
          videoBgEl.preload = 'auto';
          videoBgEl.src = encodedUrl;
          videoBgEl.load();
        }
        videoBgEl.style.display = 'block';

        if (state.media_playback) {
          timecodeSyncer.setMasterState(state.media_playback, isDedicatedMedia);
        } else if (!isDedicatedMedia && videoBgEl.paused) {
          videoBgEl.play().catch(() => {});
        }
      }
      imageBgEl.style.display = 'none';
      audioBgEl.pause();
      audioBgEl.dataset.src = '';
      screenEl.style.animation = '';
      screenEl.style.background = '#000';
    } else if (isCameraFeed(currentBg)) {
      timecodeSyncer.setMasterState(null, false);
      imageBgEl.style.display = 'none';
      audioBgEl.pause();
      audioBgEl.dataset.src = '';
      screenEl.style.animation = '';
      screenEl.style.background = '#000';
      if (videoBgEl) {
        videoBgEl.dataset.src = '';
        if (!videoBgEl.srcObject && navigator.mediaDevices && navigator.mediaDevices.getUserMedia) {
          navigator.mediaDevices.getUserMedia({ video: true, audio: false })
            .then(stream => {
              videoBgEl.srcObject = stream;
              videoBgEl.play().catch(() => {});
              videoBgEl.style.display = 'block';
            })
            .catch(err => {
              console.warn('Could not access live camera feed:', err);
              videoBgEl.style.display = 'none';
            });
        } else if (videoBgEl.srcObject) {
          videoBgEl.style.display = 'block';
          videoBgEl.play().catch(() => {});
        }
      }
    } else if (extractImageUrl(currentBg)) {
      const liveImgUrl = extractImageUrl(currentBg)!;
      timecodeSyncer.setMasterState(null, false);
      if (videoBgEl) {
        if (videoBgEl.srcObject) {
          try { (videoBgEl.srcObject as MediaStream).getTracks().forEach(t => t.stop()); } catch (e) {}
          videoBgEl.srcObject = null;
        }
        videoBgEl.style.display = 'none';
        videoBgEl.pause();
      }
      audioBgEl.pause();
      audioBgEl.dataset.src = '';
      imageBgEl.style.backgroundImage = `url("${encodeURI(liveImgUrl)}")`;
      imageBgEl.style.backgroundSize = 'cover';
      imageBgEl.style.backgroundPosition = 'center';
      imageBgEl.style.backgroundRepeat = 'no-repeat';
      imageBgEl.style.display = 'block';
      screenEl.style.animation = '';
      screenEl.style.background = '#000';
    } else if (isAudioMedia(currentBg)) {
      if (videoBgEl) {
        if (videoBgEl.srcObject) {
          try { (videoBgEl.srcObject as MediaStream).getTracks().forEach(t => t.stop()); } catch (e) {}
          videoBgEl.srcObject = null;
        }
        videoBgEl.style.display = 'none';
        videoBgEl.pause();
      }
      imageBgEl.style.display = 'none';
      screenEl.style.animation = '';
      screenEl.style.background = 'linear-gradient(135deg, #102027, #37474f)';
      if (audioBgEl) {
        const encodedAudio = encodeURI(currentBg);
        if (audioBgEl.dataset.src !== currentBg) {
          audioBgEl.dataset.src = currentBg;
          audioBgEl.src = encodedAudio;
          audioBgEl.play().catch(() => {});
        }
        if (state.media_playback) {
          if (state.media_playback.is_playing) {
            if (audioBgEl.paused) audioBgEl.play().catch(() => {});
            if (Math.abs(audioBgEl.currentTime - state.media_playback.current_time) > 0.4) {
              audioBgEl.currentTime = state.media_playback.current_time;
            }
          } else {
            if (!audioBgEl.paused) audioBgEl.pause();
          }
        }
      }
    } else {
      if (videoBgEl) {
        if (videoBgEl.srcObject) {
          try { (videoBgEl.srcObject as MediaStream).getTracks().forEach(t => t.stop()); } catch (e) {}
          videoBgEl.srcObject = null;
        }
        videoBgEl.style.display = 'none';
        videoBgEl.pause();
      }
      imageBgEl.style.display = 'none';
      audioBgEl.pause();
      audioBgEl.dataset.src = '';
      applyResolvedBackground(screenEl, currentBg);
    }
  } catch (e) {
    console.error(e);
  }
}

function connect() {
  if (liveWsClient) return;

  liveWsClient = createEngineWebSocket({
    onOpen: () => {
      liveWs = liveWsClient?.getRawSocket() || null;
      isDisconnectBlackout = false;
      if (!isStateBlackout) {
        fadeInFromBlack(0.3);
      }
    },
    onClose: () => {
      liveWs = liveWsClient?.getRawSocket() || null;
      isDisconnectBlackout = true;
      fadeToBlack(0.5);
      timecodeSyncer.setMasterState(null, false);
      setTimeout(() => {
        if (videoBgEl && !videoBgEl.paused) videoBgEl.pause();
        if (audioBgEl && !audioBgEl.paused) audioBgEl.pause();
      }, 500);
    },
    onSnapshot: (snapshot) => {
      renderSnapshot(snapshot);
    },
  });
  liveWs = liveWsClient.getRawSocket();
}

if (videoBgEl) {
  videoBgEl.addEventListener('ended', () => {
    if (!videoBgEl.loop) {
      if (blackEl) {
        blackEl.style.transition = 'opacity 0.8s ease-in-out';
        blackEl.style.opacity = '1';
        blackEl.style.display = 'block';
      }
    }
  });
}

// Applies the Alerts & Nursery settings (alertPosition/alertFontSize, configured in
// the operator console's Settings dialog) to the alert banner's CSS custom properties.
// Fetched once at load — these change rarely enough that a live-reactive update isn't
// worth the complexity a settings-aware WS payload would add.
const ALERT_FONT_SIZE_VW: Record<string, string> = {
  '28': '2.2vw',
  '36': '2.8vw',
  '48': '3.6vw',
};

async function applyDisplaySettings() {
  try {
    const res = await fetch('/api/settings');
    if (!res.ok) return;
    const settings = await res.json();
    const root = document.documentElement.style;
    if (settings.alertPosition === 'top') {
      root.setProperty('--alert-top', '0');
      root.setProperty('--alert-bottom', 'auto');
    } else {
      root.setProperty('--alert-top', 'auto');
      root.setProperty('--alert-bottom', '0');
    }
    const fontSize = ALERT_FONT_SIZE_VW[settings.alertFontSize];
    if (fontSize) root.setProperty('--alert-font-size', fontSize);
  } catch (e) {
    // Defaults in the CSS (top:auto/bottom:0, 2.8vw) already match the previous
    // hardcoded appearance, so a failed fetch here is a silent no-op, not a bug.
  }
}

applyDisplaySettings();
connect();
