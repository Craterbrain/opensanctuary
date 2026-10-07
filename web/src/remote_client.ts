/**
 * OpenSanctuary Mobile Remote Client (EasyWorship Style)
 * Bundled entrypoint for remote.html.
 * Handles real-time WebSocket communication, touch navigation, and mode toggles.
 */

import { resolveSlideAt, escapeHtml } from './core/presentation_helpers.ts';
import { createEngineWebSocket, type EngineWebSocketClient } from './core/ws_client.ts';
import { resolveNextPresentationPreview } from './core/presentation_sequence.ts';

// --- Pairing (real device token, scanned via QR in the console's Pairing menu) ---
//
// /remote requires a paired device token: minted server-side by
// /api/pairing/remote-session when the console shows the "Mobile Remote" QR,
// carried here in the URL fragment (#token=...) so it isn't sent to the
// server as part of the page request, and verified against the same
// paired_devices table TV/Roku pairing uses (see src/api/routes.rs). No
// token, or a revoked/unknown one, and this page never connects or renders
// show data — it shows a "scan to connect" screen instead.
const REMOTE_TOKEN_STORAGE_KEY = 'os_remote_device_token';

function readTokenFromLocation(): string | null {
  if (typeof location === 'undefined') return null;
  const hash = location.hash || '';
  if (hash.length > 1) {
    const params = new URLSearchParams(hash.slice(1));
    const fromHash = params.get('token');
    if (fromHash) return fromHash;
  }
  return null;
}

async function verifyDeviceToken(token: string): Promise<boolean> {
  try {
    const res = await fetch('/api/pairing/verify', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ token }),
    });
    if (!res.ok) return false;
    const data = await res.json();
    return !!(data && data.valid);
  } catch {
    return false;
  }
}

function showPairingGate() {
  if (!doc) return;
  const gate = doc.getElementById('pairing-gate');
  const app = doc.getElementById('remote-app');
  if (gate) gate.style.display = 'flex';
  if (app) app.style.display = 'none';
}

function hidePairingGate() {
  if (!doc) return;
  const gate = doc.getElementById('pairing-gate');
  const app = doc.getElementById('remote-app');
  if (gate) gate.style.display = 'none';
  if (app) app.style.display = 'flex';
}

// --- State Management ---
export let latestSnapshot: any = null;
export let isFullControl: boolean = true; // Control vs Prompter mode
export let activeTab: string = 'view-schedule';
export let wakeLockSentinel: any = null;
export let viewedItemId: string | null = null;
export let prevLiveItemId: string | null = null;

// --- DOM Elements ---
const doc = typeof document !== 'undefined' ? document : null;

const statusDot = doc?.getElementById('status-dot');
const connText = doc?.getElementById('conn-text');
const modeBtn = doc?.getElementById('mode-toggle-btn');
const modeIcon = doc?.getElementById('mode-icon');
const modeLabel = doc?.getElementById('mode-label');

const btnLogo = doc?.getElementById('btn-override-logo');
const btnBlack = doc?.getElementById('btn-override-black');
const btnClear = doc?.getElementById('btn-override-clear');

const alertBanner = doc?.getElementById('alert-banner');
const scheduleListEl = doc?.getElementById('schedule-list');
const scheduleEmptyEl = doc?.getElementById('schedule-empty');

const slidesBannerTitle = doc?.getElementById('slides-banner-title');
const slidesBannerAuthor = doc?.getElementById('slides-banner-author');
const btnBannerLive = doc?.getElementById('btn-banner-live');
const miniPreviewCard = doc?.getElementById('mini-preview-card');
const miniPreviewText = doc?.getElementById('mini-preview-text');
const miniPreviewTag = doc?.getElementById('mini-preview-tag');
const slideCardsGrid = doc?.getElementById('slide-cards-grid');

const prompterClock = doc?.getElementById('prompter-clock');
const prompterLyrics = doc?.getElementById('prompter-lyrics');
const prompterNextText = doc?.getElementById('prompter-next-text');
const btnWakeLock = doc?.getElementById('btn-wake-lock');

const btnPrevItem = doc?.getElementById('btn-prev-item');
const btnPrevSlide = doc?.getElementById('btn-prev-slide');
const btnNextSlide = doc?.getElementById('btn-next-slide');
const btnNextItem = doc?.getElementById('btn-next-item');

