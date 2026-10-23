/**
 * The operator console, as far as the devices care: it holds the host token and is the only party that
 * can mint pairing sessions, remote tokens, list/revoke devices and drive the show.
 * (Real consoles are the browser UI; tests that need the UI use Playwright instead.)
 */
export class MockConsole {
  hostToken = "";
  readonly sessionId: string;

  /** `base` is the plain-HTTP loopback URL: only loopback hands out the host token. `sessionId` just has to
   * be unique enough to disambiguate concurrent MockConsoles in one test run (any non-empty value works per
   * `x-console-session-id`'s own contract, see e2e_remote_mobile.test.ts) -- `crypto.randomUUID()` purely
   * because there's no reason to reach for `Math.random()` even for a non-secret value. */
  constructor(public base: string, sessionId = `mock-console-${crypto.randomUUID().slice(0, 8)}`) {
    this.sessionId = sessionId;
  }

  async connect(): Promise<this> {
    const r = await (await fetch(`${this.base}/api/internal/host-token`)).json();
    this.hostToken = r.host_token;
    return this;
  }

  private headers() {
    return { "Content-Type": "application/json", "x-host-token": this.hostToken, "x-console-session-id": this.sessionId };
  }

  async serverInfo() { return (await fetch(`${this.base}/api/server-info`)).json() as Promise<any>; }

  async command(cmd: unknown) {
    const r = await fetch(`${this.base}/api/command`, { method: "POST", headers: this.headers(), body: JSON.stringify(cmd) });
    if (!r.ok) throw new Error(`command failed: HTTP ${r.status} ${await r.text()}`);
    return r.json().catch(() => ({}));
  }

  async mintTvSession(): Promise<string> {
    const r = await fetch(`${this.base}/api/pairing/session`, { method: "POST", headers: this.headers() });
    if (!r.ok) throw new Error(`session mint failed: HTTP ${r.status}`);
    return (await r.json()).session_token;
  }

  async mintRemote(): Promise<{ device_id: string; token: string }> {
    const r = await (await fetch(`${this.base}/api/pairing/remote-session`, { method: "POST", headers: this.headers() })).json();
    if (!r.success) throw new Error(`remote mint failed: ${JSON.stringify(r)}`);
    return r;
  }

  async devices(): Promise<any[]> {
    const r = await fetch(`${this.base}/api/pairing/devices`, { headers: this.headers() });
    if (!r.ok) throw new Error(`devices failed: HTTP ${r.status}`);
    return r.json();
  }

  async revoke(id: string) {
    return (await fetch(`${this.base}/api/pairing/devices/${encodeURIComponent(id)}`, { method: "DELETE", headers: this.headers() })).json();
  }
}

/** A song with three short slides, enough for navigation checks. */
export function sampleSong(id = "mock_song_1", title = "Mock Hymn") {
  const slide = (label: string, tag: string, text: string) => ({ text, header: null, label, tag, background: "#102027", notes: null, elements: [] });
  return {
    id, title, subtitle: null, notes: null, item_type: "song", author_or_ref: "Test Author",
    slides: [
      slide("Verse 1", "V1", "First line of verse one\nSecond line of verse one"),
      slide("Chorus", "C", "Chorus line one\nChorus line two"),
      slide("Verse 2", "V2", "First line of verse two\nSecond line of verse two"),
    ],
    background: null, theme_name: null, is_expanded: false, is_section_header: false, arrangement: [],
  };
}
