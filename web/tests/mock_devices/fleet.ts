/**
 * A fleet of mock TVs and phones against one running server, plus the question a tester actually asks:
 * "after everything the console just did, does every device show what the console says is live?"
 * Used by the fleet control server (below) so a local agent can drive devices by name.
 */
import { MockConsole } from "./console_client.ts";
import { MockTv, freshPrefs, type TvPrefs } from "./mock_tv.ts";
import { MockPhone, pairTvByQr } from "./mock_phone.ts";
import { peekCertificate } from "./pinned_tls.ts";

export class Fleet {
  readonly tvs = new Map<string, MockTv>();
  readonly phones = new Map<string, MockPhone>();
  console!: MockConsole;

  constructor(public httpBase: string, public httpsHost: string, public httpsPort: number) {}

  async start() { this.console = await new MockConsole(this.httpBase).connect(); return this; }

  private tv(name: string) { const t = this.tvs.get(name); if (!t) throw new Error(`no TV named ${name}`); return t; }
  private phone(name: string) { const p = this.phones.get(name); if (!p) throw new Error(`no phone named ${name}`); return p; }

  addTv(name: string, prefs?: TvPrefs) {
    if (this.tvs.has(name)) throw new Error(`TV ${name} already exists`);
    this.tvs.set(name, new MockTv(prefs ?? freshPrefs(this.httpsHost, this.httpsPort), name));
    return { name, device_id: this.tv(name).deviceId };
  }

  /** The QR bridge. `pinned`: the TV was installed from the console over ADB, so it already knows the console's
   * certificate (a pin only: nothing is authorized by ADB). Otherwise its first connection is trust-on-first-use. */
  async pairTv(name: string, pinned = true) {
    const tv = this.tv(name);
    if (pinned) await tv.provisionPin((await peekCertificate(this.httpsHost, this.httpsPort)).fingerprint);
    const first = await tv.pollPairingStatus();
    if (first.kind === "already-registered") throw new Error(`TV ${name} shows "${MockTv.pairingMessage(first.kind)}": the console still lists it, so no scan can pair it`);
    await pairTvByQr(this.console, tv, this.httpBase);
    return this.tvScreen(name);
  }

  async tvScreen(name: string) { return this.tv(name).checkAndConnect(); }
  async tvOpenLive(name: string) { await this.tv(name).openLive(); return { ok: true }; }
  tvClose(name: string) { this.tv(name).closeLive(); return { ok: true }; }
  /** New process, same preferences. */
  tvReboot(name: string) {
    const old = this.tv(name); old.closeLive();
    this.tvs.set(name, new MockTv(old.prefs, name));
    return { ok: true };
  }
  tvResetPairing(name: string) { this.tv(name).clearPairing(); return { ok: true }; }
  tvSetServer(name: string, ip: string, port: number) { this.tv(name).setServer(ip, port); return { ok: true }; }

  /** The operator removes the TV in Settings > Paired Devices. */
  async removeTv(name: string) { const tv = this.tv(name); tv.closeLive(); return this.console.revoke(tv.deviceId); }

  async addRemote(name: string) {
    const minted = await this.console.mintRemote();
    const p = new MockPhone(this.httpBase);
    p.setCredential(minted.token, minted.device_id);
    await p.connect();
    this.phones.set(name, p);
    return { name, device_id: minted.device_id };
  }
  phoneSend(name: string, cmd: unknown) { this.phone(name).send(cmd); return { ok: true }; }
  async revoke(deviceId: string) { return this.console.revoke(deviceId); }
  async devices() { return this.console.devices(); }

  /** The oracle. Every connected TV must have heard the console's latest sequence and every phone must see it too. */
  async check(settleMs = 1500) {
    const truth = await (await fetch(`${this.httpBase}/api/display-state`)).json() as any;
    const deadline = Date.now() + settleMs;
    const problems: string[] = [];
    while (true) {
      problems.length = 0;
      for (const [name, tv] of this.tvs) {
        if (!tv.liveOpen) continue;
        if (tv.lastSequence !== truth.sequence_number) problems.push(`TV ${name} has sequence ${tv.lastSequence}, console is at ${truth.sequence_number}`);
      }
      for (const [name, p] of this.phones) {
        if (p.ws?.readyState !== WebSocket.OPEN) continue;
        const seq = p.lastSnapshot?.sequence_number;
        if (seq !== truth.sequence_number) problems.push(`phone ${name} has sequence ${seq}, console is at ${truth.sequence_number}`);
      }
      if (!problems.length || Date.now() >= deadline) break;
      await Bun.sleep(50);
    }
    return { ok: problems.length === 0, problems, console: { sequence: truth.sequence_number, live: truth.live?.title ?? null, slide: truth.live?.slide_index ?? null, blackout: truth.is_blackout } };
  }

  /** After a server restart: the console reconnects, TVs run their start-up check and reopen their display, phones reconnect. */
  async reconnectAll() {
    await this.start();
    const notes: string[] = [];
    for (const [name, tv] of this.tvs) {
      tv.closeLive();
      const screen = await tv.checkAndConnect();
      notes.push(`TV ${name}: ${screen.kind}`);
      if (screen.kind === "live") await tv.openLive().catch((e) => notes.push(`TV ${name} display failed: ${(e as Error).message}`));
    }
    for (const [name, p] of this.phones) {
      await p.connect().catch((e) => notes.push(`phone ${name} failed: ${(e as Error).message}`));
    }
    return notes;
  }

  closeAll() { for (const t of this.tvs.values()) t.closeLive(); for (const p of this.phones.values()) p.disconnect(); }
}
