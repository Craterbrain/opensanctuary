/**
 * Shared setup/teardown for the e2e test files under tests/e2e_*.test.ts
 * (and theme_picker.test.ts, which follows the same pattern): each one
 * spawns a real `target/release/os-next` server against an isolated temp
 * data dir, polls it until it accepts connections, drives it with a
 * headless Playwright browser, then tears both down. That whole sequence
 * used to be copy-pasted into all 12 files (with the exact same
 * server-readiness poll and the exact same hardcoded screenshot output
 * directory), which was both a maintenance hazard and hid the real cause of
 * these tests flaking specifically under the full suite (never in
 * isolation): the poll used `fetch()`, and several unrelated unit test
 * files elsewhere in this same suite temporarily replace `globalThis.fetch`
 * inside individual tests without narrow enough scoping (bun runs every
 * file in one process). If that happened to be installed at the moment one
 * of these polls fired, it silently ate the request instead of hitting the
 * real server, and the poll never recovered for the rest of its budget --
 * see `waitForServerReady`'s doc comment for the actual fix (a raw TCP
 * connect instead of `fetch`). Centralizing the whole sequence here means a
 * fix like that only has to happen once.
 */
import { spawn, type Subprocess } from "bun";
import { resolve, join } from "path";
import { mkdtempSync, rmSync, mkdirSync } from "fs";
import { tmpdir } from "os";

export interface SpawnedTestServer {
  proc: Subprocess;
  testDir: string;
  dbPath: string;
  port: number;
}

/**
 * Spawns the release binary headless against a fresh temp data dir and
 * waits until it accepts connections before returning. Throws (with the
 * server process already killed) if it never comes up within `timeoutMs`.
 * The binary itself starts in well under a second in practice, so the 10s
 * default is a generous margin on a genuinely hung/crashed server, not a
 * reflection of how long startup should normally take.
 */
export async function spawnTestServer(opts: {
  port: number;
  tempPrefix?: string;
  extraArgs?: string[];
  readyPath?: string;
  timeoutMs?: number;
  pollIntervalMs?: number;
}): Promise<SpawnedTestServer> {
  const testDir = mkdtempSync(join(tmpdir(), opts.tempPrefix ?? "os-next-e2e-"));
  const dbPath = join(testDir, "test.db");
  const binaryPath = resolve(import.meta.dir, "../../target/release/os-next");

  const proc = spawn(
    [
      binaryPath,
      "--headless",
      "--skip-first-time-setup",
      "--port", opts.port.toString(),
      "--db-path", dbPath,
      "--web-dir", resolve(import.meta.dir, "../"),
      ...(opts.extraArgs ?? []),
    ],
    {
      cwd: resolve(import.meta.dir, "../../"),
      stdout: "pipe",
      stderr: "pipe",
    }
  );

  try {
    await waitForServerReady(opts.port, {
      path: opts.readyPath,
      timeoutMs: opts.timeoutMs,
      pollIntervalMs: opts.pollIntervalMs,
    });
  } catch (e) {
    let stderrText = "";
    try {
      if (proc.stderr && typeof (proc.stderr as any).text === "function") {
        stderrText = await Promise.race([
          (proc.stderr as any).text(),
          new Promise<string>(r => setTimeout(() => r("(stderr read timed out)"), 2000)),
        ]);
      }
    } catch (_) {}
    try { proc.kill(); } catch (_) {}
    try { rmSync(testDir, { recursive: true, force: true }); } catch (_) {}
    throw new Error(`${(e as Error).message}${stderrText ? `\n--- server stderr ---\n${stderrText}` : "\n(no stderr captured)"}`);
  }

  return { proc, testDir, dbPath, port: opts.port };
}

/**
 * Polls `127.0.0.1:<port>` with a raw TCP connect until something accepts
 * the connection, or times out. Deliberately NOT `fetch()`: several unit
 * test files in this same suite temporarily replace `globalThis.fetch`
 * inside individual tests (see pairing.test.ts's own comment about this),
 * and bun runs the whole suite in one process -- if this poll's `fetch`
 * calls happened to land while an unrelated, concurrently-scheduled test
 * elsewhere had the global mocked, they'd silently hit that mock instead of
 * the real network, permanently reporting "not ready" for the rest of the
 * timeout regardless of how long it is. A raw socket connect can't be
 * affected by that -- it's a strictly lower-level check than HTTP anyway
 * ("is anything listening on this port" is the actual definition of
 * "started"), and it's what turned 12/12 e2e files failing under the full
 * suite into 0/12 once switched.
 */
export async function waitForServerReady(
  port: number,
  opts: { path?: string; timeoutMs?: number; pollIntervalMs?: number } = {}
): Promise<void> {
  const timeoutMs = opts.timeoutMs ?? 10000;
  const pollIntervalMs = opts.pollIntervalMs ?? 250;
  const deadline = Date.now() + timeoutMs;

  while (Date.now() < deadline) {
    if (await canConnect(port)) return;
    await new Promise(r => setTimeout(r, pollIntervalMs));
  }
  throw new Error(`Server on port ${port} failed to start in ${timeoutMs}ms`);
}

function canConnect(port: number): Promise<boolean> {
  return new Promise(resolve => {
    let settled = false;
    const finish = (ok: boolean) => {
      if (settled) return;
      settled = true;
      resolve(ok);
    };
    Bun.connect({
      hostname: "127.0.0.1",
      port,
      socket: {
        open(socket) {
          finish(true);
          try { socket.end(); } catch (_) {}
        },
        error() { finish(false); },
        data() {},
        close() {},
      },
    }).catch(() => finish(false));
  });
}

/** Kills the server, awaits its exit, and removes its temp data dir. */
export async function teardownTestServer(server: SpawnedTestServer | undefined | null): Promise<void> {
  if (!server) return;
  try {
    server.proc.kill();
    // A plain SIGTERM isn't always enough -- e2e_compare_windows_linux's
    // wine-wrapped Windows binary in particular can take longer to actually
    // exit after this than bun's default 5s hook timeout allows for the
    // whole `afterAll` (most of these files don't override it). Escalate to
    // SIGKILL after a bounded wait instead of waiting on `exited`
    // unconditionally, and keep the total well under 5s.
    await Promise.race([
      server.proc.exited,
      new Promise<void>(resolve => setTimeout(resolve, 2000)),
    ]);
    if (server.proc.exitCode === null && !server.proc.killed) {
      try { server.proc.kill("SIGKILL"); } catch (_) {}
      await Promise.race([
        server.proc.exited,
        new Promise<void>(resolve => setTimeout(resolve, 1500)),
      ]);
    }
  } catch (_) {}
  try { rmSync(server.testDir, { recursive: true, force: true }); } catch (_) {}
}

/**
 * Where e2e tests write their screenshots -- a repo-relative, gitignored
 * build-output directory. Previously hardcoded to a specific developer's
 * personal, unrelated-tool scratch directory
 * (`/home/<user>/.gemini/antigravity/brain/<session-id>`), which only ever
 * worked on the one machine/session that happened to have it, and left
 * screenshots piling up somewhere no one would think to look for them.
 */
export const E2E_ARTIFACT_DIR = resolve(import.meta.dir, "../../target/e2e-artifacts");

/** Ensures `E2E_ARTIFACT_DIR` exists and returns it. Call once per file, in `beforeAll`. */
export function ensureArtifactDir(): string {
  mkdirSync(E2E_ARTIFACT_DIR, { recursive: true });
  return E2E_ARTIFACT_DIR;
}
