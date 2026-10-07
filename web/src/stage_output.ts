/**
 * OpenSanctuary Stage Foldback Confidence Display Controller
 * Bundled entrypoint for stage.html.
 */

import { resolveSlideAt, escapeHtml } from './core/presentation_helpers.ts';
import { autoFitLyrics } from './core/autofit.ts';
import { clock } from './core/timesync.ts';
import { createEngineWebSocket } from './core/ws_client.ts';
import { resolveNextPresentationPreview, parseBilingualSlideText } from './core/presentation_sequence.ts';

// Synchronize NTP clock with server
clock.synchronize();

// Live Stage Clock
function updateClock() {
  const clockEl = document.getElementById('clock');
  if (clockEl) {
    const now = new Date();
    clockEl.textContent = now.toLocaleTimeString([], {
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    });
  }
}
setInterval(updateClock, 500);
updateClock();

// DOM Element References
const currEl = document.getElementById('current-slide');
const nextEl = document.getElementById('next-slide');
const notesEl = document.getElementById('presenter-notes');
const currLabelEl = document.getElementById('current-slide-label');
const nextLabelEl = document.getElementById('next-slide-label');
const infoEl = document.getElementById('song-info');

const badgeLive = document.getElementById('badge-foh-live');
const badgeBlack = document.getElementById('badge-foh-black');
const badgeClear = document.getElementById('badge-foh-clear');
const badgeLogo = document.getElementById('badge-foh-logo');
const badgeMedia = document.getElementById('badge-foh-media');

export function renderStageSnapshot(snapshot: any) {
  if (!snapshot) return;
  const state = snapshot.state || {};
  const schedule = snapshot.schedule || { items: [] };

  // Update FOH Status Badges
  if (badgeBlack) badgeBlack.style.display = state.is_blackout ? 'inline-block' : 'none';
  if (badgeClear) badgeClear.style.display = (!state.is_blackout && state.is_clear_text) ? 'inline-block' : 'none';
  if (badgeLogo) badgeLogo.style.display = (!state.is_blackout && state.is_logo_override) ? 'inline-block' : 'none';
  if (badgeLive) badgeLive.style.display = (!state.is_blackout && !state.is_clear_text && !state.is_logo_override && state.live_item) ? 'inline-block' : 'none';
  if (badgeMedia) badgeMedia.style.display = (state.media_playback && state.media_playback.is_playing) ? 'inline-block' : 'none';

  if (state.live_item && state.live_item.slides && state.live_item.slides.length > 0) {
    if (infoEl) infoEl.textContent = state.live_item.title;
    const curr = resolveSlideAt(state.live_item, state.live_slide_index);
    const isMedia = state.live_item.item_type === 'media' || (curr && curr.background && (curr.background.endsWith('.mp4') || curr.background.endsWith('.webm') || curr.background.includes('/media/')));
    const bilingualCurr = parseBilingualSlideText(curr?.text);

    if (currLabelEl) currLabelEl.textContent = curr ? (curr.label || `Slide ${state.live_slide_index + 1}`) : '';
    if (currEl) {
      if (isMedia && (!curr || !curr.text)) {
        currEl.textContent = `🎬 [Media Playback: ${state.live_item.title}]`;
      } else if (bilingualCurr.isBilingual) {
        currEl.innerHTML = `<div style="display:grid; grid-template-columns:1fr 1fr; gap:16px; text-align:left;">
          <div style="background:rgba(255,255,255,0.06); padding:10px; border-radius:6px; border-left:3px solid #ff5722; white-space:pre-line;">${escapeHtml(bilingualCurr.primary)}</div>
          <div style="background:rgba(255,255,255,0.06); padding:10px; border-radius:6px; border-left:3px solid #00e5ff; white-space:pre-line;">${escapeHtml(bilingualCurr.secondary)}</div>
        </div>`;
      } else {
        currEl.textContent = curr ? curr.text : '';
      }

      // Auto-fit current slide text to fill card -- same binary-search
      // algorithm as core/autofit.ts's autoFitLyrics (shared with the live
      // presentation screen and slide editor canvas), just reused here
      // instead of hand-rolled a second time. This card reserves a fixed
      // pixel margin for its own chrome (badges/label) rather than a
      // percentage of the container, so widthFactor/heightFactor are
      // computed per-call as the ratio that reproduces that same maxW/maxH
      // -- not a fixed constant like other callers use.
      if (currEl.parentElement) {
        const p = currEl.parentElement;
        const maxH = p.clientHeight - 80;
        const maxW = p.clientWidth - 40;
        if (maxH > 0 && maxW > 0 && p.clientWidth > 0 && p.clientHeight > 0) {
          autoFitLyrics(p, currEl, {
            minFontSize: 16,
            maxFontSize: Math.min(maxH * 0.35, maxW * 0.1, 75),
            maxSteps: 8,
            widthFactor: maxW / p.clientWidth,
            heightFactor: maxH / p.clientHeight,
            lineHeightScale: false,
          });
        }
      }
    }

    if (notesEl) {
      if (state.alert_message) {
        notesEl.innerHTML = `<span style="color: #FF5252; font-weight: bold;">🚨 STAGE ALERT:</span> ${escapeHtml(state.alert_message)}`;
      } else if (curr && (curr.footer || curr.notes)) {
        const noteContent = (curr.notes || curr.footer || '').trim();
        notesEl.textContent = noteContent;
      } else if (state.live_item && state.live_item.subtitle) {
        notesEl.textContent = `Reference: ${state.live_item.subtitle}`;
      } else {
        notesEl.textContent = 'No presenter notes for this slide.';
      }
    }

    const nextPreview = resolveNextPresentationPreview(schedule, state.live_item, state.live_slide_index);
    if (nextLabelEl) nextLabelEl.textContent = nextPreview.label;
    if (nextEl) {
      if (nextPreview.isNextSlide && nextPreview.bilingual.isBilingual) {
        nextEl.innerHTML = `<div style="display:grid; grid-template-columns:1fr 1fr; gap:12px; text-align:left; font-size:0.85em;">
          <div style="white-space:pre-line; opacity:0.8;">${escapeHtml(nextPreview.bilingual.primary)}</div>
          <div style="white-space:pre-line; opacity:0.8;">${escapeHtml(nextPreview.bilingual.secondary)}</div>
        </div>`;
      } else {
        nextEl.textContent = nextPreview.text;
      }
    }
  } else {
    if (infoEl) infoEl.textContent = 'Standby';
    if (currEl) currEl.textContent = 'No Live Item';
    if (currLabelEl) currLabelEl.textContent = '';
    if (nextLabelEl) nextLabelEl.textContent = '';
    if (notesEl) notesEl.textContent = 'No active presentation.';
    if (nextEl) nextEl.textContent = '';
  }
}

// Connect Unified Engine WebSocket
export const stageWsClient = createEngineWebSocket({
  onSnapshot: (snapshot) => {
    try {
      renderStageSnapshot(snapshot);
    } catch (e) {
      console.error('[StageOutput] Render error:', e);
    }
  },
});
