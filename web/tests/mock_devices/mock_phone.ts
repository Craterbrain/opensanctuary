/**
 * A mock phone. It plays both roles a phone has in the pairing docs:
 *  - the volunteer's *remote* (/remote.html): a device token, /api/pairing/verify, and commands over /ws
 *    in the `{cmd, device_token}` envelope (web/src/core/ws_client.ts);
 *  - the technician's *scanner* (pairing.html): reads a TV's QR payload and POSTs /api/pairing/authorize.
 * Phones use the plain-HTTP port here; the transport is the same code path on the server.
 */
export class MockPhone {
  token = "";
  deviceId = "";
  ws: WebSocket | null = null;
  readonly received: any[] = [];
  rejected: any[] = [];

  constructor(public base: string) {}

  // --- remote role ---
  setCredential(token: string, deviceId = "") { this.token = token; this.deviceId = deviceId; }

  async verify(): Promise<{ valid: boolean; device_id?: string }> {
    return (await fetch(`${this.base}/api/pairing/verify`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ token: this.token }) })).json();
  }

  async connect(timeoutMs = 5000): Promise<void> {
    this.disconnect();
    const ws = new WebSocket(`${this.base.replace(/^http/, "ws")}/ws`);
    this.ws = ws;
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("no snapshot from /ws")), timeoutMs);
      ws.onerror = (e: any) => { clearTimeout(timer); reject(new Error(`ws error: ${e?.message ?? e}`)); };
      ws.onmessage = (m) => {
        let v: any; try { v = JSON.parse(String(m.data)); } catch { return; }
        if (v?.error) this.rejected.push(v); else this.received.push(v);
        if (this.received.length === 1) { clearTimeout(timer); resolve(); }
      };
    });
  }

  disconnect() { try { this.ws?.close(); } catch { /* closed */ } this.ws = null; }

  /** Sends a ShowCommand as a paired remote. Over WS there is no reply for success; watch snapshots. */
  send(cmd: unknown) {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) throw new Error("phone is not connected");
    this.ws.send(JSON.stringify({ cmd, device_token: this.token }));
  }

  /** The same command over REST with the device token (the fallback the web client uses). A refused
   * command is still HTTP 200, with `success: false` and the reason in `error`. */
  async sendRest(cmd: unknown): Promise<{ success: boolean; error?: string | null }> {
    const r = await fetch(`${this.base}/api/command`, { method: "POST", headers: { "Content-Type": "application/json", "x-device-token": this.token }, body: JSON.stringify(cmd) });
    return r.json();
  }

  get lastSnapshot(): any { return this.received.at(-1); }

  async waitFor(pred: (snapshot: any) => boolean, timeoutMs = 5000, what = "snapshot"): Promise<any> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const hit = [...this.received].reverse().find(pred);
      if (hit) return hit;
      await Bun.sleep(20);
    }
    throw new Error(`phone never saw ${what}`);
  }

  // --- technician / scanner role ---
  /** Scans a TV's pairing QR and authorizes it with the console-minted session token. */
  async authorizeTv(qr: { device_id: string; name: string; platform: string }, sessionToken: string) {
    return fetch(`${this.base}/api/pairing/authorize`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ session_token: sessionToken, ...qr }) });
  }
}

/** The whole QR bridge in one call: the console mints a session, the technician's phone scans the TV's code, the TV's
 * status poll collects its token. Returns the token. */
export async function pairTvByQr(console_: { mintTvSession(): Promise<string> }, tv: { qrPayload(): { device_id: string; name: string; platform: string }; pollPairing(): Promise<string | null> }, base: string): Promise<string> {
  const res = await new MockPhone(base).authorizeTv(tv.qrPayload(), await console_.mintTvSession());
  if (!res.ok) throw new Error(`authorize failed: HTTP ${res.status} ${await res.text()}`);
  const token = await tv.pollPairing();
  if (!token) throw new Error("the TV did not receive its token");
  return token;
}