// --- Tab Switching ---
export function switchTab(viewId: string) {
  activeTab = viewId;
  if (!doc) return;
  doc.querySelectorAll('.tab-item').forEach(t => {
    t.classList.toggle('active', t.getAttribute('data-view') === viewId);
  });
  ['view-schedule', 'view-slides', 'view-prompter'].forEach(id => {
    const el = doc.getElementById(id);
    if (el) el.style.display = (id === viewId) ? 'flex' : 'none';
  });
}

function setupTabs() {
  if (!doc) return;
  doc.querySelectorAll('.tab-item').forEach(tab => {
    tab.addEventListener('click', () => {
      const viewId = tab.getAttribute('data-view');
      if (viewId) switchTab(viewId);
    });
  });
}

// --- Mode Toggle (Control vs Prompter) ---
function setupModeToggle() {
  if (!modeBtn) return;
  modeBtn.addEventListener('click', () => {
    isFullControl = !isFullControl;
    if (isFullControl) {
      modeBtn.classList.remove('prompter-mode');
      if (modeIcon) modeIcon.textContent = '🔒';
      if (modeLabel) modeLabel.textContent = 'Control';
      vibrate(20);
    } else {
      modeBtn.classList.add('prompter-mode');
      if (modeIcon) modeIcon.textContent = '👁️';
      if (modeLabel) modeLabel.textContent = 'Prompter';
      switchTab('view-prompter');
      vibrate(30);
    }
  });
}

export function getItemTypeIcon(type: string): string {
  switch ((type || '').toLowerCase()) {
    case 'song': return '🎵';
    case 'scripture': return '📖';
    case 'presentation': return '📽️';
    case 'media': return '🎬';
    case 'header': return '🏷️';
    default: return '📄';
  }
}

export function getTagClass(tag: string): string {
  if (!tag) return '';
  const t = tag.toUpperCase();
  if (t.startsWith('V')) return 'tag-v';
  if (t.startsWith('C')) return 'tag-c';
  if (t.startsWith('B')) return 'tag-b';
  if (t.startsWith('P')) return 'tag-p';
  if (t.startsWith('E')) return 'tag-e';
  if (t.startsWith('I')) return 'tag-i';
  return '';
}

// --- Haptic Feedback Helper ---
export function vibrate(ms = 20) {
  if (typeof navigator !== 'undefined' && 'vibrate' in navigator) {
    try { navigator.vibrate(ms); } catch {}
  }
}

// --- Helper to render background on preview card ---
export function applyPreviewBackground(card: HTMLElement | null, slide: any) {
  if (!card) return;
  if (!slide || !slide.background) {
    card.style.background = '#000000';
    return;
  }
  const bg = String(slide.background).trim();
  if (bg.startsWith('#') || bg.startsWith('rgb') || bg.includes('gradient')) {
    card.style.background = bg;
  } else {
    card.style.background = `url('/media/${encodeURIComponent(bg)}') center/cover no-repeat #000000`;
  }
}

// --- Command Dispatch ---
export async function sendCommand(cmd: any): Promise<boolean | void> {
  if (!isFullControl) {
    // In view-only prompter mode, lock out ALL mutating commands
    return;
  }
  if (!wsClient) return; // Not paired yet — nothing to send through.
  vibrate(25);
  return wsClient.sendCommand(cmd);
}

