import { test, expect, afterAll, describe } from "bun:test";
import { chromium, type Browser } from "playwright";
import { spawn, type Subprocess } from "bun";
import { resolve, join } from "path";
import { mkdtempSync, rmSync } from "fs";
import { tmpdir } from "os";

/**
 * A remote console (a browser on another machine, reached over the HTTPS plane at the
 * server's LAN address) opens the printed `#host_token=` link once, trades it for a
 * persistent console credential, keeps working after the server restarts with a brand
 * new host token, and is deauthenticated the moment its device is removed from Paired
 * Devices. See docs/CLIENT_PAIRING.md "Persistent console credential".
 */
describe("E2E Live Test: remote console credential survives restarts and is revocable", () => {
  const PORT = 9071;
  const HTTPS_PORT = 9471;
  const testDir = mkdtempSync(join(tmpdir(), "os-next-console-cred-e2e-"));
  const dbPath = join(testDir, "test.db");
  const binary = resolve(import.meta.dir, "../../target/release/os-next");
  let proc: Subprocess | null = null;
  let browser: Browser | null = null;

  async function startServer(): Promise<{ origin: string; hostToken: string }> {
    proc = spawn(
      [binary, "--headless", "--skip-first-time-setup", "--port", String(PORT), "--https-port", String(HTTPS_PORT),
        "--db-path", dbPath, "--web-dir", resolve(import.meta.dir, "../")],
      { cwd: resolve(import.meta.dir, "../../"), stdout: "pipe", stderr: "ignore", env: { ...process.env, OS_NEXT_PRINT_TOKEN: "1", OS_NEXT_TLS_KEY_STORE: "file" } },
    );
    // The banner prints the token URL; read stdout until it appears.
    const reader = (proc.stdout as ReadableStream<Uint8Array>).getReader();
    const decoder = new TextDecoder();
    let text = "";
    const deadline = Date.now() + 20000;
    while (Date.now() < deadline) {
      const { value, done } = await reader.read();
      if (done) break;
      text += decoder.decode(value);
      const m = text.match(/Remote Console Access : (https?:\/\/[^\s/]+)\/#host_token=([0-9a-f-]+)/);
      if (m) {
        // Keep draining so the server never blocks on a full pipe.
        (async () => { while (!(await reader.read()).done) {} })();
        // The banner prints before the listeners are bound; wait until HTTPS answers.
        for (let i = 0; i < 100; i++) {
          try {
            // @ts-ignore bun-specific: self-signed cert
            await fetch(`${m[1]}/api/time`, { tls: { rejectUnauthorized: false } });
            break;
          } catch {
            await Bun.sleep(100);
          }
        }
        return { origin: m[1], hostToken: m[2] };
      }
    }
    throw new Error("server never printed its console URL:\n" + text);
  }

  async function stopServer() {
    if (!proc) return;
    proc.kill();
    await proc.exited;
    proc = null;
    await Bun.sleep(500);
  }

  afterAll(async () => {
    if (browser) await browser.close();
    await stopServer();
    rmSync(testDir, { recursive: true, force: true });
  });

  test("link once, survive a restart, deauthenticate on revoke", async () => {
    // --- Run 1: open the one-time link from a non-loopback origin. ---
    const run1 = await startServer();
    expect(run1.origin.startsWith("https://")).toBe(true);
    expect(run1.origin).not.toContain("127.0.0.1"); // a genuinely remote origin
    browser = await chromium.launch({ headless: true });
    const context = await browser.newContext({ ignoreHTTPSErrors: true });
    const page = await context.newPage();
    await page.goto(`${run1.origin}/#host_token=${run1.hostToken}`, { waitUntil: "networkidle" });

    await page.waitForFunction(() => !!localStorage.getItem("os_console_credential"), null, { timeout: 15000 });
    const credential = await page.evaluate(() => localStorage.getItem("os_console_credential"));
    expect(credential!.startsWith("console_tok_")).toBe(true);
    // The token was stripped from the address bar.
    expect(page.url()).not.toContain("host_token");

    const devicesStatus = async () =>
      page.evaluate(async () => {
        for (let i = 0; i < 50; i++) {
          const r = await fetch("/api/pairing/devices");
          if (r.status === 200) return 200;
          if (i === 49) return r.status;
          await new Promise((res) => setTimeout(res, 200));
        }
        return -1;
      });
    expect(await devicesStatus()).toBe(200);

    const listed = await page.evaluate(async () => (await fetch("/api/pairing/devices")).json());
    const consoleDevice = listed.find((d: any) => d.platform === "console");
    expect(consoleDevice).toBeDefined();
    expect(consoleDevice.token).toBe(""); // hashed at rest, never listed

    // --- Run 2: "restart" -- same library, new per-run host token. ---
    await stopServer();
    const run2 = await startServer();
    expect(run2.hostToken).not.toBe(run1.hostToken);
    // The old host token is dead...
    const stale = await fetch(`${run2.origin}/api/internal/verify-host-token`, {
      method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ token: run1.hostToken }),
      // @ts-ignore bun-specific: self-signed cert
      tls: { rejectUnauthorized: false },
    }).then((r) => r.json());
    expect(stale.valid).toBe(false);
    // ...yet the same browser, with no link and no fragment, is still a console.
    await page.goto(`${run2.origin}/`, { waitUntil: "networkidle" });
    expect(await devicesStatus()).toBe(200);

    // --- Revoke the device (the stolen-laptop path). ---
    const del = await fetch(`${run2.origin}/api/pairing/devices/${consoleDevice.id}`, {
      method: "DELETE", headers: { "x-host-token": run2.hostToken },
      // @ts-ignore bun-specific: self-signed cert
      tls: { rejectUnauthorized: false },
    });
    expect(del.ok).toBe(true);
    await page.goto(`${run2.origin}/`, { waitUntil: "networkidle" });
    await page.waitForFunction(() => !localStorage.getItem("os_console_credential"), null, { timeout: 15000 });
    const afterRevoke = await page.evaluate(async () => (await fetch("/api/pairing/devices")).status);
    expect(afterRevoke).toBe(401);
  }, 120000);
});
