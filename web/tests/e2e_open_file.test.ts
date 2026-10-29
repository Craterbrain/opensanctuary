import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawn } from "bun";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join, resolve } from "path";
import { spawnTestServer, teardownTestServer, pollUntil, type SpawnedTestServer } from "./e2e_helpers";

// Double-clicking an OpenSanctuary file: the desktop starts the program with the file's path. A second start hands the file to the
// running program and quits; the running console opens it through the normal steps (so a copyrighted song still shows its advisory).
describe("E2E: opening a file from the desktop", () => {
  const PORT = 9473;
  const slide = (text: string) => ({ text, header: null, label: "Verse 1", background: null });
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  let files: string;
  const binary = resolve(import.meta.dir, "../../target/release/os-next");

  const tokenOf = async (port: number) => (await (await fetch(`http://127.0.0.1:${port}/api/internal/host-token`)).json()).host_token as string;
  const songs = async (port = PORT): Promise<any[]> => (await fetch(`http://127.0.0.1:${port}/api/songs`)).json();

  /** What a file manager does: start the program with the file's path, against the same library. */
  const launchWithFile = async (path: string, dbPath: string, port: number) => {
    const p = spawn([binary, "--headless", "--skip-first-time-setup", "--port", String(port), "--db-path", dbPath, "--web-dir", resolve(import.meta.dir, "../"), path], {
      stdout: "pipe", stderr: "pipe", env: { ...process.env, OS_NEXT_TLS_KEY_STORE: "file" },
    });
    const code = await Promise.race([p.exited, new Promise<number>((r) => setTimeout(() => { p.kill(); r(-1); }, 15000))]);
    return { code, out: await new Response(p.stdout).text() };
  };

  /** A bundle made by a throwaway library, so the file is real. */
  const makeSong = async (id: string, title: string, copyright: string, ccli: string | null): Promise<string> => {
    const port = 9474;
    const tmp = await spawnTestServer({ port, tempPrefix: "os-open-src-" });
    try {
      const tok = await tokenOf(port);
      const H = { "Content-Type": "application/json", "x-host-token": tok, "x-console-session-id": "mk" };
      await fetch(`http://127.0.0.1:${port}/api/songs`, { method: "POST", headers: H, body: JSON.stringify({ id, title, author: "A. Writer", copyright, ccli_number: ccli, slides: [slide("Come thou fount of every blessing")] }) });
      const r = await fetch(`http://127.0.0.1:${port}/api/export/bundle`, { method: "POST", headers: H, body: JSON.stringify({ kind: "song", ids: [id] }) });
      const path = join(files, `${title}.ossong`);
      writeFileSync(path, Buffer.from(await r.arrayBuffer()));
      return path;
    } finally {
      await teardownTestServer(tmp);
    }
  };

  beforeAll(async () => {
    files = mkdtempSync(join(tmpdir(), "os-open-files-"));
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-open-main-" });
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1500, height: 1000 } });
    await page.goto(`http://127.0.0.1:${PORT}/`);
    await page.waitForFunction(() => (window as any).__APP_READY__ === true, null, { timeout: 20000 });
  }, 90000);
  afterAll(async () => { if (browser) await browser.close(); await teardownTestServer(server); });

  test("a second start with a public-domain file hands it over and quits; the running console opens it", async () => {
    const path = await makeSong("pd1", "Old Hymn", "Public Domain", null);
    expect(await songs()).toHaveLength(0);
    const started = Date.now();
    const { code, out } = await launchWithFile(path, server.dbPath, 9475);
    expect(code).toBe(0);
    expect(out).toContain("already running");
    expect(Date.now() - started).toBeLessThan(10000);
    const list = await pollUntil(songs, (s) => s.length === 1, 15000, "the hymn imported into the running library");
    expect(list[0].title).toBe("Old Hymn");
    // A second launch did not start a second server.
    expect(await fetch("http://127.0.0.1:9475/api/songs").then((r) => r.status, () => 0)).toBe(0);
  }, 60000);

  test("a copyrighted file shows the copyright pop-up first; Cancel adds nothing", async () => {
    const path = await makeSong("cr1", "New Anthem", "© 2020 Worship Co", "123456");
    const { code } = await launchWithFile(path, server.dbPath, 9475);
    expect(code).toBe(0);
    await page.waitForSelector("#bundle-advisory-modal", { state: "visible", timeout: 15000 });
    expect(await page.locator("#bundle-advisory-modal").innerText()).toContain("New Anthem");
    await page.click("#btn-cancel-bundle-advisory");
    await Bun.sleep(500);
    expect((await songs()).map((s) => s.title)).toEqual(["Old Hymn"]);
  }, 60000);

  test("a file that is not ours is not passed on, and the program starts normally", async () => {
    const junk = join(files, "notes.txt");
    writeFileSync(junk, "hello");
    const { code } = await launchWithFile(junk, server.dbPath, 9476);
    // Nothing to hand over, so it behaves as a normal second start (it runs on). Stop it.
    expect(code).toBe(-1);
  }, 30000);

  test("a file on the command line of the first start opens in that start's console", async () => {
    const path = await makeSong("pd2", "Another Hymn", "Public Domain", null);
    const b = await spawnTestServer({ port: 9477, tempPrefix: "os-open-first-", extraArgs: [path] });
    try {
      const p = await browser.newPage({ viewport: { width: 1500, height: 1000 } });
      await p.goto(`http://127.0.0.1:9477/`);
      await p.waitForFunction(() => (window as any).__APP_READY__ === true, null, { timeout: 20000 });
      const list = await pollUntil(() => songs(9477), (s) => s.length === 1, 15000, "the file given at start-up opened");
      expect(list[0].title).toBe("Another Hymn");
      await p.close();
    } finally {
      await teardownTestServer(b);
    }
  }, 60000);
});