// --- UI Render from StateSnapshot ---
export function renderSnapshot(snapshot: any) {
  if (!snapshot) return;
  latestSnapshot = snapshot;
  const state = snapshot.state || {};
  const schedule = snapshot.schedule || { items: [] };
  const liveItem = state.live_item;
  const stagedItem = state.staged_item;

  // Track live item changes from server
  if (liveItem && liveItem.id !== prevLiveItemId) {
    prevLiveItemId = liveItem.id;
    // Follow live item unless user explicitly selected a different staged item
    if (!viewedItemId || (stagedItem && viewedItemId !== stagedItem.id)) {
      viewedItemId = liveItem.id;
    }
  }

  // Overrides
  if (btnBlack) btnBlack.classList.toggle('active-black', !!state.is_blackout);
  if (btnClear) btnClear.classList.toggle('active-clear', !state.is_blackout && !!state.is_clear_text);
  if (btnLogo) btnLogo.classList.toggle('active-logo', !state.is_blackout && !!state.is_logo_override);

  // Alert Banner
  if (alertBanner) {
    if (state.alert_message && state.alert_message.trim()) {
      alertBanner.style.display = 'block';
      alertBanner.textContent = `⚠️ ${state.alert_message}`;
    } else {
      alertBanner.style.display = 'none';
    }
  }

  // 1. Render Schedule List
  if (scheduleListEl && scheduleEmptyEl) {
    if (schedule.items && schedule.items.length > 0) {
      scheduleEmptyEl.style.display = 'none';
      scheduleListEl.innerHTML = '';
      schedule.items.forEach((item: any, idx: number) => {
        const card = doc!.createElement('div');
        card.className = 'schedule-card';

        const isLiveItem = liveItem && liveItem.id === item.id;
        const isStagedItem = stagedItem && stagedItem.id === item.id;
        if (isLiveItem) card.classList.add('is-live');
        if (isStagedItem && !isLiveItem) card.classList.add('is-staged');

        const icon = getItemTypeIcon(item.item_type);
        const slidesCount = (item.slides || []).length;

        card.innerHTML = `
          <div class="sched-header-row">
            <div class="sched-title-wrap">
              <span class="item-type-icon">${icon}</span>
              <span class="item-title">${idx + 1}. ${escapeHtml(item.title || 'Untitled')}</span>
            </div>
            <div class="sched-badges">
              ${isLiveItem ? '<span class="live-badge">LIVE</span>' : ''}
              ${isStagedItem && !isLiveItem ? '<span class="staged-badge">STAGED</span>' : ''}
            </div>
          </div>
          <div class="sched-meta-row">
            <span>${escapeHtml(item.author_or_ref || item.item_type || '')}</span>
            <span>${slidesCount} slides</span>
          </div>
        `;

        card.addEventListener('click', () => {
          viewedItemId = item.id;
          if (isLiveItem) {
            switchTab('view-slides');
          } else {
            sendCommand({ StageItem: { item_index: idx, slide_index: 0 } });
            switchTab('view-slides');
          }
        });

        scheduleListEl.appendChild(card);
      });
    } else {
      scheduleEmptyEl.style.display = 'block';
      scheduleListEl.innerHTML = '';
    }
  }

  // Determine which item to show in Slides view
  let currentItem: any = null;
  if (viewedItemId && schedule.items) {
    currentItem = schedule.items.find((it: any) => it.id === viewedItemId);
  }
  if (!currentItem) {
    currentItem = stagedItem || liveItem;
    if (currentItem) viewedItemId = currentItem.id;
  }

  // 2. Render Mini Live Output Preview Card (ALWAYS reflects active FOH live output)
  const livePlayPos = state.live_slide_index ?? 0;
  const liveSlide = liveItem ? resolveSlideAt(liveItem, livePlayPos) : null;

  if (miniPreviewCard && miniPreviewTag && miniPreviewText) {
    if (state.is_blackout) {
      miniPreviewCard.style.background = '#000000';
      miniPreviewCard.style.borderColor = 'var(--blackout-red)';
      miniPreviewTag.textContent = '⬛ BLACKOUT ACTIVE';
      miniPreviewTag.style.color = '#ff5252';
      miniPreviewText.textContent = 'FOH Screen is Blacked Out';
    } else if (state.is_logo_override) {
      miniPreviewCard.style.background = '#000000';
      miniPreviewCard.style.borderColor = 'var(--logo-amber)';
      miniPreviewTag.textContent = '⭐ LOGO ACTIVE';
      miniPreviewTag.style.color = 'var(--logo-amber)';
      miniPreviewText.textContent = 'Church Logo Screen';
    } else if (state.is_clear_text) {
      miniPreviewCard.style.borderColor = 'var(--clear-orange)';
      miniPreviewTag.textContent = '📄 CLEAR TEXT ACTIVE';
      miniPreviewTag.style.color = 'var(--clear-orange)';
      miniPreviewText.textContent = '(Text Cleared - Background Only)';
      applyPreviewBackground(miniPreviewCard, liveSlide);
    } else if (liveSlide) {
      miniPreviewCard.style.borderColor = 'var(--live-green-glow)';
      miniPreviewTag.textContent = liveSlide.label || `SLIDE ${livePlayPos + 1}`;
      miniPreviewTag.style.color = 'var(--accent-cyan)';
      miniPreviewText.textContent = liveSlide.text || `[${liveSlide.label || 'Slide'}]`;
      applyPreviewBackground(miniPreviewCard, liveSlide);
    } else {
      miniPreviewCard.style.background = '#000000';
      miniPreviewCard.style.borderColor = 'var(--border-highlight)';
      miniPreviewTag.textContent = 'LIVE OUTPUT';
      miniPreviewTag.style.color = 'var(--text-muted)';
      miniPreviewText.textContent = 'No active presentation';
    }
  }

  // 3. Render Slides View
  if (slidesBannerTitle && slidesBannerAuthor && btnBannerLive && slideCardsGrid) {
    if (currentItem) {
      slidesBannerTitle.textContent = currentItem.title || 'Untitled';
      slidesBannerAuthor.textContent = currentItem.author_or_ref || currentItem.item_type || '';

      const isCurrentItemLive = liveItem && liveItem.id === currentItem.id;
      if (isCurrentItemLive) {
        btnBannerLive.textContent = '● LIVE';
        btnBannerLive.style.background = 'var(--live-green)';
        btnBannerLive.style.borderColor = 'var(--live-green-glow)';
      } else {
        btnBannerLive.textContent = '⚡ GO LIVE';
        btnBannerLive.style.background = '#FF6F00';
        btnBannerLive.style.borderColor = '#FFA000';
      }

      const activePlayPos = isCurrentItemLive
        ? state.live_slide_index
        : (state.staged_slide_index ?? 0);

      slideCardsGrid.innerHTML = '';
      const effectiveSlides = (currentItem.arrangement && currentItem.arrangement.length > 0)
        ? currentItem.arrangement.map((_: any, playPos: number) => resolveSlideAt(currentItem, playPos))
        : (currentItem.slides || []).map((_: any, i: number) => resolveSlideAt(currentItem, i));

      effectiveSlides.forEach((slide: any, idx: number) => {
        if (!slide) return;
        const sc = doc!.createElement('div');
        sc.className = 'slide-card';
        const isCardActive = isCurrentItemLive && (idx === activePlayPos);
        if (isCardActive) {
          sc.classList.add('is-active');
        }

        const tagClass = getTagClass(slide.label || slide.tag);

        sc.innerHTML = `
          <div class="slide-card-top">
            <span class="slide-idx-badge">#${idx + 1}</span>
            <span class="slide-tag-pill ${tagClass}">${escapeHtml(slide.label || slide.tag || `S${idx + 1}`)}</span>
          </div>
          <div class="slide-text-preview">${escapeHtml(slide.text || '')}</div>
        `;

        sc.addEventListener('click', () => {
          const itemIdx = schedule.items.findIndex((it: any) => it.id === currentItem.id);
          const targetIdx = itemIdx !== -1 ? itemIdx : null;
          sendCommand({ GoLive: { item_index: targetIdx, slide_index: idx } });
        });

        slideCardsGrid.appendChild(sc);

        if (isCardActive) {
          try {
            sc.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
          } catch (_) {}
        }
      });
    } else {
      slidesBannerTitle.textContent = 'No Active Item';
      slidesBannerAuthor.textContent = '';
      btnBannerLive.textContent = '⚡ LIVE';
      slideCardsGrid.innerHTML = '';
    }
  }

  // 4. Render Prompter View (Stage Prompter - ALWAYS tracks live projector output)
  if (prompterLyrics && prompterNextText) {
    if (liveSlide && !state.is_blackout) {
      if (state.is_clear_text) {
        prompterLyrics.innerHTML = `<span style="opacity: 0.5; font-size: 0.7em; display: block; margin-bottom: 8px;">(FOH TEXT CLEARED)</span>${escapeHtml(liveSlide.text || '')}`;
      } else {
        prompterLyrics.textContent = liveSlide.text || `[${liveSlide.label || ''}]`;
      }
      const nextPreview = resolveNextPresentationPreview(schedule, liveItem, livePlayPos);
      if (nextPreview.isNextSlide) {
        prompterNextText.textContent = nextPreview.text.replace(/\n/g, ' ').slice(0, 70);
      } else if (nextPreview.isNextScheduleItem && nextPreview.nextItem) {
        prompterNextText.textContent = `⏭ [Next: ${nextPreview.nextItem.title || 'Untitled'}]`;
      } else {
        prompterNextText.textContent = '(End of service)';
      }
    } else if (state.is_blackout) {
      prompterLyrics.innerHTML = '<span style="color: var(--blackout-red); font-size: 0.8em;">⬛ BLACKOUT ACTIVE ON FOH</span>';
      prompterNextText.textContent = '—';
    } else if (state.is_logo_override) {
      prompterLyrics.innerHTML = '<span style="color: var(--logo-amber); font-size: 0.8em;">⭐ LOGO ACTIVE ON FOH</span>';
      prompterNextText.textContent = '—';
    } else {
      prompterLyrics.textContent = 'No Live Item';
      prompterNextText.textContent = '—';
    }
  }
}

