/**
 * A mock Android TV display: the same protocol as apps/android-tv (PairingManager.kt + MainActivity.kt)
 * without a TV. It keeps the app's persistent preferences in a plain object, so a test can "reboot" the
 * TV by constructing a new MockTv from the same prefs, or "factory reset" it with a fresh object.
 *
 * What the real app does, step by step, and the method that mirrors it:
 *   checkAndConnect()            MainActivity.checkAndConnect: no token -> QR + poll; instance mismatch ->
 *                                "Different Console Detected"; otherwise load live.html
 *   qrPayload() / pollPairing()  the two-way QR bridge: the only way a TV is ever paired (ADB install
 *                                gives it the console address and a certificate pin, never a token)
 *   clearPairing()               Settings > Reset Pairing
 *   openLive()                   the WebView's /ws connection to live.html (zero-auth)
 */
import { PinnedHttps, PinMismatchError } from "./pinned_tls.ts";

export interface TvPrefs {
  server_ip: string;
  server_port: number;
  device_id?: string;
  device_token?: string;
  paired_instance_id?: string;
  pinned_cert_fingerprint?: string;
}

export type TvScreen =
  | { kind: "pairing-qr"; qr: { device_id: string; name: string; platform: string } }
  | { kind: "wrong-console"; expected: string; actual: string }
  | { kind: "connection-failed"; reason: string }
  | { kind: "live" };

export interface TvPicture {
  sequence_number: number;
  is_blackout: boolean;
  is_clear_text: boolean;
  is_logo_override: boolean;
  alert_message: string | null;
  live: null | { title: string; slide_index: number; slide_count: number; current_slide_text: string; next_slide_text: string | null };
}

export function freshPrefs(serverIp: string, serverPort: number): TvPrefs {
  return { server_ip: serverIp, server_port: serverPort };
}

export class MockTv {
  readonly deviceName: string;
  private net: PinnedHttps;
  private ws: WebSocket | null = null;
  /** Every WS message the "WebView" has received, newest last (state snapshots). */
  readonly received: any[] = [];
  wsClosed = 0;
  wsOpened = 0;

  constructor(public prefs: TvPrefs, deviceName = "Mock Android TV") {
    this.deviceName = deviceName;
    this.net = new PinnedHttps(prefs.server_ip, prefs.server_port, prefs.pinned_cert_fingerprint ?? null);
  }

  // --- preferences, as the app persists them -------------------------------------------------------
  private sync() { if (this.net.pinned) this.prefs.pinned_cert_fingerprint = this.net.pinned; }
  get deviceId(): string { return (this.prefs.device_id ??= `mock-tv-${crypto.randomUUID()}`); }
  get deviceToken() { return this.prefs.device_token; }
  get pinned() { return this.net.pinned; }

  /** Settings: the operator typed a different console address. Drops the pin (not the credential). */
  setServer(ip: string, port: number) {
    this.prefs.server_ip = ip; this.prefs.server_port = port;
    delete this.prefs.pinned_cert_fingerprint;
    this.net = new PinnedHttps(ip, port, null);
  }

  /** Settings > Reset Pairing: credential, console identity and pin go together; device_id stays. */
  clearPairing() {
    delete this.prefs.device_token; delete this.prefs.paired_instance_id; delete this.prefs.pinned_cert_fingerprint;
    this.net = new PinnedHttps(this.prefs.server_ip, this.prefs.server_port, null);
  }

  /** The ADB launch intent's `cert_fingerprint` extra: a pin from a trusted channel. */
  async provisionPin(fingerprint: string) { await this.net.pinFromTrustedChannel(fingerprint); this.sync(); }

  // --- network calls the app makes -----------------------------------------------------------------
  async fetchInstanceId(): Promise<string> {
    const r = await this.net.json("/api/server-info");
    this.sync();
    return r.instance_id;
  }

  async fetchServerTimeMs(): Promise<number> {
    const r = await this.net.json("/api/time");
    this.sync();
    return r.server_time_ms;
  }

  private async persistPairing(token: string) {
    let instanceId: string | undefined;
    try { instanceId = await this.fetchInstanceId(); } catch { /* best effort, as in the app */ }
    this.prefs.device_token = token;
    if (instanceId) this.prefs.paired_instance_id = instanceId;
    this.sync();
  }

