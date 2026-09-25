import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawn, type Subprocess } from "bun";
import { resolve, join } from "path";
import { mkdtempSync, rmSync } from "fs";
import { tmpdir } from "os";

describe("E2E Visual Comparison: Linux vs Windows Release Binaries", () => {
  let linuxProc: Subprocess;
  let winProc: Subprocess;
  let browser: Browser;
  let linuxPage: Page;
  let winPage: Page;
  let linuxTestDir: string;
  let winTestDir: string;

  const LINUX_PORT = 9061;
  const WIN_PORT = 9062;
  const artifactDir = "/home/jasonb/.gemini/antigravity/brain/f8991531-6ae4-4b9a-9bd5-2b8c056c8256";

  const sampleHymn = {
    id: "hymn_comparison",
    title: "Crown Him with Many Crowns",
    subtitle: "Diademata",
    notes: "Festive Easter Hymn",
    item_type: "song",
    author_or_ref: "Matthew Bridges & Godfrey Thring",
    slides: [
      {
        text: "Crown Him with many crowns,\nThe Lamb upon His throne;\nHark! how the heavenly anthem drowns\nAll music but its own!",
        header: null,
        label: "Verse 1",
        tag: "V1",
        background: "#0d1b2a",
        notes: null,
        elements: []
      },
      {
        text: "Awake, my soul, and sing\nOf Him who died for thee,\nAnd hail Him as thy matchless King\nThrough all eternity.",
        header: null,
        label: "Chorus",
        tag: "C",
        background: "#1b263b",
        notes: null,
        elements: []
      },
      {
        text: "Crown Him the Lord of peace,\nWhose power a sceptre sways\nFrom pole to pole, that wars may cease,\nAnd all be prayer and praise.",
        header: null,
        label: "Verse 2",
        tag: "V2",
        background: "#415a77",
        notes: null,
        elements: []
      }
    ],
    background: null,
    theme_name: null,
    is_expanded: false,
    is_section_header: false,
    arrangement: []
  };

  beforeAll(async () => {
    linuxTestDir = mkdtempSync(join(tmpdir(), "os-next-linux-comp-"));
    winTestDir = mkdtempSync(join(tmpdir(), "os-next-win-comp-"));

    const linuxBin = resolve(__dirname, "../../target/release/os-next");
    const winBin = resolve(__dirname, "../../target/x86_64-pc-windows-gnu/release/os-next.exe");
    const webDir = resolve(__dirname, "../");

    // 1. Spawn native Linux server
    linuxProc = spawn([
      linuxBin,
      "--headless",
      "--port", LINUX_PORT.toString(),
      "--db-path", join(linuxTestDir, "linux.db"),
      "--web-dir", webDir
    ], {
      cwd: resolve(__dirname, "../../"),
      stdout: "ignore",
      stderr: "ignore"
    });

    // 2. Spawn Windows server via Wine
    winProc = spawn([
      "wine",
      winBin,
      "--headless",
      "--port", WIN_PORT.toString(),
      "--db-path", join(winTestDir, "win.db"),
      "--web-dir", "web"
    ], {
      cwd: resolve(__dirname, "../../"),
      stdout: "ignore",
      stderr: "ignore"
    });

    // Wait for Linux server
    let linuxReady = false;
    for (let i = 0; i < 40; i++) {
      try {
        const res = await fetch(`http://127.0.0.1:${LINUX_PORT}/api/network/info`);
        if (res.ok) { linuxReady = true; break; }
      } catch (_) {}
      await new Promise(r => setTimeout(r, 250));
    }
    if (!linuxReady) throw new Error("Linux server failed to start in 10s");

    // Wait for Windows server
    let winReady = false;
    for (let i = 0; i < 40; i++) {
      try {
        const res = await fetch(`http://127.0.0.1:${WIN_PORT}/api/network/info`);
        if (res.ok) { winReady = true; break; }
      } catch (_) {}
      await new Promise(r => setTimeout(r, 250));
    }
    if (!winReady) throw new Error("Windows server failed to start in 10s");

    // Seed schedule on both servers
    await fetch(`http://127.0.0.1:${LINUX_PORT}/api/command`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ AddToSchedule: sampleHymn })
    });

    await fetch(`http://127.0.0.1:${WIN_PORT}/api/command`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ AddToSchedule: sampleHymn })
    });

    browser = await chromium.launch({ headless: true });

    // Open Linux page
    linuxPage = await browser.newPage({ viewport: { width: 1440, height: 900 } });
    linuxPage.on("pageerror", err => console.log("LINUX PAGE ERR:", err.message));

    // Open Windows page
    winPage = await browser.newPage({ viewport: { width: 1440, height: 900 } });
    winPage.on("pageerror", err => console.log("WIN PAGE ERR:", err.message));
  }, 60000);

  afterAll(async () => {
    if (browser) await browser.close();
    if (linuxProc) {
      linuxProc.kill();
      await linuxProc.exited;
    }
    if (winProc) {
      winProc.kill();
      await winProc.exited;
    }
    try { rmSync(linuxTestDir, { recursive: true, force: true }); } catch (_) {}
    try { rmSync(winTestDir, { recursive: true, force: true }); } catch (_) {}
  });

  test("Action 1: Home Console & Presentation / Live State Display", async () => {
    // Navigate both pages
    await linuxPage.goto(`http://127.0.0.1:${LINUX_PORT}/`, { waitUntil: "networkidle" });
    await linuxPage.waitForFunction(() => (window as any).__APP_READY__ === true, { timeout: 15000 });

    await winPage.goto(`http://127.0.0.1:${WIN_PORT}/`, { waitUntil: "networkidle" });
    await winPage.waitForFunction(() => (window as any).__APP_READY__ === true, { timeout: 15000 });

    await linuxPage.waitForTimeout(500);
    await winPage.waitForTimeout(500);

    // Capture Home Console Screenshots
    await linuxPage.screenshot({ path: `${artifactDir}/screenshot_linux_console.png` });
    await winPage.screenshot({ path: `${artifactDir}/screenshot_windows_console.png` });

    console.log("Action 1 captured: Home Console & Live Display");
  }, 30000);

  test("Action 2: Settings Dialog — Network Tab & Detected Interfaces", async () => {
    // Open Settings on Linux (F2)
    await linuxPage.keyboard.press("F2");
    await linuxPage.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await linuxPage.locator(".settings-category-btn:has-text('Network')").click();
    await linuxPage.waitForSelector(".network-overview-card", { state: "visible", timeout: 5000 });

    // Open Settings on Windows (F2)
    await winPage.keyboard.press("F2");
    await winPage.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await winPage.locator(".settings-category-btn:has-text('Network')").click();
    await winPage.waitForSelector(".network-overview-card", { state: "visible", timeout: 5000 });

    // Wait for LAN conflict check badges to settle on both
    await linuxPage.waitForFunction(() => {
      const el = document.getElementById("network-hostname-badge");
      return el && el.textContent && (el.textContent.includes("Available") || el.textContent.includes("LAN Conflict"));
    }, { timeout: 15000 });

    await winPage.waitForFunction(() => {
      const el = document.getElementById("network-hostname-badge");
      return el && el.textContent && (el.textContent.includes("Available") || el.textContent.includes("LAN Conflict"));
    }, { timeout: 15000 });

    // Screenshot top half (Overview + Hostname + Option 12 + Port)
    await linuxPage.screenshot({ path: `${artifactDir}/screenshot_linux_network_tab.png` });
    await winPage.screenshot({ path: `${artifactDir}/screenshot_windows_network_tab.png` });

    // Scroll down to reveal Dedicated Virtual Adapter and Detected Interfaces
    await linuxPage.locator("#settings-content").evaluate((el: HTMLElement) => {
      el.scrollTop = el.scrollHeight;
    });
    await winPage.locator("#settings-content").evaluate((el: HTMLElement) => {
      el.scrollTop = el.scrollHeight;
    });

    await linuxPage.waitForTimeout(400);
    await winPage.waitForTimeout(400);

    // Screenshot bottom half (Virtual Adapter + Interface Cards)
    await linuxPage.screenshot({ path: `${artifactDir}/screenshot_linux_network_interfaces.png` });
    await winPage.screenshot({ path: `${artifactDir}/screenshot_windows_network_interfaces.png` });

    // Close settings dialogs
    await linuxPage.keyboard.press("Escape");
    await winPage.keyboard.press("Escape");
    await linuxPage.waitForSelector("#options-modal", { state: "hidden", timeout: 5000 });
    await winPage.waitForSelector("#options-modal", { state: "hidden", timeout: 5000 });

    console.log("Action 2 captured: Settings Dialog Network Tab");
  }, 30000);

  test("Action 3: Schedule & Slide Navigation (Live Presentation Output)", async () => {
    // Go live on schedule item 0, slide 0
    await fetch(`http://127.0.0.1:${LINUX_PORT}/api/command`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ GoLive: { item_index: 0, slide_index: 0 } })
    });
    await fetch(`http://127.0.0.1:${WIN_PORT}/api/command`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ GoLive: { item_index: 0, slide_index: 0 } })
    });

    await linuxPage.waitForTimeout(400);
    await winPage.waitForTimeout(400);

    // Advance to next slide (Chorus)
    await fetch(`http://127.0.0.1:${LINUX_PORT}/api/command`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ NextSlide: null })
    });
    await fetch(`http://127.0.0.1:${WIN_PORT}/api/command`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ NextSlide: null })
    });

    await linuxPage.waitForTimeout(500);
    await winPage.waitForTimeout(500);

    // Capture Live Slide Navigation Screenshots
    await linuxPage.screenshot({ path: `${artifactDir}/screenshot_linux_slide_nav.png` });
    await winPage.screenshot({ path: `${artifactDir}/screenshot_windows_slide_nav.png` });

    console.log("Action 3 captured: Schedule & Slide Navigation");
  }, 30000);

  test("Action 4: Desktop Remote QR Barcode Modal", async () => {
    // Click #btn-remote on Linux
    const linuxBtnRemote = linuxPage.locator("#btn-remote");
    await linuxBtnRemote.click();
    await linuxPage.waitForSelector("#remote-modal", { state: "visible", timeout: 5000 });
    await linuxPage.waitForSelector("#remote-qr-canvas", { state: "visible", timeout: 5000 });

    // Click #btn-remote on Windows
    const winBtnRemote = winPage.locator("#btn-remote");
    await winBtnRemote.click();
    await winPage.waitForSelector("#remote-modal", { state: "visible", timeout: 5000 });
    await winPage.waitForSelector("#remote-qr-canvas", { state: "visible", timeout: 5000 });

    // Give QR Canvas time to paint
    await linuxPage.waitForTimeout(400);
    await winPage.waitForTimeout(400);

    // Verify QR Canvas pixel rendering on both
    const checkQrCanvas = (canvas: HTMLCanvasElement) => {
      const ctx = canvas.getContext("2d");
      if (!ctx) return false;
      const imgData = ctx.getImageData(0, 0, canvas.width, canvas.height);
      let darkCount = 0;
      let lightCount = 0;
      for (let i = 0; i < imgData.data.length; i += 4) {
        if (imgData.data[i] < 50) darkCount++;
        else if (imgData.data[i] > 200) lightCount++;
      }
      return darkCount > 50 && lightCount > 50;
    };

    expect(await linuxPage.locator("#remote-qr-canvas").evaluate(checkQrCanvas)).toBe(true);
    expect(await winPage.locator("#remote-qr-canvas").evaluate(checkQrCanvas)).toBe(true);

    // Capture QR Modal Screenshots
    await linuxPage.screenshot({ path: `${artifactDir}/screenshot_linux_remote_qr.png` });
    await winPage.screenshot({ path: `${artifactDir}/screenshot_windows_remote_qr.png` });

    // Close remote modal
    await linuxPage.keyboard.press("Escape");
    await winPage.keyboard.press("Escape");
    await linuxPage.waitForSelector("#remote-modal", { state: "hidden", timeout: 5000 });
    await winPage.waitForSelector("#remote-modal", { state: "hidden", timeout: 5000 });

    console.log("Action 4 captured: Desktop Remote QR Barcode Modal");
  }, 30000);

  test("Action 5: DHCP Option 12 Broadcast Trigger & Conflict Validation", async () => {
    // Open Settings on Linux -> Network
    await linuxPage.keyboard.press("F2");
    await linuxPage.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await linuxPage.locator(".settings-category-btn:has-text('Network')").click();
    await linuxPage.waitForSelector("#btn-broadcast-option12", { state: "visible", timeout: 5000 });

    // Open Settings on Windows -> Network
    await winPage.keyboard.press("F2");
    await winPage.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await winPage.locator(".settings-category-btn:has-text('Network')").click();
    await winPage.waitForSelector("#btn-broadcast-option12", { state: "visible", timeout: 5000 });

    // Trigger Broadcast Option 12 Now on both
    await linuxPage.locator("#btn-broadcast-option12").click();
    await winPage.locator("#btn-broadcast-option12").click();

    // Verify broadcast badge appears with status on both
    await linuxPage.waitForFunction(() => {
      const badge = document.getElementById("network-broadcast-status");
      return badge && badge.style.display !== "none" && badge.textContent && badge.textContent.includes("Option 12");
    }, { timeout: 8000 });

    await winPage.waitForFunction(() => {
      const badge = document.getElementById("network-broadcast-status");
      return badge && badge.style.display !== "none" && badge.textContent && badge.textContent.includes("Option 12");
    }, { timeout: 8000 });

    if (!linuxPage.isClosed()) {
      await linuxPage.waitForTimeout(300);
      await linuxPage.screenshot({ path: `${artifactDir}/screenshot_linux_option12_broadcast.png` });
    }
    if (!winPage.isClosed()) {
      await winPage.waitForTimeout(300);
      await winPage.screenshot({ path: `${artifactDir}/screenshot_windows_option12_broadcast.png` });
    }

    console.log("Action 5 captured: DHCP Option 12 Broadcast Trigger & Conflict Validation");
  }, 30000);
});