// --- Touch Swipe Gestures ---
function setupGestures() {
  const gestureZone = doc?.getElementById('main-content');
  if (!gestureZone) return;

  let touchStartX = 0;
  let touchStartY = 0;
  let touchStartTime = 0;

  gestureZone.addEventListener('touchstart', (e: any) => {
    if (e.touches && e.touches.length === 1) {
      touchStartX = e.touches[0].clientX;
      touchStartY = e.touches[0].clientY;
      touchStartTime = Date.now();
    }
  }, { passive: true });

  gestureZone.addEventListener('touchend', (e: any) => {
    if (!isFullControl) return;
    if (e.changedTouches && e.changedTouches.length === 1) {
      const deltaX = e.changedTouches[0].clientX - touchStartX;
      const deltaY = e.changedTouches[0].clientY - touchStartY;
      const elapsed = Date.now() - touchStartTime;

      // Require swift swipe (< 450ms) and minimum distance (45px)
      if (elapsed < 450) {
        if (Math.abs(deltaX) > 45 && Math.abs(deltaX) > Math.abs(deltaY) * 1.5) {
          // Horizontal Swipe: Next / Previous slide across all views
          if (deltaX < 0) {
            sendCommand('NextSlide'); // Swipe Left -> Next Slide
          } else {
            sendCommand('PrevSlide'); // Swipe Right -> Prev Slide
          }
        } else if (activeTab === 'view-prompter' && Math.abs(deltaY) > 45 && Math.abs(deltaY) > Math.abs(deltaX) * 1.5) {
          // Vertical Swipe: ONLY in non-scrolling Prompter view
          if (deltaY < 0) {
            sendCommand('NextItem'); // Swipe Up -> Next Item
          } else {
            sendCommand('PrevItem'); // Swipe Down -> Prev Item
          }
        }
      }
    }
  }, { passive: true });
}

