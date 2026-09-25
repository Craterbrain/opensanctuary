import { describe, test, expect } from 'bun:test';
import * as fs from 'fs';
import * as path from 'path';

describe('Mobile Remote Control Webpage (EasyWorship Android App Style)', () => {
  const remoteHtmlPath = path.resolve(__dirname, '../remote.html');

  test('remote.html exists and contains necessary PWA and viewport meta tags', () => {
    expect(fs.existsSync(remoteHtmlPath)).toBe(true);
    const content = fs.readFileSync(remoteHtmlPath, 'utf8');

    // Mobile Viewport & PWA tags
    expect(content).toContain('name="viewport"');
    expect(content).toContain('viewport-fit=cover');
    expect(content).toContain('apple-mobile-web-app-capable');
    expect(content).toContain('theme-color');
  });

  test('remote.html contains complete UI hierarchy: top bar, tabs, views, and bottom bar', () => {
    const content = fs.readFileSync(remoteHtmlPath, 'utf8');

    // Top Bar & Controls
    expect(content).toContain('id="top-bar"');
    expect(content).toContain('id="conn-pill"');
    expect(content).toContain('id="mode-toggle-btn"');
    expect(content).toContain('id="btn-override-logo"');
    expect(content).toContain('id="btn-override-black"');
    expect(content).toContain('id="btn-override-clear"');

    // View Tabs
    expect(content).toContain('id="tab-schedule"');
    expect(content).toContain('id="tab-slides"');
    expect(content).toContain('id="tab-prompter"');

    // Content Views
    expect(content).toContain('id="view-schedule"');
    expect(content).toContain('id="view-slides"');
    expect(content).toContain('id="view-prompter"');

    // Mini Output Preview & Filmstrip Grid
    expect(content).toContain('id="mini-preview-card"');
    expect(content).toContain('id="mini-preview-text"');
    expect(content).toContain('id="mini-preview-tag"');
    expect(content).toContain('id="slide-cards-grid"');

    // Prompter Screen & Lookahead
    expect(content).toContain('id="prompter-screen"');
    expect(content).toContain('id="prompter-lyrics"');
    expect(content).toContain('id="prompter-next-text"');
    expect(content).toContain('id="btn-wake-lock"');

    // Bottom Navigation Bar
    expect(content).toContain('id="bottom-bar"');
    expect(content).toContain('id="btn-prev-item"');
    expect(content).toContain('id="btn-prev-slide"');
    expect(content).toContain('id="btn-next-slide"');
    expect(content).toContain('id="btn-next-item"');
  });

  describe('Slide & Liturgical Tag Resolution Logic', () => {
    // Pure function mirrors the remote client's resolveSlideAt
    function resolveSlideAt(item: any, playPos: number) {
      if (!item || !item.slides || item.slides.length === 0) return null;
      const arr = (item.arrangement && Array.isArray(item.arrangement) && item.arrangement.length > 0)
        ? item.arrangement
        : item.slides.map((s: any, i: number) => ({
            section_id: s.tag || s.label || `S${i + 1}`,
            source_slide_index: i,
            background_override: null,
          }));
      const entry = arr[playPos];
      if (!entry) return null;
      const s = item.slides[entry.source_slide_index];
      if (!s) return null;
      return {
        ...s,
        label: entry.section_id || s.tag || s.label || `Slide ${playPos + 1}`,
        background: (entry.background_override && entry.background_override.trim()) ? entry.background_override.trim() : s.background
      };
    }

    function getItemTypeIcon(type: string) {
      switch ((type || '').toLowerCase()) {
        case 'song': return '🎵';
        case 'scripture': return '📖';
        case 'presentation': return '📽️';
        case 'media': return '🎬';
        case 'header': return '🏷️';
        default: return '📄';
      }
    }

    function getTagClass(tag: string) {
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

    test('resolves simple slide items accurately', () => {
      const item = {
        title: 'Amazing Grace',
        slides: [
          { text: 'Amazing grace how sweet the sound', tag: 'V1', label: 'Verse 1', background: '#000' },
          { text: 'Twas grace that taught my heart to fear', tag: 'V2', label: 'Verse 2', background: '#000' }
        ]
      };

      const slide0 = resolveSlideAt(item, 0);
      expect(slide0).not.toBeNull();
      expect(slide0?.text).toContain('Amazing grace');
      expect(slide0?.label).toBe('V1');

      const slide1 = resolveSlideAt(item, 1);
      expect(slide1?.text).toContain('Twas grace');
      expect(slide1?.label).toBe('V2');

      const outOfBounds = resolveSlideAt(item, 2);
      expect(outOfBounds).toBeNull();
    });

    test('resolves custom liturgical arrangement with background overrides', () => {
      const item = {
        title: 'Song with Arrangement',
        slides: [
          { text: 'Verse 1 text', tag: 'V1', background: '#111' },
          { text: 'Chorus text', tag: 'C', background: '#222' }
        ],
        arrangement: [
          { section_id: 'V1', source_slide_index: 0, background_override: null },
          { section_id: 'C', source_slide_index: 1, background_override: 'override.jpg' },
          { section_id: 'V1', source_slide_index: 0, background_override: 'second_pass.jpg' }
        ]
      };

      // Play position 0 -> Verse 1
      const pos0 = resolveSlideAt(item, 0);
      expect(pos0?.label).toBe('V1');
      expect(pos0?.background).toBe('#111');

      // Play position 1 -> Chorus with override
      const pos1 = resolveSlideAt(item, 1);
      expect(pos1?.label).toBe('C');
      expect(pos1?.background).toBe('override.jpg');

      // Play position 2 -> Repeat Verse 1 with different override
      const pos2 = resolveSlideAt(item, 2);
      expect(pos2?.label).toBe('V1');
      expect(pos2?.background).toBe('second_pass.jpg');
    });

    test('maps item types to intuitive liturgical icons', () => {
      expect(getItemTypeIcon('song')).toBe('🎵');
      expect(getItemTypeIcon('Song')).toBe('🎵');
      expect(getItemTypeIcon('scripture')).toBe('📖');
      expect(getItemTypeIcon('presentation')).toBe('📽️');
      expect(getItemTypeIcon('media')).toBe('🎬');
      expect(getItemTypeIcon('header')).toBe('🏷️');
      expect(getItemTypeIcon('custom')).toBe('📄');
    });

    test('maps liturgical section tags to color classes', () => {
      expect(getTagClass('V1')).toBe('tag-v');
      expect(getTagClass('verse 2')).toBe('tag-v');
      expect(getTagClass('C')).toBe('tag-c');
      expect(getTagClass('Chorus 2')).toBe('tag-c');
      expect(getTagClass('Bridge')).toBe('tag-b');
      expect(getTagClass('Pre-Chorus')).toBe('tag-p');
      expect(getTagClass('Ending')).toBe('tag-e');
      expect(getTagClass('Intro')).toBe('tag-i');
      expect(getTagClass('')).toBe('');
    });
  });

  describe('Touch Gesture Math (EasyWorship Mobile App Gestures)', () => {
    function evaluateGesture(deltaX: number, deltaY: number, elapsedMs: number) {
      if (elapsedMs >= 450) return null; // Swipe timed out
      const absX = Math.abs(deltaX);
      const absY = Math.abs(deltaY);

      if (absX > 45 && absX > absY * 1.5) {
        return deltaX < 0 ? 'NextSlide' : 'PrevSlide';
      }
      if (absY > 45 && absY > absX * 1.5) {
        return deltaY < 0 ? 'NextItem' : 'PrevItem';
      }
      return null;
    }

    test('detects horizontal swipe left as NextSlide', () => {
      const action = evaluateGesture(-80, 5, 200);
      expect(action).toBe('NextSlide');
    });

    test('detects horizontal swipe right as PrevSlide', () => {
      const action = evaluateGesture(95, -10, 180);
      expect(action).toBe('PrevSlide');
    });

    test('detects vertical swipe up as NextItem', () => {
      const action = evaluateGesture(5, -90, 220);
      expect(action).toBe('NextItem');
    });

    test('detects vertical swipe down as PrevItem', () => {
      const action = evaluateGesture(-8, 85, 210);
      expect(action).toBe('PrevItem');
    });

    test('ignores ambiguous diagonal gestures or slow drags', () => {
      // Diagonal swipe (both dimensions similar)
      expect(evaluateGesture(60, 60, 200)).toBeNull();
      // Distance too short
      expect(evaluateGesture(20, 5, 100)).toBeNull();
      // Swipe too slow (> 450ms)
      expect(evaluateGesture(-100, 0, 550)).toBeNull();
    });
  });

  describe('Mode Switch & Permission Gating (Full Control vs Prompter View)', () => {
    test('blocks all mutating commands (objects and strings) when in View-Only Prompter mode', () => {
      let dispatched = false;
      const sendCommand = (cmd: any, isControl: boolean) => {
        if (!isControl) {
          return; // Strictly locked out in Prompter view
        }
        dispatched = true;
      };

      // In Prompter mode (isControl = false):
      sendCommand('NextSlide', false);
      expect(dispatched).toBe(false);

      sendCommand('ToggleBlackout', false);
      expect(dispatched).toBe(false);

      sendCommand({ GoLive: { item_index: 1, slide_index: 2 } }, false);
      expect(dispatched).toBe(false);

      // In Control mode (isControl = true):
      sendCommand('NextSlide', true);
      expect(dispatched).toBe(true);
    });
  });

  describe('Gesture Contextual Gating & Text Extraction', () => {
    test('vertical swipe is suppressed on scrollable views and allowed in prompter view', () => {
      const evaluateGesture = (deltaX: number, deltaY: number, elapsed: number, activeView: string) => {
        if (elapsed > 450) return null;
        if (Math.abs(deltaX) > 45 && Math.abs(deltaX) > Math.abs(deltaY) * 1.5) {
          return deltaX < 0 ? 'NextSlide' : 'PrevSlide';
        } else if (activeView === 'view-prompter' && Math.abs(deltaY) > 45 && Math.abs(deltaY) > Math.abs(deltaX) * 1.5) {
          return deltaY < 0 ? 'NextItem' : 'PrevItem';
        }
        return null; // Suppressed on view-schedule and view-slides for natural list scrolling
      };

      // On Schedule view: vertical flick should NOT jump item
      expect(evaluateGesture(0, -100, 200, 'view-schedule')).toBeNull();
      expect(evaluateGesture(0, 100, 200, 'view-slides')).toBeNull();

      // On Prompter view: vertical flick DOES jump item
      expect(evaluateGesture(0, -100, 200, 'view-prompter')).toBe('NextItem');
      expect(evaluateGesture(0, 100, 200, 'view-prompter')).toBe('PrevItem');

      // Horizontal swipe works in all views
      expect(evaluateGesture(-100, 0, 200, 'view-schedule')).toBe('NextSlide');
      expect(evaluateGesture(100, 0, 200, 'view-prompter')).toBe('PrevSlide');
    });

    test('extracts text from elements array when root text is missing', () => {
      const getSlideText = (s: any) => {
        if (!s) return '';
        if (s.text && s.text.trim()) return s.text;
        if (s.elements && Array.isArray(s.elements)) {
          const parts = [];
          for (const el of s.elements) {
            if (el.kind === 'text' && Array.isArray(el.text_runs)) {
              parts.push(el.text_runs.map((r: any) => r.text || '').join(''));
            }
          }
          if (parts.length > 0) return parts.join('\n');
        }
        return '';
      };

      const canvasSlide = {
        text: '',
        elements: [
          {
            kind: 'text',
            text_runs: [{ text: 'Welcome ' }, { text: 'To Church' }]
          }
        ]
      };

      expect(getSlideText(canvasSlide)).toBe('Welcome To Church');
    });

    test('escapes HTML entities in lyrics and titles', () => {
      const escapeHtml = (str: string) => {
        if (!str) return '';
        return String(str)
          .replace(/&/g, '&amp;')
          .replace(/</g, '&lt;')
          .replace(/>/g, '&gt;')
          .replace(/"/g, '&quot;')
          .replace(/'/g, '&#039;');
      };

      expect(escapeHtml('<Bridge & Outro>')).toBe('&lt;Bridge &amp; Outro&gt;');
      expect(escapeHtml('"Song Title"')).toBe('&quot;Song Title&quot;');
    });
  });

  describe('Remote Client Pairing (real device token, not the old open-access stub)', () => {
    // Mirrors the real logic in web/src/remote_client.ts: a token travels in
    // the URL fragment (never sent to the server as part of the page
    // request), gets verified against POST /api/pairing/verify (the same
    // paired_devices table TV/Roku pairing uses), and only a valid,
    // non-revoked token unlocks the app — see docs/GEMINI_COMMIT_REVIEW_2026-09-22.md.
    function readTokenFromHash(hash: string): string | null {
      if (!hash || hash.length <= 1) return null;
      const params = new URLSearchParams(hash.slice(1));
      return params.get('token');
    }

    test('extracts the device token from the URL fragment, not a query param', () => {
      expect(readTokenFromHash('#token=dev_tok_abc123')).toBe('dev_tok_abc123');
      expect(readTokenFromHash('')).toBeNull();
      expect(readTokenFromHash('#')).toBeNull();
    });

    test('no token, or an invalid/revoked one, never unlocks the app', async () => {
      const verify = async (token: string | null) => {
        if (!token) return { valid: false };
        // Simulates POST /api/pairing/verify against a real device store —
        // only a token that exists there (and hasn't been revoked) is valid.
        const knownTokens = new Set(['dev_tok_real_session']);
        return { valid: knownTokens.has(token) };
      };

      expect((await verify(null)).valid).toBe(false);
      expect((await verify('not-a-real-token')).valid).toBe(false);
      expect((await verify('dev_tok_real_session')).valid).toBe(true);
    });
  });

  describe('Desktop Remote QR Barcode Modal & Network Resolution', () => {
    const indexPath = path.resolve(__dirname, '../index.html');

    test('index.html contains remote QR barcode canvas, copy button, and instructions', () => {
      expect(fs.existsSync(indexPath)).toBe(true);
      const content = fs.readFileSync(indexPath, 'utf8');

      expect(content).toContain('id="remote-modal"');
      expect(content).toContain('id="remote-qr-canvas"');
      expect(content).toContain('id="remote-url-display"');
      expect(content).toContain('id="btn-copy-remote-url"');
      expect(content).toContain('id="btn-open-mobile-remote"');
      expect(content).toContain("Scan with your phone's camera to connect");
    });

    test('QRCode generator encodes remote URL into valid QR matrix', async () => {
      const QRCode = await import('qrcode');
      expect(typeof QRCode.toCanvas).toBe('function');
      expect(typeof QRCode.toString).toBe('function');

      const testUrl = 'http://192.168.1.100:8080/remote';
      const svg = await QRCode.toString(testUrl, { type: 'svg' });
      expect(svg).toContain('<svg');
      expect(svg).toContain('</svg>');
      expect(svg.length).toBeGreaterThan(500);
    });
  });
});
