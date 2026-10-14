import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { mkdtempSync, writeFileSync, chmodSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { spawnTestServer, teardownTestServer, ensureArtifactDir, type SpawnedTestServer } from "./e2e_helpers";

// A stand-in for yt-dlp on PATH (the plugin uses a copy already on the computer, and never tries to
// replace it): prints progress, writes a media file into the job folder, prints the JSON line.
const FAKE_YTDLP = `#!/bin/sh
if [ "$1" = "--version" ]; then echo "2099.01.01"; exit 0; fi
out=""; audio=0; cookies=0
while [ $# -gt 0 ]; do
  case "$1" in -o) out="$2"; shift;; -x) audio=1;; --cookies-from-browser) cookies=1;; esac
  shift
done
dir=$(dirname "$out")
if [ $cookies = 1 ]; then echo "ERROR: could not read browser cookies" >&2; exit 1; fi
ext=webm; [ $audio = 1 ] && ext=mp3
echo "[download]  25.0% of 1.00MiB at 1.00MiB/s ETA 00:02"
sleep 0.4
echo "[download]  75.0% of 1.00MiB at 1.00MiB/s ETA 00:01"
sleep 0.4
printf 'fake media' > "$dir/E2E Clip [zz9].$ext"
echo "{\\"id\\":\\"zz9\\",\\"title\\":\\"E2E Clip\\",\\"duration\\":3.5,\\"_filename\\":\\"$dir/E2E Clip [zz9].webm\\"}"
`;

describe("E2E Live Test: Video Downloader plugin (yt-dlp) downloads through the host", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  const PORT = 9076;
  let artifactDir: string;

  beforeAll(async () => {
    const toolsDir = mkdtempSync(join(tmpdir(), "os-next-fake-ytdlp-"));
    writeFileSync(join(toolsDir, "yt-dlp"), FAKE_YTDLP);
    chmodSync(join(toolsDir, "yt-dlp"), 0o755);
    const mediaDir = mkdtempSync(join(tmpdir(), "os-next-ytdlp-media-"));
    server = await spawnTestServer({
      port: PORT, tempPrefix: "os-next-ytdlp-plugin-e2e-",
      extraArgs: ["--media-dir", mediaDir],
      env: { PATH: `${toolsDir}:${process.env.PATH}` },
    });
    artifactDir = ensureArtifactDir();
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  }, 45000);

  afterAll(async () => {
    if (browser) await browser.close();
    await teardownTestServer(server);
  });

  const frameOf = () => page.frames().find((f) => f.url().includes("/plugin-frame/ytdlp/") && f.url().includes("slot=library-source"));

  test("needs approval (showing the exact command), then downloads into the Media library", async () => {
    await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "networkidle" });
    // The built-in downloader is gone from the server and the Import dialog.
    expect((await fetch(`http://127.0.0.1:${PORT}/api/media/ytdlp/download`, { method: "POST" })).status).not.toBe(200);
    expect((await fetch(`http://127.0.0.1:${PORT}/api/ytdlp-updater/status`)).status).toBe(404);
    expect(await page.locator("#tab-btn-ytdlp").count()).toBe(0);
    await page.click('.tab-btn[data-tab="media"]');
    expect(await page.locator('.category-node:has-text("yt-dlp")').count()).toBe(0); // not until approved

    await page.keyboard.press("F2");
    await page.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await page.locator(".settings-category-btn:has-text('Plugins')").click();
    const card = page.locator('.plugin-card[data-plugin-id="ytdlp"]');
    await card.waitFor({ state: "visible", timeout: 10000 });
    // The operator can read exactly what it may run, and where its tool comes from.
    const command = await card.locator(".plugin-command").first().innerText();
    expect(command).toContain("yt-dlp --no-playlist --newline --progress --print-json");
    expect(command).toContain("[if browser: --cookies-from-browser {browser}]");
    expect(command.trim().endsWith("-- {url}")).toBe(true);
    const text = await card.innerText();
    expect(text).toContain("github.com/yt-dlp/yt-dlp");
    expect(text).toContain("process.run");
    expect(text).toContain("Critical");
    await page.screenshot({ path: `${artifactDir}/screenshot_ytdlp_plugin_review.png` });
    await card.locator("button:has-text('Approve')").click();
    await page.waitForFunction(
      () => document.querySelector('.plugin-card[data-plugin-id="ytdlp"] .plugin-state')?.textContent?.includes("Approved"),
      null, { timeout: 10000 },
    );

    // It appears in the Media library sidebar, where "Download Video (yt-dlp)" used to be.
    await page.keyboard.press("Escape");
    const node = page.locator('.category-node:has-text("Download Video (yt-dlp)")');
    await node.waitFor({ state: "visible", timeout: 10000 });
    await node.click();
    const frame = frameOf()!;
    await frame.waitForSelector("#url", { state: "visible", timeout: 15000 });
    await frame.waitForFunction(() => document.getElementById("tool-status")?.textContent?.includes("2099.01.01"), null, { timeout: 15000 });
    await page.screenshot({ path: `${artifactDir}/screenshot_ytdlp_plugin_panel.png` });
    await page.screenshot({ path: "../docs/screenshots/ytdlp-plugin.png" });

    // The same plugin also fills the video background picker's download tab (and the pointer there goes away).
    expect(await page.locator('#plugin-picker-pane iframe[data-slot="video-picker"]').count()).toBe(1);
    expect(await page.locator("#plugin-picker-hint").evaluate((el) => el.classList.contains("hidden"))).toBe(true);

    // A bad URL never starts anything.
    await frame.fill("#url", "not a url");
    await frame.click("#download");
    await frame.waitForFunction(() => document.getElementById("status")?.textContent?.includes("http"), null, { timeout: 5000 });

    // A real run through the host: progress shows, the first attempt (with browser cookies, "Auto-Detect")
    // fails and the plugin retries without them, then the file is added to the library.
    await frame.fill("#url", "https://video.example/watch?v=zz9");
    await frame.click("#download");
    await frame.waitForFunction(() => /Retrying direct download|Downloading: /.test(document.getElementById("status")?.textContent || ""), null, { timeout: 15000 });
    try {
      await frame.waitForFunction(() => /^(Completed|Failed)/.test(document.getElementById("status")?.textContent || ""), null, { timeout: 30000 });
    } catch (e) {
      throw new Error("download never finished; status: " + (await frame.locator("#status").innerText()));
    }
    expect(await frame.locator("#status").innerText()).toStartWith("Completed");
    const media = await (await fetch(`http://127.0.0.1:${PORT}/api/media`)).json();
    const item = media.find((m: any) => m.id === "plugin_ytdlp_ytdlp_zz9");
    expect(item?.name).toBe("E2E Clip");
    expect(item?.file_path).toBe("/media/videos/E2E Clip [zz9].webm");
    expect(item?.duration_seconds).toBe(3.5);

    // And it shows up in the Media list like anything else.
    await page.locator('.category-node:has-text("All Media")').click();
    await page.waitForSelector("text=E2E Clip", { timeout: 10000 });
  }, 120000);
});
