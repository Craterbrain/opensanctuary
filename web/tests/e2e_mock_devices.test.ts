import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { spawnTestServer, teardownTestServer, type SpawnedTestServer } from "./e2e_helpers";
import { MockConsole, sampleSong } from "./mock_devices/console_client";
import { MockTv, freshPrefs } from "./mock_devices/mock_tv";
import { MockPhone, pairTvByQr } from "./mock_devices/mock_phone";
import { peekCertificate } from "./mock_devices/pinned_tls";
import { Fleet } from "./mock_devices/fleet";
import { serveFleet } from "./mock_devices/fleet_server";

// Protocol-level conformance for the devices we can't run in CI: a mock Android TV and a mock phone speak
// exactly what apps/android-tv and web/src/remote_client.ts speak (docs/CLIENT_PAIRING.md), against a real
// server. The mocks keep the app's persistent preferences in an object so "reboot" and "reset" are one line.
describe("Mock devices: pairing, display and remote over the real protocol", () => {
  const HTTP = 9401;
  const HTTPS = 9402;
  let server: SpawnedTestServer;
  let consoleClient: MockConsole;
  const base = `http://127.0.0.1:${HTTP}`;
  const newTv = (name = "Mock Android TV") => new MockTv(freshPrefs("127.0.0.1", HTTPS), name);
  const live = async () => {
    await consoleClient.command({ AddToSchedule: sampleSong() });
    await consoleClient.command({ GoLive: { item_index: 0, slide_index: 0 } });
  };

  beforeAll(async () => {
    server = await spawnTestServer({ port: HTTP, tempPrefix: "os-next-mock-devices-", extraArgs: ["--https-port", String(HTTPS)] });
    consoleClient = await new MockConsole(base).connect();
  }, 45000);

  afterAll(async () => { await teardownTestServer(server); });

  test("a TV installed over ADB is only pinned: it still has to be paired by the phone's scan, then follows the show", async () => {
    const tv = newTv("ADB TV");
    // The ADB launch intent carries the cert fingerprint, so even the first request is a real verification.
    const cert = await peekCertificate("127.0.0.1", HTTPS);
    await tv.provisionPin(cert.fingerprint);
    expect((await tv.checkAndConnect()).kind).toBe("pairing-qr");
    expect((await tv.pollPairingStatus()).kind).toBe("waiting");
    expect(tv.deviceToken).toBeUndefined(); // installing and launching it paired nothing
    expect((await consoleClient.devices()).some((d) => d.id === tv.deviceId)).toBe(false);

    await pairTvByQr(consoleClient, tv, base);
    expect(tv.deviceToken).toStartWith("dev_tok_");
    expect(tv.prefs.paired_instance_id).toBe((await consoleClient.serverInfo()).instance_id);
    expect((await consoleClient.devices()).some((d) => d.id === tv.deviceId && d.platform === "android-tv")).toBe(true);

    expect((await tv.checkAndConnect()).kind).toBe("live");
    await tv.openLive();
    await live();
    const seq = (await tv.picture()).sequence_number;
    await tv.waitForSequence(seq);
    const pic = await tv.picture();
    expect(pic.live?.title).toBe("Mock Hymn");
    expect(pic.live?.current_slide_text).toContain("First line of verse one");
    tv.closeLive();
  }, 30000);

  test("manual-QR TV: shows its QR, waits, receives its token exactly once after a phone authorizes it", async () => {
    const tv = newTv("QR TV");
    const tech = new MockPhone(base);
    const session = await consoleClient.mintTvSession();

    expect((await tv.checkAndConnect()).kind).toBe("pairing-qr");
    expect(await tv.pollPairing()).toBeNull(); // nobody has scanned yet
    expect(tv.pinned).not.toBeNull(); // trust on first use pinned the cert

    const res = await tech.authorizeTv(tv.qrPayload(), session);
    expect(res.status).toBe(200);

    const token = await tv.pollPairing();
    expect(token).toStartWith("dev_tok_");
    expect(await tv.pollPairing()).toBeNull(); // the token is delivered once, not to every poll
    expect((await tv.checkAndConnect()).kind).toBe("live");

    // The session token is reusable (one console session pairs several TVs) but may not re-pair an existing id.
    const again = await tech.authorizeTv(tv.qrPayload(), session);
    expect(again.status).toBe(409);
  }, 30000);

  test("a TV that was reset while the console still lists it says so, and pairs again once the operator removes it", async () => {
    const tv = newTv("Reset TV");
    const tech = new MockPhone(base);
    expect((await tech.authorizeTv(tv.qrPayload(), await consoleClient.mintTvSession())).status).toBe(200);
    expect(await tv.pollPairing()).toStartWith("dev_tok_"); // paired by QR; its one token delivery is used up
    tv.clearPairing(); // Settings > Reset Pairing on the TV: its device id stays, the console's record does too

    expect((await tv.checkAndConnect()).kind).toBe("pairing-qr");
    const poll = await tv.pollPairingStatus();
    expect(poll.kind).toBe("already-registered");
    expect(MockTv.pairingMessage(poll.kind)).toBe("Remove this device from the console to pair again");

    // The phone is refused too, with the console's own wording.
    const session = await consoleClient.mintTvSession();
    const refused = await tech.authorizeTv(tv.qrPayload(), session);
    expect(refused.status).toBe(409);
    expect((await refused.json()).error).toContain("Remove it from the console first");

    // The operator removes it in Settings > Paired Devices: the TV goes back to waiting, and a scan works.
    expect((await consoleClient.revoke(tv.deviceId)).success).toBe(true);
    expect((await tv.pollPairingStatus()).kind).toBe("waiting");
    expect((await tech.authorizeTv(tv.qrPayload(), session)).status).toBe(200);
    expect(await tv.pollPairing()).toStartWith("dev_tok_");
    expect((await tv.checkAndConnect()).kind).toBe("live");
  }, 30000);

  test("a bad or missing session token never pairs a TV", async () => {
    const tv = newTv("Rogue TV");
    const tech = new MockPhone(base);
    expect((await tech.authorizeTv(tv.qrPayload(), "not-a-session")).status).toBe(401);
    expect(tv.deviceToken).toBeUndefined();
    const anon = await fetch(`${base}/api/pairing/session`, { method: "POST" });
    expect(anon.status).toBeGreaterThanOrEqual(401); // minting needs the host token
    expect((await tv.pollPairing())).toBeNull();
  }, 20000);

  test("phone remote: verifies its token, drives the show, and loses access the moment it is revoked", async () => {
    await live();
    const tv = newTv("Follower TV");
    await pairTvByQr(consoleClient, tv, base);
    await tv.openLive();

    const minted = await consoleClient.mintRemote();
    const phone = new MockPhone(base);
    phone.setCredential(minted.token, minted.device_id);
    expect((await phone.verify()).valid).toBe(true);
    await phone.connect();

    const before = (await tv.picture());
    phone.send("NextSlide");
    await phone.waitFor((s) => s.state?.live_slide_index === (before.live!.slide_index + 1), 5000, "the next slide");
    await tv.waitForSequence((await tv.picture()).sequence_number);
    const after = await tv.picture();
    expect(after.live!.slide_index).toBe(before.live!.slide_index + 1);
    // Both views of the same show agree: what the display socket last heard is what the console serves.
    expect(tv.lastSequence).toBe(after.sequence_number);

    // Overrides land on the TV too.
    phone.send("ToggleBlackout");
    await phone.waitFor((s) => s.state?.is_blackout === true, 5000, "blackout");
    expect((await tv.picture()).is_blackout).toBe(true);
    phone.send("ToggleBlackout");
    await phone.waitFor((s) => s.state?.is_blackout === false, 5000, "blackout off");

    await consoleClient.revoke(minted.device_id);
    expect((await phone.verify()).valid).toBe(false);
    const seqBefore = (await tv.picture()).sequence_number;
    phone.send("NextSlide"); // the revoked phone's socket may still be open; the server must ignore it
    const rest = await phone.sendRest("NextSlide");
    expect(rest.success).toBe(false);
    await Bun.sleep(400);
    expect((await tv.picture()).sequence_number).toBe(seqBefore);

    // A command with no credential at all, over the socket, is not the console's either.
    phone.token = "";
    phone.send("NextSlide");
    await Bun.sleep(300);
    expect((await tv.picture()).sequence_number).toBe(seqBefore);
    phone.disconnect(); tv.closeLive();
  }, 40000);

  test("a server restart keeps the TV paired: same certificate, same console identity", async () => {
    const tv = newTv("Restart TV");
    await pairTvByQr(consoleClient, tv, base);
    const identity = (await consoleClient.serverInfo()).instance_id;
    const pin = tv.pinned;
    expect(pin).not.toBeNull();

    server.proc.kill();
    await server.proc.exited;
    server = await spawnTestServer({ port: HTTP, reuseDir: server.testDir, extraArgs: ["--https-port", String(HTTPS)] });
    consoleClient = await new MockConsole(base).connect();

    // "Reboot" the TV: same preferences, new process.
    const rebooted = new MockTv(tv.prefs, "Restart TV");
    expect((await consoleClient.serverInfo()).instance_id).toBe(identity);
    expect((await rebooted.checkAndConnect()).kind).toBe("live");
    expect(rebooted.pinned).toBe(pin!); // the pinned certificate still verifies: no "regenerated on restart"
    await rebooted.openLive();
    rebooted.closeLive();
  }, 60000);

  test("a different console on the same address: a pinned TV refuses it, an unpinned one reports the wrong console", async () => {
    const tv = newTv("Church TV");
    await pairTvByQr(consoleClient, tv, base);
    const savedPrefs = structuredClone(tv.prefs);

    // Another install shows up at the same IP and port (a different data dir: new certificate, new identity).
    server.proc.kill();
    await server.proc.exited;
    server = await spawnTestServer({ port: HTTP, tempPrefix: "os-next-mock-devices-other-", extraArgs: ["--https-port", String(HTTPS)] });

    const pinned = new MockTv(structuredClone(savedPrefs), "Church TV");
    const screen = await pinned.checkAndConnect();
    expect(screen.kind).toBe("connection-failed");
    expect((screen as any).reason).toContain("does not match the pinned certificate");
    await expect(pinned.openLive()).rejects.toThrow(/pinned certificate/);

    // If the pin is gone (the operator retyped the address) the instance check is what catches it.
    const unpinned = new MockTv(structuredClone(savedPrefs), "Church TV");
    unpinned.setServer("127.0.0.1", HTTPS);
    const wrong = await unpinned.checkAndConnect();
    expect(wrong.kind).toBe("wrong-console");

    // "Reset Pairing" is the way out: the TV shows its QR again and can pair with the new console.
    unpinned.clearPairing();
    expect((await unpinned.checkAndConnect()).kind).toBe("pairing-qr");
    const newConsole = await new MockConsole(base).connect();
    await pairTvByQr(newConsole, unpinned, base);
    expect((await unpinned.checkAndConnect()).kind).toBe("live");
  }, 60000);
});

