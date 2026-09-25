/**
 * OpenSanctuary / OS-Next Presentation Auto-Advance Controller
 * Extracted from app_core.ts (ongoing modularization pass) — the Play/Pause/Loop
 * slideshow controls for a live "presentation" schedule item, advancing on a
 * per-slide timer (falling back to a default duration when a slide has none of
 * its own).
 */
import { effectiveArrangement } from '../core/presentation_helpers';

export interface PresentationPlaybackContext {
  getCurrentSnapshot(): any;
  sendCommand(cmd: any): void;
}

let ctx: PresentationPlaybackContext | null = null;

const DEFAULT_SLIDESHOW_SECONDS_PER_SLIDE = 10;
let presentationPlaybackTimer: any = null;
let presentationPlaybackItemId: string | null = null;

export function getPresentationPlaybackItemId(): string | null {
  return presentationPlaybackItemId;
}

export function getEffectiveSlideDuration(item: any, slideIdx: number): number {
  const slide = item && item.slides ? item.slides[slideIdx] : null;
  const perSlide = slide ? slide.duration_seconds : null;
  const perItem = item ? item.default_slide_duration_seconds : null;
  return (perSlide != null ? perSlide : (perItem != null ? perItem : DEFAULT_SLIDESHOW_SECONDS_PER_SLIDE));
}

export function isPresentationPlaying(): boolean {
  return presentationPlaybackTimer !== null;
}

export function stopPresentationPlayback() {
  if (presentationPlaybackTimer) {
    clearTimeout(presentationPlaybackTimer);
    presentationPlaybackTimer = null;
  }
  presentationPlaybackItemId = null;
  updatePresentationControlsUI();
}

export function startPresentationPlayback() {
  const state = ctx!.getCurrentSnapshot() && ctx!.getCurrentSnapshot().state;
  const liveItem = state && state.live_item;
  if (!liveItem || liveItem.item_type !== 'presentation' || !liveItem.slides || liveItem.slides.length === 0) return;
  presentationPlaybackItemId = liveItem.id;
  // schedulePresentationTick() must run first — it's what actually arms
  // presentationPlaybackTimer, which isPresentationPlaying() (and therefore
  // the button label) reads. Updating the UI before that would show "Play"
  // for the entire first slide's duration instead of flipping immediately.
  schedulePresentationTick();
  updatePresentationControlsUI();
}

function schedulePresentationTick() {
  if (presentationPlaybackTimer) clearTimeout(presentationPlaybackTimer);
  const state = ctx!.getCurrentSnapshot() && ctx!.getCurrentSnapshot().state;
  const liveItem = state && state.live_item;
  if (!liveItem || liveItem.id !== presentationPlaybackItemId) {
    // The live item changed out from under a running slideshow — stop rather
    // than keep advancing whatever is live now.
    stopPresentationPlayback();
    return;
  }
  const slideIdx = state.live_slide_index || 0;
  const durationMs = Math.max(0.5, getEffectiveSlideDuration(liveItem, slideIdx)) * 1000;
  presentationPlaybackTimer = setTimeout(() => advancePresentationSlide(), durationMs);
}

function advancePresentationSlide() {
  const state = ctx!.getCurrentSnapshot() && ctx!.getCurrentSnapshot().state;
  const liveItem = state && state.live_item;
  if (!liveItem || liveItem.id !== presentationPlaybackItemId) {
    stopPresentationPlayback();
    return;
  }
  const slideCount = effectiveArrangement(liveItem).length;
  const curIdx = state.live_slide_index || 0;
  const nextIdx = curIdx + 1;
  if (nextIdx < slideCount) {
    ctx!.sendCommand({ JumpSlide: nextIdx });
    schedulePresentationTick();
  } else if (liveItem.slideshow_loop) {
    ctx!.sendCommand({ JumpSlide: 0 });
    schedulePresentationTick();
  } else {
    stopPresentationPlayback();
  }
}

export function updatePresentationControlsUI() {
  const playBtn = document.getElementById('btn-presentation-play');
  const loopBtn = document.getElementById('btn-presentation-loop');
  const durationInput = document.getElementById('presentation-default-duration') as HTMLInputElement | null;
  const snapshot = ctx!.getCurrentSnapshot();
  const liveItem = snapshot && snapshot.state && snapshot.state.live_item;
  if (playBtn) playBtn.textContent = isPresentationPlaying() ? '⏸ Pause' : '▶ Play';
  if (loopBtn && liveItem) loopBtn.textContent = liveItem.slideshow_loop ? '🔁 Loop: ON' : '🔁 Loop: OFF';
  if (durationInput && liveItem && document.activeElement !== durationInput) {
    durationInput.value = String(liveItem.default_slide_duration_seconds != null ? liveItem.default_slide_duration_seconds : DEFAULT_SLIDESHOW_SECONDS_PER_SLIDE);
  }
}

export function findLiveItemIndex(): number {
  const snapshot = ctx!.getCurrentSnapshot();
  const sched = snapshot && snapshot.schedule;
  const liveItem = snapshot && snapshot.state && snapshot.state.live_item;
  if (!sched || !sched.items || !liveItem) return -1;
  return sched.items.findIndex((it: any) => it.id === liveItem.id);
}

export function initPresentationPlayback(context: PresentationPlaybackContext) {
  ctx = context;

  document.getElementById('btn-presentation-play')?.addEventListener('click', () => {
    if (isPresentationPlaying()) {
      stopPresentationPlayback();
    } else {
      startPresentationPlayback();
    }
  });
  document.getElementById('btn-presentation-loop')?.addEventListener('click', () => {
    const itemIdx = findLiveItemIndex();
    const snapshot = ctx!.getCurrentSnapshot();
    const liveItem = snapshot && snapshot.state && snapshot.state.live_item;
    if (itemIdx < 0 || !liveItem) return;
    ctx!.sendCommand({ SetPresentationLoop: { item_index: itemIdx, loop_enabled: !liveItem.slideshow_loop } });
  });
  document.getElementById('presentation-default-duration')?.addEventListener('change', () => {
    const itemIdx = findLiveItemIndex();
    if (itemIdx < 0) return;
    const input = document.getElementById('presentation-default-duration') as HTMLInputElement | null;
    if (!input) return;
    const seconds = parseFloat(input.value);
    if (!isNaN(seconds) && seconds > 0) {
      ctx!.sendCommand({ SetPresentationDefaultDuration: { item_index: itemIdx, duration_seconds: seconds } });
    }
  });
}