// --- Wake Lock Toggle ---
function setupWakeLock() {
  if (!btnWakeLock) return;
  btnWakeLock.addEventListener('click', async () => {
    if (typeof navigator !== 'undefined' && 'wakeLock' in navigator) {
      if (!wakeLockSentinel) {
        try {
          wakeLockSentinel = await (navigator as any).wakeLock.request('screen');
          btnWakeLock.style.borderColor = '#00E676';
          btnWakeLock.style.color = '#00E676';
          wakeLockSentinel.addEventListener('release', () => {
            wakeLockSentinel = null;
            btnWakeLock.style.borderColor = '';
            btnWakeLock.style.color = '';
          });
        } catch {}
      } else {
        wakeLockSentinel.release();
        wakeLockSentinel = null;
      }
    } else {
      if (typeof alert !== 'undefined') alert('Screen Wake Lock is not supported on this browser.');
    }
  });
}

// --- Clock Update ---
function updateClock() {
  if (prompterClock) {
    const now = new Date();
    prompterClock.textContent = now.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
  }
}

// --- Button Bindings ---
function setupButtons() {
  if (btnNextSlide) btnNextSlide.addEventListener('click', () => sendCommand('NextSlide'));
  if (btnPrevSlide) btnPrevSlide.addEventListener('click', () => sendCommand('PrevSlide'));
  if (btnNextItem) btnNextItem.addEventListener('click', () => sendCommand('NextItem'));
  if (btnPrevItem) btnPrevItem.addEventListener('click', () => sendCommand('PrevItem'));

  if (btnLogo) btnLogo.addEventListener('click', () => sendCommand('ToggleLogo'));
  if (btnBlack) btnBlack.addEventListener('click', () => sendCommand('ToggleBlackout'));
  if (btnClear) btnClear.addEventListener('click', () => sendCommand('ToggleClearText'));

  if (miniPreviewCard) {
    miniPreviewCard.addEventListener('click', () => sendCommand('NextSlide'));
  }

  if (btnBannerLive) {
    btnBannerLive.addEventListener('click', () => {
      if (!latestSnapshot) return;
      const sched = latestSnapshot.schedule || { items: [] };
      const currentItem = (viewedItemId && sched.items)
        ? sched.items.find((it: any) => it.id === viewedItemId)
        : (latestSnapshot.state?.staged_item || latestSnapshot.state?.live_item);
      if (!currentItem) return;
      const idx = sched.items.findIndex((it: any) => it.id === currentItem.id);
      const targetIdx = idx !== -1 ? idx : null;
      sendCommand({ GoLive: { item_index: targetIdx, slide_index: 0 } });
    });
  }
}