// The same devices, driven over the fleet control server a local agent uses.
describe("Mock devices: fleet control server", () => {
  const HTTP = 9403;
  const HTTPS = 9404;
  const CONTROL = 9405;
  let server: SpawnedTestServer;
  let fleet: Fleet;
  let control: ReturnType<typeof serveFleet>;
  const call = async (path: string, body?: unknown) => (await fetch(`http://127.0.0.1:${CONTROL}${path}`, body === undefined ? {} : { method: "POST", body: JSON.stringify(body) })).json() as Promise<any>;

  beforeAll(async () => {
    server = await spawnTestServer({ port: HTTP, tempPrefix: "os-next-fleet-", extraArgs: ["--https-port", String(HTTPS)] });
    fleet = await new Fleet(`http://127.0.0.1:${HTTP}`, "127.0.0.1", HTTPS).start();
    control = serveFleet(fleet, CONTROL);
  }, 45000);

  afterAll(async () => { fleet?.closeAll(); control?.stop(true); await teardownTestServer(server); });

  test("an agent can pair TVs both ways, connect a remote, and the check agrees everyone shows the same show", async () => {
    await call("/tv/add", { name: "sanctuary" });
    await call("/tv/add", { name: "foyer" });
    expect((await call("/tv/pair", { name: "sanctuary", pinned: true })).kind).toBe("live");
    expect((await call("/tv/pair", { name: "foyer", pinned: false })).kind).toBe("live");
    await call("/tv/live", { name: "sanctuary" });
    await call("/tv/live", { name: "foyer" });
    await call("/remote/add", { name: "usher" });

    await fleet.console.command({ AddToSchedule: sampleSong() });
    await fleet.console.command({ GoLive: { item_index: 0, slide_index: 0 } });
    await call("/remote/send", { name: "usher", cmd: "NextSlide" });
    const ok = await call("/check", {});
    expect(ok.problems).toEqual([]);
    expect(ok.console.slide).toBe(1);

    // A TV that lost its socket is not "in sync": the check only judges connected displays, /state shows who is offline.
    await call("/tv/close", { name: "foyer" });
    await call("/remote/send", { name: "usher", cmd: "NextSlide" });
    expect((await call("/check", {})).ok).toBe(true);
    const state = await call("/state");
    expect(state.tvs.find((t: any) => t.name === "foyer").live_open).toBe(false);

    // Reboot keeps the pairing; reset drops it back to the QR screen.
    await call("/tv/reboot", { name: "sanctuary" });
    expect((await call("/tv/screen", { name: "sanctuary" })).kind).toBe("live");
    await call("/tv/reset", { name: "sanctuary" });
    expect((await call("/tv/screen", { name: "sanctuary" })).kind).toBe("pairing-qr");
    expect((await call("/tv/pair", { name: "nobody" })).error).toContain("no TV named");
  }, 40000);
});