  /** What the TV renders as its pairing QR. */
  qrPayload() { return { device_id: this.deviceId, name: this.deviceName, platform: "android-tv" }; }

  /**
   * One poll of the pairing status, as `PairingManager.pollPairingStatus` reports it:
   *  - `paired`: a phone authorized this TV and the token arrived (delivered once);
   *  - `already-registered`: the console still lists this device id (e.g. after Reset Pairing on the TV) but won't
   *    hand the token out again, so the TV tells the operator to remove it in the console first;
   *  - `waiting`: nobody has scanned yet.
   */
  async pollPairingStatus(): Promise<{ kind: "paired"; token: string } | { kind: "already-registered" } | { kind: "waiting" }> {
    const r = await this.net.json(`/api/pairing/status?device_id=${encodeURIComponent(this.deviceId)}`);
    this.sync();
    if (!r.paired) return { kind: "waiting" };
    if (!r.token) return { kind: "already-registered" };
    await this.persistPairing(r.token);
    return { kind: "paired", token: r.token };
  }

  /** The text the TV shows on its pairing screen for a poll result (strings.xml). */
  static pairingMessage(kind: "paired" | "already-registered" | "waiting"): string {
    return kind === "already-registered" ? "Remove this device from the console to pair again" : "Scan to pair this display";
  }

  /** Returns the token the moment a phone has authorized this TV, else null. */
  async pollPairing(): Promise<string | null> {
    const r = await this.pollPairingStatus();
    return r.kind === "paired" ? r.token : null;
  }

  // --- the state machine -----------------------------------------------------------------------------
  async checkAndConnect(): Promise<TvScreen> {
    if (!this.prefs.device_token) return { kind: "pairing-qr", qr: this.qrPayload() };
    let actual: string;
    try {
      actual = await this.fetchInstanceId();
    } catch (e) {
      if (e instanceof PinMismatchError) return { kind: "connection-failed", reason: e.message };
      // Unreachable-to-check still loads the live URL in the app, which then fails and retries.
      return { kind: "connection-failed", reason: (e as Error).message };
    }
    const expected = this.prefs.paired_instance_id;
    if (expected && expected !== actual) return { kind: "wrong-console", expected, actual };
    return { kind: "live" };
  }

  // --- live.html -----------------------------------------------------------------------------------------
  /** Opens the display's WebSocket and resolves once the first state snapshot has arrived. */
  async openLive(timeoutMs = 5000): Promise<void> {
    this.closeLive();
    const ws = await this.net.webSocket("/ws");
    this.sync();
    this.ws = ws;
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("no snapshot from /ws")), timeoutMs);
      ws.onopen = () => { this.wsOpened++; };
      ws.onclose = () => { this.wsClosed++; };
      ws.onerror = (e: any) => { clearTimeout(timer); reject(new Error(`ws error: ${e?.message ?? e}`)); };
      ws.onmessage = (m) => {
        let v: any; try { v = JSON.parse(String(m.data)); } catch { return; }
        this.received.push(v);
        if (this.received.length === 1) { clearTimeout(timer); resolve(); }
      };
    });
  }

  closeLive() { try { this.ws?.close(); } catch { /* already closed */ } this.ws = null; }
  get liveOpen() { return this.ws?.readyState === WebSocket.OPEN; }

  /** The latest sequence number the display has heard about over the socket. */
  get lastSequence(): number {
    for (let i = this.received.length - 1; i >= 0; i--) if (typeof this.received[i]?.sequence_number === "number") return this.received[i].sequence_number;
    return -1;
  }

  /** The Roku way (and our oracle): what the console says should be on screen right now. */
  async picture(): Promise<TvPicture> {
    const r = await this.net.json("/api/display-state");
    this.sync();
    return r;
  }

  /** Waits until the display has received a snapshot at or past `sequence`. */
  async waitForSequence(sequence: number, timeoutMs = 5000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) { if (this.lastSequence >= sequence) return; await Bun.sleep(20); }
    throw new Error(`display never reached sequence ${sequence} (at ${this.lastSequence})`);
  }
}