// --- Fast initial HTTP snapshot load ---
export async function loadInitialState() {
  try {
    const res = await fetch('/api/state');
    if (res.ok) {
      const snapshot = await res.json();
      renderSnapshot(snapshot);
    }
  } catch (_) {}
}

// --- WebSocket Real-Time Connection (created only once a valid device token is confirmed) ---
export let wsClient: EngineWebSocketClient | null = null;

function initRemoteApp(deviceToken: string) {
  wsClient = createEngineWebSocket({
    deviceToken,
    onStatusChange: (status) => {
      if (statusDot && connText) {
        if (status === 'connected') {
          statusDot.className = 'status-dot connected';
          connText.textContent = 'Connected';
        } else if (status === 'connecting') {
          statusDot.className = 'status-dot reconnecting';
          connText.textContent = 'Connecting...';
        } else {
          statusDot.className = 'status-dot offline';
          connText.textContent = 'Offline';
        }
      }
    },
    onSnapshot: (snapshot) => {
      try {
        renderSnapshot(snapshot);
      } catch (e) {
        console.error('[RemoteClient] Render error:', e);
      }
    },
  });

  // Initialize remote UI listeners
  setupTabs();
  setupModeToggle();
  setupButtons();
  setupGestures();
  setupWakeLock();

  if (typeof window !== 'undefined') {
    setInterval(updateClock, 500);
    updateClock();
  }

  loadInitialState();

  // Export helpers on window for testing environments
  if (typeof window !== 'undefined') {
    (window as any).OS_REMOTE = {
      renderSnapshot,
      resolveSlideAt,
      getItemTypeIcon,
      getTagClass,
      sendCommand,
      switchTab,
      getMode: () => isFullControl ? 'control' : 'prompter',
      setMode: (mode: string) => {
        isFullControl = (mode === 'control');
        if (modeLabel) modeLabel.textContent = isFullControl ? 'Control' : 'Prompter';
      },
      getViewedItemId: () => viewedItemId,
      setViewedItemId: (id: string | null) => { viewedItemId = id; },
      getWsClient: () => wsClient,
    };
  }
}

// --- Pairing gate: verify the token before anything else runs ---
(async function bootstrapPairing() {
  const token = readTokenFromLocation();
  if (token) {
    // Strip the token out of the visible URL/address bar immediately so it
    // isn't left sitting there if the page is shared or the URL is copied.
    try {
      if (typeof sessionStorage !== 'undefined') sessionStorage.setItem(REMOTE_TOKEN_STORAGE_KEY, token);
      if (typeof history !== 'undefined' && history.replaceState) {
        history.replaceState(null, '', location.pathname + location.search);
      }
    } catch (_) {}
  }

  const candidateToken = token || (typeof sessionStorage !== 'undefined' ? sessionStorage.getItem(REMOTE_TOKEN_STORAGE_KEY) : null);

  if (!candidateToken || !(await verifyDeviceToken(candidateToken))) {
    showPairingGate();
    return;
  }

  hidePairingGate();
  initRemoteApp(candidateToken);
})();
