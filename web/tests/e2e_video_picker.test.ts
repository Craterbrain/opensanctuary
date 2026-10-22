import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { join } from "path";
import { mkdtempSync, mkdirSync } from "fs";
import { tmpdir } from "os";
import { spawnSync } from "child_process";
import { spawnTestServer, teardownTestServer, ensureArtifactDir, type SpawnedTestServer } from "./e2e_helpers";

describe("E2E Live Test: Background Picker Videos Tab & Video Background Playback", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  const PORT = 9015;
  let artifactDir: string;

  // A one-second clip made with ffmpeg into this test's own media folder (the picker lists the
  // videos it finds there). Without ffmpeg there is nothing to play, so the test skips itself.
  const mediaDir = mkdtempSync(join(tmpdir(), "os-next-video-picker-media-"));
  mkdirSync(join(mediaDir, "videos"), { recursive: true });
  const haveClip = spawnSync("ffmpeg", ["-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=duration=1:size=160x90:rate=10",
    "-c:v", "libvpx", "-b:v", "200k", join(mediaDir, "videos", "sample-loop.webm")]).status === 0;

  beforeAll(async () => {
    artifactDir = ensureArtifactDir();
    server = await spawnTestServer({
      port: PORT,
      tempPrefix: "os-next-video-picker-e2e-",
      // Media lives alongside the database by default (docs/paths.md); point it at the folder
      // holding the clip made above instead of the fresh tmp dir --db-path implies.
      extraArgs: ["--media-dir", mediaDir],
    });

    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
    page.on("pageerror", err => console.log("PAGE ERR:", err.message));

    await page.goto(`http://127.0.0.1:${PORT}/`);
    await page.waitForFunction(() => (window as any).__APP_READY__ === true, { timeout: 15000 });
    // Not waitForLoadState("networkidle") -- the app's persistent time-sync
    // WebSocket means "idle" may never fire; __APP_READY__ above is the
    // real readiness signal (see main.ts).
  }, 45000);

  afterAll(async () => {
    if (browser) await browser.close();
    await teardownTestServer(server);
  });

  test.skipIf(!haveClip)("Video Background workflow: modal tabs, library grid, URL preview, canvas playback, and properties browse", async () => {
    // 1. Open slide editor
    await page.evaluate(() => (window as any).openSlideEditor('song'));
    await page.waitForSelector('#create-modal', { state: 'visible', timeout: 8000 });

    // 2. Open background picker
    await page.click('#tb-background');
    await page.waitForSelector('#media-image-picker-modal', { state: 'visible', timeout: 5000 });

    // 3. Click Videos header tab
    await page.click('#bg-picker-tab-videos');
    await page.waitForSelector('#bg-picker-panel-videos', { state: 'visible' });

    // Verify Videos tab is active and other tabs inactive
    const isVideosTabActive = await page.$eval('#bg-picker-tab-videos', el => el.classList.contains('active'));
    const isImagesTabActive = await page.$eval('#bg-picker-tab-images', el => el.classList.contains('active'));
    const isCssTabActive = await page.$eval('#bg-picker-tab-css', el => el.classList.contains('active'));

    expect(isVideosTabActive).toBe(true);
    expect(isImagesTabActive).toBe(false);
    expect(isCssTabActive).toBe(false);

    // Verify panel visibility
    const isVideosPanelVisible = await page.isVisible('#bg-picker-panel-videos');
    const isImagesPanelVisible = await page.isVisible('#bg-picker-panel-images');
    const isCssPanelVisible = await page.isVisible('#bg-picker-panel-css');

    expect(isVideosPanelVisible).toBe(true);
    expect(isImagesPanelVisible).toBe(false);
    expect(isCssPanelVisible).toBe(false);

    // Verify footer actions
    const isVideosFooterVisible = await page.isVisible('#bg-picker-footer-actions-videos');
    expect(isVideosFooterVisible).toBe(true);

    // 4. Video Library subtab shows auto-discovered video cards
    await page.waitForSelector('#media-video-picker-grid .media-video-card', { timeout: 6000 });

    const videoCardsCount = await page.$$eval('#media-video-picker-grid .media-video-card', cards => cards.length);
    expect(videoCardsCount).toBeGreaterThanOrEqual(1);

    const firstCardTitle = await page.$eval('#media-video-picker-grid .media-video-card .media-video-info', el => el.textContent?.trim());
    expect(firstCardTitle).toBeDefined();
    expect(firstCardTitle!.length).toBeGreaterThan(0);

    // Take screenshot of Videos tab in Background Picker
    await page.screenshot({ path: `${artifactDir}/screenshot_editor_bg_videos_tab.png` });

    // 5. Switch to 'From URL' subtab
    await page.click('#btn-video-subtab-url');

    expect(await page.isVisible('#video-subview-url')).toBe(true);
    expect(await page.isVisible('#video-subview-library')).toBe(false);
    expect(await page.isVisible('#video-subview-ytdl')).toBe(false);

    // Fill URL and test preview
    const sampleUrl = 'https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/BigBuckBunny.mp4';
    await page.fill('#video-url-input', sampleUrl);
    await page.click('#btn-video-url-preview');
    await page.waitForFunction(() => (document.getElementById('video-url-preview-element') as HTMLVideoElement)?.src.includes('BigBuckBunny.mp4'));

    const previewSrc = await page.$eval('#video-url-preview-element', (el: HTMLVideoElement) => el.src);
    expect(previewSrc).toContain('BigBuckBunny.mp4');
    expect(await page.isVisible('#video-url-preview-container')).toBe(true);

    // Screenshot of URL tab
    await page.screenshot({ path: `${artifactDir}/screenshot_editor_bg_video_url_tab.png` });

    // 6. Switch to the download subtab
    await page.click('#btn-video-subtab-ytdl');

    expect(await page.isVisible('#video-subview-ytdl')).toBe(true);
    expect(await page.isVisible('#video-subview-url')).toBe(false);
    expect(await page.isVisible('#video-subview-library')).toBe(false);
    // This tab exists because an installed plugin (the bundled Video Downloader) declares the
    // video-picker slot; until it's approved, the tab offers its review instead of its page.
    expect(await page.isVisible('#plugin-picker-hint')).toBe(true);
    expect(await page.isVisible('#plugin-picker-hint button[data-plugin-id="ytdlp"]')).toBe(true);
    expect(await page.textContent('#btn-video-subtab-ytdl')).toContain('Video Downloader');

    // 7. Switch back to Library, select video card, and apply to active slide
    await page.click('#btn-video-subtab-library');

    // Click first card to select
    await page.click('#media-video-picker-grid .media-video-card');
    await page.waitForSelector('#media-video-picker-grid .media-video-card.selected');

    const isCardSelected = await page.$eval('#media-video-picker-grid .media-video-card', el => el.classList.contains('selected'));
    expect(isCardSelected).toBe(true);

    // Verify loop checkbox is checked
    const isLoopChecked = await page.$eval('#video-picker-loop', (el: HTMLInputElement) => el.checked);
    expect(isLoopChecked).toBe(true);

    // Click Apply to Active Slide
    await page.click('#btn-apply-video-slide');
    await page.waitForSelector('#media-image-picker-modal', { state: 'hidden' });

    // Modal should be closed
    const isModalVisible = await page.isVisible('#media-image-picker-modal');
    expect(isModalVisible).toBe(false);

    // Canvas stage should contain looping video
    await page.waitForSelector('.editor-slide-canvas-stage video.slide-canvas-bg-video', { timeout: 4000 });
    const bgVideoDetails = await page.evaluate(() => {
      const vid = document.querySelector('.editor-slide-canvas-stage video.slide-canvas-bg-video') as HTMLVideoElement;
      if (!vid) return null;
      return {
        hasSrc: !!vid.src,
        isLoop: vid.loop,
        isMuted: vid.muted,
        zIndex: vid.style.zIndex
      };
    });

    expect(bgVideoDetails).not.toBeNull();
    expect(bgVideoDetails!.hasSrc).toBe(true);
    expect(bgVideoDetails!.isLoop).toBe(true);
    expect(bgVideoDetails!.isMuted).toBe(true);
    expect(bgVideoDetails!.zIndex).toBe('0');

    // Screenshot of applied video background in slide editor
    await page.screenshot({ path: `${artifactDir}/screenshot_editor_bg_video_applied.png` });

    // 8. Properties panel video Browse button directly opens Videos tab
    await page.waitForSelector('#prop-bg-vid-browse', { timeout: 3000 });
    await page.click('#prop-bg-vid-browse');
    await page.waitForSelector('#media-image-picker-modal', { state: 'visible', timeout: 5000 });

    // Should open directly to the Videos tab
    const isVideosTabAgain = await page.$eval('#bg-picker-tab-videos', el => el.classList.contains('active'));
    expect(isVideosTabAgain).toBe(true);

    // Close modal
    await page.click('#btn-close-image-picker');
  }, 45000);
});
