import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { spawnTestServer, teardownTestServer, type SpawnedTestServer } from "./e2e_helpers";
import { MockConsole, sampleSong } from "./mock_devices/console_client";

/**
 * Deterministic concurrency tests for the pairing endpoints -- no model, no browser, just many
 * real requests fired at once against a real server. These target the TOCTOU shape every
 * check-then-act over `Database.main_conn` (a single `Arc<Mutex<Connection>>`, see
 * src/storage/db/core.rs) can have: each DB call locks and releases the mutex on its own, so two
 * calls in one handler (a SELECT, then later an INSERT/UPDATE) are NOT atomic with each other --
 * another concurrent handler can run its own SELECT in the gap between them.
 *
 * Unlike the Qwen-driven exploration (slow, non-deterministic, costly to supervise), these are
 * cheap, fast, and exercise the exact race window the code's shape suggests, every run.
 */
describe("Pairing endpoints under real concurrency", () => {
  const HTTP = 9420;
  let server: SpawnedTestServer;
  let consoleClient: MockConsole;
  const base = `http://127.0.0.1:${HTTP}`;

  beforeAll(async () => {
    server = await spawnTestServer({ port: HTTP, tempPrefix: "os-next-pairing-races-" });
    consoleClient = await new MockConsole(base).connect();
  }, 45000);

  afterAll(async () => { await teardownTestServer(server); });

  test("racing authorize() for the SAME new device id: at most one call may actually win the registration", async () => {
    // A real session is reusable for several devices by design (one console session pairs a whole
    // building) -- so anyone who legitimately has it can call authorize for an unregistered id
    // repeatedly. If two concurrent calls for the SAME id can both report success, the loser's
    // response handed its caller a token that doesn't work (the winner's overwrote it in storage) --
    // and worse, whichever call's token ends up actually stored is now known to BOTH callers, since
    // each one generated and was told its own candidate token before the race was resolved.
    const session = await consoleClient.mintTvSession();
    const deviceId = `race_tv_${crypto.randomUUID()}`;
    const N = 12;
    const authorize = () => fetch(`${base}/api/pairing/authorize`, {
      method: "POST", headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ session_token: session, device_id: deviceId, name: "Race TV", platform: "android-tv" }),
    }).then(async (r) => ({ status: r.status, body: await r.json().catch(() => ({})) }));

    const results = await Promise.all(Array.from({ length: N }, authorize));
    const succeeded = results.filter((r) => r.status === 200 && r.body.success === true);
    const conflicted = results.filter((r) => r.status === 409);

    // What's actually stored, from the console's own (host-token-gated) view -- the ground truth.
    const devices = await consoleClient.devices();
    const stored = devices.find((d: any) => d.id === deviceId);
    expect(stored, "the device should exist after at least one authorize succeeded").toBeDefined();

    if (succeeded.length > 1) {
      // This is the race actually firing: more than one caller was told "success", each with its
      // own distinct token, but only one of those tokens can be the one now in storage.
      const distinctTokens = new Set(succeeded.map((r) => r.body.token));
      const matchingStored = succeeded.filter((r) => r.body.token === stored!.token);
      throw new Error(
        `authorize() let ${succeeded.length}/${N} concurrent calls for the same new device id "${deviceId}" ` +
        `all report success (expected at most 1; ${conflicted.length} got 409). ${distinctTokens.size} distinct ` +
        `tokens were handed out to callers, but storage holds only one -- ${matchingStored.length} of the ` +
        `"successful" callers actually got a token that will work, and ${succeeded.length - matchingStored.length} ` +
        `were told they succeeded while holding a token nothing will ever accept again. Every caller who got a ` +
        `"success" response also independently knows the token value it generated, including whichever one lost ` +
        `the write race -- so a second party racing a legitimate pairing can end up knowing the real device's ` +
        `working token even without winning the race itself, if their own losing write still matches by chance, ` +
        `or more directly if THEIRS is the one that wins.`
      );
    }
    expect(succeeded.length, "exactly one authorize() for a given new device id should ever succeed").toBe(1);
    expect(succeeded[0].body.token).toBe(stored!.token);
  }, 30000);

  test("racing a device's own two status polls right after pairing: the one-time token delivery must stay one-time", async () => {
    const session = await consoleClient.mintTvSession();
    const deviceId = `race_poll_${crypto.randomUUID()}`;
    const authRes = await (await fetch(`${base}/api/pairing/authorize`, {
      method: "POST", headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ session_token: session, device_id: deviceId, name: "Poll Race TV", platform: "android-tv" }),
    })).json() as any;
    expect(authRes.success).toBe(true);

    const N = 20;
    const poll = () => fetch(`${base}/api/pairing/status?device_id=${deviceId}`).then((r) => r.json());
    const results = await Promise.all(Array.from({ length: N }, poll));
    const withToken = results.filter((r: any) => r.paired && r.token);
    if (withToken.length > 1) {
      throw new Error(`${withToken.length}/${N} concurrent status polls for the same freshly-paired device all received its token (expected exactly 1) -- a second poller racing the real device's first poll could walk away with its token.`);
    }
    expect(withToken.length).toBe(1);
    expect(withToken[0].token).toBe(authRes.token);

    // And a slower, sequential re-poll afterward must still never see it again.
    const again = await (await fetch(`${base}/api/pairing/status?device_id=${deviceId}`)).json() as any;
    expect(again.token).toBeFalsy();
  }, 20000);

  test("racing remote-session minting: every minted token is independently valid and none interferes with another", async () => {
    // Not a check-then-act on a shared key here (device_id is generated fresh server-side, not
    // caller-supplied) -- this instead checks that concurrent, legitimate mints never produce
    // colliding ids or tokens, and that revoking one mid-mint-storm never touches the others.
    const N = 15;
    const mints = await Promise.all(Array.from({ length: N }, () => consoleClient.mintRemote()));
    const ids = new Set(mints.map((m) => m.device_id));
    const tokens = new Set(mints.map((m) => m.token));
    expect(ids.size).toBe(N);
    expect(tokens.size).toBe(N);

    const verifyAll = await Promise.all(mints.map((m) => fetch(`${base}/api/pairing/verify`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ token: m.token }) }).then((r) => r.json())));
    expect(verifyAll.every((v: any) => v.valid)).toBe(true);

    // Revoke half of them while the test harness concurrently re-verifies ALL of them; the
    // untouched half must never flicker invalid, and the revoked half must never verify as valid
    // again afterward (sequential re-check, since "valid right up until the response lands" is
    // expected -- only *after* revoke's own response is the guarantee).
    const [toRevoke, toKeep] = [mints.slice(0, 7), mints.slice(7)];
    await Promise.all(toRevoke.map((m) => consoleClient.revoke(m.device_id)));
    const keptChecks = await Promise.all(toKeep.map((m) => fetch(`${base}/api/pairing/verify`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ token: m.token }) }).then((r) => r.json())));
    expect(keptChecks.every((v: any) => v.valid), "revoking one device must never invalidate an unrelated one").toBe(true);
    const revokedChecks = await Promise.all(toRevoke.map((m) => fetch(`${base}/api/pairing/verify`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ token: m.token }) }).then((r) => r.json())));
    expect(revokedChecks.every((v: any) => !v.valid)).toBe(true);
  }, 30000);

  test("a command racing its own device token's revocation: nothing accepted with that token succeeds once revoke has returned", async () => {
    const song = sampleSong("race_song", "Race Hymn");
    await consoleClient.command({ AddToSchedule: song });
    await consoleClient.command({ GoLive: { item_index: 0, slide_index: 0 } });
    const minted = await consoleClient.mintRemote();
    const sendNext = () => fetch(`${base}/api/command`, { method: "POST", headers: { "Content-Type": "application/json", "x-device-token": minted.token }, body: JSON.stringify("NextSlide") }).then((r) => r.json());

    expect((await sendNext()).success).toBe(true); // the token genuinely works before revocation
    await consoleClient.revoke(minted.device_id); // revoke's own response has returned by the time this resolves

    // Fire a burst with the now-revoked token; every single one must be refused -- not "mostly".
    const after = await Promise.all(Array.from({ length: 10 }, sendNext));
    expect(after.every((r: any) => r.success === false), "a revoked device token must never succeed, even under a concurrent burst").toBe(true);
  }, 20000);
});
