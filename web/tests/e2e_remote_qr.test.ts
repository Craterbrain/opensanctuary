import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawnTestServer, teardownTestServer, ensureArtifactDir, type SpawnedTestServer } from "./e2e_helpers";

describe("E2E Live Test: Desktop Remote QR Barcode Modal", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  const PORT = 9035;
  let artifactDir: string;

  beforeAll(async () => {
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-next-remote-qr-e2e-" });
    artifactDir = ensureArtifactDir();

    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({
      viewport: { width: 1280, height: 800 }
    });
    page.on("pageerror", err => console.log("PAGE ERR:", err.message));
  }, 45000);

  afterAll(async () => {
    if (browser) await browser.close();
    await teardownTestServer(server);
  });

  test("Desktop console opens remote modal and generates a real QR barcode canvas", async () => {
    // 1. Verify GET /api/network/info returns LAN IP and remote URL
    const netRes = await fetch(`http://127.0.0.1:${PORT}/api/network/info`);
    expect(netRes.ok).toBe(true);
    const netData = await netRes.json();
    expect(netData.success).toBe(true);
    expect(typeof netData.lan_ip).toBe("string");
    expect(netData.remote_url).toContain("/remote");

    // 2. Load Desktop OpenSanctuary UI
    await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "networkidle" });

    // 3. Click #btn-remote in the top ribbon
    const btnRemote = page.locator("#btn-remote");
    expect(await btnRemote.isVisible()).toBe(true);
    await btnRemote.click();

    // 4. Verify #remote-modal backdrop is open
    const remoteModal = page.locator("#remote-modal");
    expect(await remoteModal.isVisible()).toBe(true);

    // 5. Verify the QR canvas has rendered
    const qrCanvas = page.locator("#remote-qr-canvas");
    expect(await qrCanvas.isVisible()).toBe(true);

    // Give QRCode canvas a brief tick to finish rendering pixels
    await page.waitForTimeout(300);

    // Verify canvas has non-empty pixel data
    const hasPixelData = await qrCanvas.evaluate((canvas: HTMLCanvasElement) => {
      const ctx = canvas.getContext("2d");
      if (!ctx) return false;
      const imgData = ctx.getImageData(0, 0, canvas.width, canvas.height);
      // Ensure there are both dark and light pixels (standard QR code)
      let darkCount = 0;
      let lightCount = 0;
      for (let i = 0; i < imgData.data.length; i += 4) {
        const r = imgData.data[i];
        if (r < 50) darkCount++;
        else if (r > 200) lightCount++;
      }
      return darkCount > 50 && lightCount > 50;
    });
    expect(hasPixelData).toBe(true);

    // 6. Verify URL display shows /remote
    const urlDisplay = page.locator("#remote-url-display");
    const displayedText = await urlDisplay.textContent();
    expect(displayedText).toContain("/remote");

    // 7. Verify copy button exists
    const btnCopy = page.locator("#btn-copy-remote-url");
    expect(await btnCopy.isVisible()).toBe(true);

    // 8. Capture visual artifact screenshot of the open modal with the QR code
    await page.screenshot({
      path: `${artifactDir}/screenshot_desktop_remote_qr_modal.png`
    });

    // 9. Verify the open link button points to /remote
    const openBtn = page.locator("#btn-open-mobile-remote");
    expect(await openBtn.isVisible()).toBe(true);
    const href = await openBtn.getAttribute("href");
    expect(href).toContain("/remote");
  }, 25000);
});
