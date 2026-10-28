/**
 * Runs sandboxed web plugins (docs/PLUGIN_PERMISSIONS.md, phase 4).
 *
 * Each plugin lives in an iframe with `sandbox="allow-scripts"` and no `allow-same-origin`: an
 * opaque origin with no access to this page, its storage, the host token or the console
 * credential. Its CSP (set by the server) also forbids network access beyond the hosts it was
 * granted. The only way out is the message bridge below, which is the *only* code holding the
 * plugin's scoped token and which checks each request against what the operator granted.
 * The server re-checks commands and keyring access against the token, so these client-side
 * checks are a second line, not the only one.
 *
 * Protocol (plugin -> console): `{ type: 'os.request', id, method, params }`
 *   getState                      show.observe
 *   sendCommand { command }       commands.send (only the granted command names)
 *   keyring.get|set|delete { service?, account, secret? }  keyring.own (the plugin's own namespace), or keyring.service
 *                                 with `service` set to a granted shared namespace (`OpenSanctuary:Shared:<name>`);
 *                                 set and delete there need `write` access
 *   net.fetch { url, headers? }   network.outbound: the *server* fetches it, only from granted hosts (headers: Authorization,
 *                                 Accept, Accept-Language, and any the plugin's manifest declared under network.outbound.headers)
 *   library.addSong { song }      library.write on "songs"; refreshes the library list
 *   library.setRemoteBibles { bibles } / library.getRemoteBibles   library.write on "scriptures": the translations this plugin offers in the
 *                                 Scriptures sidebar (metadata only; text is fetched when a passage is added to the schedule)
 *   library.refreshLicensed { ids, ttlSecs } library.write on "scriptures": renews the lifetime of licensed passages this plugin stored
 *   library.addScriptures { items } library.write on "scriptures": Bible chapters (an item may carry `licensed`, see src/storage/db/licensed.rs); refreshes the Bibles
 *   process.start|poll|cancel|release   process.run: run one of the plugin's manifest-declared commands
 *   tool.status|update            process.run: the host-managed tool behind those commands
 *   library.addMedia { job, file, ... } library.write on "media": a finished file from a job's folder
 *   ui.pasteText                  ui.clipboard: the clipboard's text (for a URL box)
 *   ui.theme                      the console's design tokens (--os-*) and icon set, so a page can look native
 *   ui.resize { height }          ui.panel
 *   ui.toolbar.update { id, disabled?, pressed? } ui.toolbar: on/off and pressed state of one of the plugin's own ribbon
 *                                 buttons (declared in plugin.json; label and icon can't change at runtime)
 *   log { message }
 * console -> plugin: `{ type: 'os.response', id, ok, result|error }`
 *   and, with ui.toolbar, `{ type: 'os.event', event: 'toolbar', data: { id } }` when one of its ribbon buttons is clicked.
 *   and, with show.observe, `{ type: 'os.event', event: 'state', data }` for each state update.
 */
import type { StateSnapshot } from './models.ts';
import { errorMessage } from './ui_utils.ts';
import { api, type PluginInfo } from "./api_client.ts";
import { getCurrentIconSet } from "./icon_sets.ts";
import { registerLibrarySource, unregisterLibrarySource } from "../ui/library_panel.ts";
import { addToolbarItems, removeToolbarItems, updateToolbarItem } from "./plugin_toolbar.ts";
import { syncVideoPickerTab } from "./plugin_hints.ts";
import { silentFrames, PING_EVERY_MS, LoadShedder, type Watched } from "./plugin_guard.ts";
import { showToast } from "./ui_utils.ts";

interface Running {
  plugin: PluginInfo;
  frame: HTMLIFrameElement;
  granted: Set<string>;
  commands: string[];
  token: string | null;
  /** Requests from this frame still being handled (see MAX_IN_FLIGHT). */
  inFlight: number;
}

/**
 * Most bridge requests one plugin may have outstanding at once. Each request can start real work
 * (a fetch, a JSON parse, a reply), so without a cap one frame posting 20,000 `getState`s queues
 * 20,000 fetches on the console's own thread and keeps it busy for seconds after the flood stops.
 * Past the cap a request is refused straight away, which costs next to nothing. The bundled
 * plugins peak at about 5 (Online Bibles' download workers).
 */
export const MAX_IN_FLIGHT = 32;

function grantedList(p: PluginInfo, id: string, field: 'hosts' | 'categories' | 'commands' | 'headers'): string[] {
  return p.granted.filter(g => g.id === id).flatMap(g => (g[field] as string[] | undefined) ?? []);
}

const running = new Map<Window, Running>();
/** Liveness of each running frame, for the watchdog (plugin_guard.ts). */
const watched = new Map<Window, Watched>();
let watchdogTimer: number | null = null;
const shedder = new LoadShedder();
/** Plugins the shedder switched off to test them, so the innocent ones can be started again. */
const shedPlugins = new Map<string, PluginInfo>();

/** Once a second: is the console running late, and if so which plugin is the cause (see LoadShedder)? */
function startLoadShedding() {
  let expected = Date.now() + 1000;
  window.setInterval(() => {
    const now = Date.now();
    const drift = Math.max(0, now - expected);
    expected = now + 1000;
    const order: string[] = [];
    const info = new Map<string, PluginInfo>();
    for (const r of running.values()) if (!info.has(r.plugin.id)) { info.set(r.plugin.id, r.plugin); order.push(r.plugin.id); }
    for (const act of shedder.tick(drift, order)) {
      if (act.kind === 'suspend') {
        const plugin = info.get(act.id);
        if (plugin) { shedPlugins.set(act.id, plugin); stopSandboxedPlugin(act.id); }
      } else if (act.kind === 'restore') {
        for (const id of act.ids) { const plugin = shedPlugins.get(id); shedPlugins.delete(id); if (plugin && !isSandboxedPluginRunning(id)) startSandboxedPlugin(plugin); }
      } else if (act.kind === 'blame') {
        const name = shedPlugins.get(act.id)?.name ?? act.id;
        shedPlugins.delete(act.id);
        showToast(`Plugin "${name}" was slowing the console down and was turned off for this session. It is still approved; reload to start it again.`, 'error', 15000);
      }
    }
  }, 1000);
}

function startWatchdog() {
  if (watchdogTimer !== null) return;
  startLoadShedding();
  let expected = Date.now() + PING_EVERY_MS;
  watchdogTimer = window.setInterval(() => {
    const now = Date.now();
    const lagged = now - expected > 3000; // the console itself was stalled: nobody is judged this round
    expected = now + PING_EVERY_MS;
    for (const [win] of watched) {
      try { win.postMessage({ type: 'os.ping', n: now }, '*'); } catch { /* frame gone */ }
      if (!running.has(win)) watched.delete(win);
    }
    for (const dead of silentFrames(watched.values(), now, lagged)) {
      const name = Array.from(running.values()).find((r) => r.plugin.id === dead.id)?.plugin.name ?? dead.id;
      stopSandboxedPlugin(dead.id);
      showToast(`Plugin "${name}" stopped responding and was turned off for this session. It is still approved; reload to start it again.`, 'error', 15000);
    }
  }, PING_EVERY_MS);
}
/** The one frame of a plugin that receives its ribbon-button clicks (so two placements don't both act). */
const primaryFrame = new Map<string, HTMLIFrameElement>();

/** The console's design tokens (the --os-* custom properties), as currently computed. */
function collectThemeVars(): Record<string, string> {
  const names = new Set<string>();
  for (const sheet of Array.from(document.styleSheets)) {
    let rules: CSSRuleList;
    try { rules = sheet.cssRules; } catch { continue; } // a cross-origin sheet
    for (const rule of Array.from(rules)) {
      const style = (rule as CSSStyleRule).style;
      const sel = (rule as CSSStyleRule).selectorText || '';
      if (!style || !/(^|[\s,]):root|html|body|\[data-/.test(sel)) continue;
      for (let i = 0; i < style.length; i++) {
        const prop = style[i];
        if (prop && prop.startsWith('--os-')) names.add(prop);
      }
    }
  }
  const computed = getComputedStyle(document.documentElement);
  const vars: Record<string, string> = {};
  names.forEach((n) => {
    const v = computed.getPropertyValue(n).trim();
    if (v) vars[n] = v;
  });
  return vars;
}

function pushTheme(win?: Window) {
  const vars = collectThemeVars();
  const targets = win ? [win] : Array.from(running.keys());
  const iconSet = getCurrentIconSet();
  targets.forEach((w) => { try { w.postMessage({ type: 'os.theme', vars, iconSet }, '*'); } catch { /* frame gone */ } });
}

let themeObserver: MutationObserver | null = null;
function watchTheme() {
  if (themeObserver || typeof MutationObserver === 'undefined') return;
  let timer: ReturnType<typeof setTimeout> | null = null;
  themeObserver = new MutationObserver(() => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => pushTheme(), 120);
  });
  const opts = { attributes: true, attributeFilter: ['style', 'class', 'data-theme', 'data-icon-set'] };
  themeObserver.observe(document.documentElement, opts);
  themeObserver.observe(document.body, opts);
}
let listenerInstalled = false;
let stateSocket: WebSocket | null = null;

function grantedCommands(p: PluginInfo): string[] {
  return p.granted.filter(g => g.id === 'commands.send').flatMap(g => g.commands ?? []);
}

function commandName(cmd: unknown): string | null {
  if (typeof cmd === 'string') return cmd;
  if (cmd && typeof cmd === 'object') {
    const keys = Object.keys(cmd as object);
    if (keys.length === 1) return keys[0] ?? null;
  }
  return null;
}

async function mintToken(r: Running): Promise<string | null> {
  const scopes: Array<{ id: string; commands?: string[]; hosts?: string[]; headers?: string[]; categories?: string[]; services?: string[]; access?: string }> = [];
  if (r.granted.has('keyring.own')) scopes.push({ id: 'keyring.own' });
  if (r.granted.has('commands.send')) scopes.push({ id: 'commands.send', commands: r.commands });
  if (r.granted.has('network.outbound')) scopes.push({ id: 'network.outbound', hosts: grantedList(r.plugin, 'network.outbound', 'hosts'), headers: grantedList(r.plugin, 'network.outbound', 'headers') });
  if (r.granted.has('library.write')) scopes.push({ id: 'library.write', categories: grantedList(r.plugin, 'library.write', 'categories') });
  const shared = r.plugin.granted.filter((g) => g.id === 'keyring.service');
  if (shared.length) {
    scopes.push({ id: 'keyring.service', services: shared.flatMap((g) => g.services ?? []), access: shared.some((g) => g.access === 'write') ? 'write' : 'read' });
  }
  if (r.granted.has('process.run')) scopes.push({ id: 'process.run', commands: grantedList(r.plugin, 'process.run', 'commands') });
  if (r.granted.has('notifications.post')) scopes.push({ id: 'notifications.post' });
  if (!scopes.length) return null;
  try {
    return (await api.plugins.token(r.plugin.id, scopes)).token;
  } catch (e) {
    console.warn(`[PluginBridge] couldn't get a token for ${r.plugin.id}:`, e);
    return null;
  }
}

async function withToken(r: Running, call: (token: string) => Promise<Response>): Promise<Response> {
  if (!r.token) r.token = await mintToken(r);
  if (!r.token) throw new Error('no token');
  let res = await call(r.token);
  if (res.status === 401) {
    // Expired (they last 12 hours): mint a fresh one once.
    r.token = await mintToken(r);
    if (r.token) res = await call(r.token);
  }
  return res;
}

async function handle(r: Running, method: string, params: any): Promise<unknown> {
  const need = (perm: string) => {
    if (!r.granted.has(perm)) throw new Error(`permission '${perm}' not granted`);
  };
  switch (method) {
    case 'getState': {
      need('show.observe');
      const res = await fetch('/api/state');
      return res.json();
    }
    case 'sendCommand': {
      need('commands.send');
      const name = commandName(params?.command);
      if (!name || !(r.commands.includes('*') || r.commands.includes(name))) {
        throw new Error(`sending '${name ?? '?'}' not granted`);
      }
      const res = await withToken(r, (t) => fetch('/api/command', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'x-plugin-token': t },
        body: JSON.stringify(params.command),
      }));
      return res.json();
    }
    case 'keyring.get':
    case 'keyring.set':
    case 'keyring.delete': {
      const op = method.split('.')[1];
      const own = `OpenSanctuary:Plugin:${r.plugin.id}`;
      const service = params?.service === undefined ? own : String(params.service);
      if (service === own) {
        need('keyring.own');
      } else {
        // A shared namespace (keyring.service): it must be one this plugin was granted, and changing it needs write access.
        need('keyring.service');
        const grants = r.plugin.granted.filter((g) => g.id === 'keyring.service' && (g.services ?? []).includes(service));
        if (!grants.length) throw new Error(`shared keyring '${service}' not granted`);
        if (op !== 'get' && !grants.some((g) => g.access === 'write')) throw new Error(`read-only access to '${service}'`);
      }
      const body: Record<string, unknown> = { service, account: String(params?.account ?? '') };
      if (op === 'set') body.secret = String(params?.secret ?? '');
      const res = await withToken(r, (t) => fetch(`/api/keyring/${op}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'x-plugin-token': t },
        body: JSON.stringify(body),
      }));
      if (!res.ok) throw new Error(`keyring ${op} failed (${res.status})`);
      return res.json();
    }
    case 'net.fetch': {
      need('network.outbound');
      const res = await withToken(r, (t) => fetch('/api/plugin-host/fetch', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'x-plugin-token': t },
        body: JSON.stringify({ url: String(params?.url ?? ''), headers: params?.headers && typeof params.headers === 'object' ? params.headers : undefined }),
      }));
      const body = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(body?.error || `fetch refused (${res.status})`);
      return { status: body.status, contentType: body.content_type, body: body.body, url: body.url, cacheSecs: body.cache_secs ?? null };
    }
    case 'library.addSong': {
      need('library.write');
      const res = await withToken(r, (t) => fetch('/api/plugin-host/songs', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'x-plugin-token': t },
        body: JSON.stringify({ song: params?.song }),
      }));
      const body = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(body?.error || `couldn't add the song (${res.status})`);
      // Let the library list pick the new song up.
      window.dispatchEvent(new CustomEvent('os-library-changed', { detail: { category: 'songs', id: body.id } }));
      return { id: body.id, song: body.song };
    }
    case 'library.addScriptures': {
      need('library.write');
      const res = await withToken(r, (t) => fetch('/api/plugin-host/scriptures', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'x-plugin-token': t },
        body: JSON.stringify({ items: params?.items }),
      }));
      const body = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(body?.error || `couldn't add the chapters (${res.status})`);
      window.dispatchEvent(new CustomEvent('os-library-changed', { detail: { category: 'scriptures' } }));
      return { chapters: body.chapters, verses: body.verses, ids: body.ids, expiresAtMs: body.expires_at_ms ?? [] };
    }
    case 'library.setRemoteBibles':
    case 'library.getRemoteBibles': {
      need('library.write');
      const setting = method === 'library.setRemoteBibles';
      const res = await withToken(r, (t) => fetch(setting ? '/api/plugin-host/remote-bibles' : '/api/plugin-host/remote-bibles/list', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'x-plugin-token': t },
        body: JSON.stringify(setting ? { bibles: params?.bibles } : {}),
      }));
      const body = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(body?.error || `couldn't ${setting ? 'save' : 'read'} the translations (${res.status})`);
      if (setting) window.dispatchEvent(new CustomEvent('os-library-changed', { detail: { category: 'scriptures', bibles: true } }));
      return setting ? { count: body.count } : { bibles: body.bibles };
    }
    case 'library.refreshLicensed': {
      need('library.write');
      const res = await withToken(r, (t) => fetch('/api/plugin-host/scriptures/refresh', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'x-plugin-token': t },
        body: JSON.stringify({ ids: params?.ids, ttl_secs: params?.ttlSecs }),
      }));
      const body = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(body?.error || `couldn't renew the passages (${res.status})`);
      return { refreshed: body.refreshed, expiresAtMs: body.expires_at_ms };
    }
    case 'process.start':
    case 'process.poll':
    case 'process.cancel':
    case 'process.release':
    case 'tool.status':
    case 'tool.update': {
      need('process.run');
      const path = method.replace('.', '/');
      const res = await withToken(r, (t) => fetch(`/api/plugin-host/${path}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'x-plugin-token': t },
        body: JSON.stringify(params ?? {}),
      }));
      const body = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(body?.error || `${method} failed (${res.status})`);
      return body;
    }
    case 'library.addMediaUrl': {
      need('library.write');
      need('network.outbound');
      const res = await withToken(r, (t) => fetch('/api/plugin-host/media-url', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'x-plugin-token': t },
        body: JSON.stringify({ url: String(params?.url ?? ''), title: String(params?.title ?? ''), id: params?.id === undefined ? undefined : String(params.id) }),
      }));
      const body = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(body?.error || `couldn't add the media (${res.status})`);
      window.dispatchEvent(new CustomEvent('os-library-changed', { detail: { category: 'media', id: body.id } }));
      return { id: body.id, item: body.item };
    }
    case 'library.addMedia': {
      need('library.write');
      const res = await withToken(r, (t) => fetch('/api/plugin-host/media', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'x-plugin-token': t },
        body: JSON.stringify(params ?? {}),
      }));
      const body = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(body?.error || `couldn't add the media (${res.status})`);
      window.dispatchEvent(new CustomEvent('os-library-changed', { detail: { category: 'media', id: body.id } }));
      return { id: body.id, item: body.item };
    }
    case 'notify': {
      need('notifications.post');
      const res = await withToken(r, (t) => fetch('/api/plugin-host/notify', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'x-plugin-token': t },
        body: JSON.stringify({
          level: params?.level, title: String(params?.title ?? ''), message: String(params?.message ?? ''),
          key: params?.key === undefined ? undefined : String(params.key), sticky: !!params?.sticky,
        }),
      }));
      const body = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(body?.error || `notification refused (${res.status})`);
      return { id: body.id };
    }
    case 'ui.pasteText': {
      need('ui.clipboard');
      // The console's own clipboard route first (works on the desktop app), then the browser's.
      let text = '';
      try {
        const res = await fetch('/api/system/clipboard');
        if (res.ok) text = String((await res.json())?.text ?? '');
      } catch { /* fall through */ }
      if (!text && navigator.clipboard?.readText) {
        try { text = await navigator.clipboard.readText(); } catch { /* denied */ }
      }
      return { text: text.slice(0, 4000) };
    }
    case 'ui.theme':
      return { vars: collectThemeVars(), iconSet: getCurrentIconSet() };
    case 'ui.toolbar.update': {
      need('ui.toolbar');
      const id = String(params?.id ?? '');
      if (!updateToolbarItem(r.plugin.id, id, { disabled: params?.disabled, pressed: params?.pressed })) {
        throw new Error(`no toolbar button '${id}'`);
      }
      return {};
    }
    case 'ui.resize': {
      need('ui.panel');
      const h = Math.max(40, Math.min(600, Number(params?.height) || 0));
      r.frame.style.height = `${h}px`;
      return { height: h };
    }
    case 'log':
      console.log(`[plugin ${r.plugin.id}]`, String(params?.message ?? '').slice(0, 2000));
      return {};
    default:
      throw new Error(`unknown method '${method}'`);
  }
}

function installListener() {
  if (listenerInstalled) return;
  listenerInstalled = true;
  window.addEventListener('message', async (ev) => {
    // The sandboxed origin is "null", so the only reliable identity is the window itself.
    const r = ev.source ? running.get(ev.source as Window) : undefined;
    const msg = ev.data;
    const w = ev.source ? watched.get(ev.source as Window) : undefined;
    if (w) {
      w.alive = Date.now();
      if (msg && msg.type === 'os.pong') w.everAlive = true;
    }
    if (!r || !msg || msg.type !== 'os.request' || typeof msg.method !== 'string') return;
    const reply = (body: Record<string, unknown>) =>
      r.frame.contentWindow?.postMessage({ type: 'os.response', id: msg.id, ...body }, '*');
    if (r.inFlight >= MAX_IN_FLIGHT) {
      reply({ ok: false, error: `too many requests in flight (max ${MAX_IN_FLIGHT})` });
      return;
    }
    r.inFlight++;
    try {
      reply({ ok: true, result: await handle(r, msg.method, msg.params) });
    } catch (e) {
      reply({ ok: false, error: String(errorMessage(e)) });
    } finally {
      r.inFlight--;
    }
  });
}

// ---- licensed passages ----------------------------------------------------------------------------
// A schedule item for licensed Bible text (YouVersion's NIV, ...) is stored and exported without the text:
// only a reference and a placeholder (src/storage/db/licensed.rs). When one shows up with its text missing
// (a file was opened, or the app restarted), the plugin named in its reference is asked to look it up again;
// it answers with the `ResolveLicensed` command. The console only relays the request.
const LICENSED_RETRY_MS = 20_000;
const licensedAsked = new Map<string, number>();
const licensedWarned = new Set<string>();
let lastSnapshot: StateSnapshot | null = null;

/** Does this plugin hold the grant that lets it fill licensed text in? */
function resolvesLicensed(p: PluginInfo): boolean {
  return grantedCommands(p).includes('ResolveLicensed');
}

/** `data` is a snapshot, or a WebSocket message that wraps one as `{ snapshot }`. */
function watchLicensed(data: StateSnapshot | { snapshot?: StateSnapshot } | null | undefined) {
  if (data) lastSnapshot = ('snapshot' in data && data.snapshot ? data.snapshot : data) as StateSnapshot;
  const items = lastSnapshot?.schedule?.items;
  if (!Array.isArray(items)) return;
  const now = Date.now();
  const bySource = new Map<string, any[]>();
  for (const item of items) {
    const l = item?.licensed;
    if (!l || l.resolved) continue;
    const key = `${item.id}|${l.usfm}`;
    if (now - (licensedAsked.get(key) ?? 0) < LICENSED_RETRY_MS) continue;
    const list = bySource.get(l.source) ?? [];
    list.push({ item_id: item.id, ...l });
    bySource.set(l.source, list);
  }
  for (const [source, requests] of bySource) {
    const frame = primaryFrame.get(source);
    const plugin = frame ? [...running.values()].find((r) => r.frame === frame)?.plugin : undefined;
    if (!frame || !plugin || !resolvesLicensed(plugin)) {
      if (!licensedWarned.has(source)) {
        licensedWarned.add(source);
        showToast(`This schedule has licensed Bible passages (${requests[0].tag}). Turn on the "${source}" plugin to show their text.`, 'warning', 9000);
      }
      continue;
    }
    for (const q of requests) licensedAsked.set(`${q.item_id}|${q.usfm}`, now);
    frame.contentWindow?.postMessage({ type: 'os.event', event: 'licensed', data: { requests } }, '*');
  }
}

// Frames that were not ready the first time, a lookup that failed: ask again now and then.
setInterval(() => watchLicensed(null), LICENSED_RETRY_MS / 2);

function ensureStateSocket() {
  if (stateSocket) return;
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  try {
    stateSocket = new WebSocket(`${proto}//${location.host}/ws`);
  } catch {
    return;
  }
  stateSocket.onmessage = (ev) => {
    let data: unknown;
    try {
      data = JSON.parse(String(ev.data));
    } catch {
      return;
    }
    for (const r of running.values()) {
      if (r.granted.has('show.observe')) {
        r.frame.contentWindow?.postMessage({ type: 'os.event', event: 'state', data }, '*');
      }
    }
    watchLicensed(data as StateSnapshot | { snapshot?: StateSnapshot });
  };
  stateSocket.onclose = () => {
    stateSocket = null;
    if (running.size) setTimeout(ensureStateSocket, 3000);
  };
}

function dock(): HTMLElement {
  let d = document.getElementById('plugin-dock');
  if (!d) {
    d = document.createElement('div');
    d.id = 'plugin-dock';
    document.body.appendChild(d);
  }
  return d;
}

function frameSrc(plugin: PluginInfo, slot: string): string {
  const entry = plugin.entry || 'index.html';
  return `/plugin-frame/${encodeURIComponent(plugin.id)}/${entry.split('/').map(encodeURIComponent).join('/')}?slot=${encodeURIComponent(slot)}`;
}

/** Where a plugin's page appears: what its manifest asks for, else a dock panel if it may show UI. */
function placementsOf(plugin: PluginInfo, granted: Set<string>): Array<{ slot: string; label?: string; tab?: string }> {
  const declared = plugin.placements ?? [];
  if (declared.length) return declared;
  return granted.has('ui.panel') ? [{ slot: 'dock' }] : [];
}

function libraryPane(): HTMLElement | null {
  const content = document.querySelector('.resource-content');
  if (!content) return null;
  let pane = content.querySelector('.plugin-source-pane') as HTMLElement | null;
  if (!pane) {
    pane = document.createElement('div');
    pane.className = 'plugin-source-pane';
    content.appendChild(pane);
  }
  return pane;
}

/** Starts one approved sandboxed plugin. */
export function startSandboxedPlugin(plugin: PluginInfo): void {
  installListener();
  const granted = new Set(plugin.granted.map(g => g.id));
  const placements = placementsOf(plugin, granted);

  // Register the frame's window *now*, not on its 'load' event: a plugin's script runs while the page is
  // still loading and sends its first request straight away, which would arrive before 'load' and be
  // ignored as coming from an unknown window. (The WindowProxy stays the same across the navigation.)
  const track = (frame: HTMLIFrameElement) => {
    if (!primaryFrame.has(plugin.id)) primaryFrame.set(plugin.id, frame);
    const register = () => {
      const w = frame.contentWindow;
      if (w) {
        running.set(w, { plugin, frame, granted, commands: grantedCommands(plugin), token: null, inFlight: 0 });
        watched.set(w, { id: plugin.id, alive: Date.now(), everAlive: false });
        startWatchdog();
      }
      watchTheme();
    };
    register();
    frame.addEventListener('load', () => {
      if (frame.contentWindow && !running.has(frame.contentWindow)) register();
      pushTheme(frame.contentWindow ?? undefined);
    });
    if (granted.has('show.observe') || resolvesLicensed(plugin)) ensureStateSocket();
  };
  const makeFrame = (slot: string) => {
    const frame = document.createElement('iframe');
    frame.setAttribute('sandbox', 'allow-scripts'); // deliberately NOT allow-same-origin
    frame.setAttribute('referrerpolicy', 'no-referrer');
    frame.dataset.pluginId = plugin.id;
    frame.dataset.slot = slot;
    frame.src = frameSrc(plugin, slot);
    return frame;
  };

  for (const placement of placements) {
    if (placement.slot === 'library-source') {
      const pane = libraryPane();
      if (!pane) continue;
      const wrapper = document.createElement('div');
      wrapper.className = 'plugin-source';
      wrapper.dataset.pluginId = plugin.id;
      const frame = makeFrame('library-source');
      wrapper.appendChild(frame);
      pane.appendChild(wrapper);
      track(frame);
      registerLibrarySource({ id: plugin.id, tab: placement.tab || 'songs', label: placement.label || plugin.name });
    } else if (placement.slot === 'video-picker') {
      // The Videos tab of the background picker has a pane for it (and shows the tab only while some plugin uses it).
      const pane = document.getElementById('plugin-picker-pane');
      if (!pane) continue;
      const wrapper = document.createElement('div');
      wrapper.className = 'plugin-source plugin-source--picker';
      wrapper.dataset.pluginId = plugin.id;
      wrapper.style.display = 'flex';
      const frame = makeFrame('video-picker');
      wrapper.appendChild(frame);
      pane.appendChild(wrapper);
      track(frame);
      syncVideoPickerTab();
    } else if (placement.slot === 'dock') {
      const frame = makeFrame('dock');
      const card = document.createElement('div');
      card.className = 'plugin-dock-card';
      const title = document.createElement('div');
      title.className = 'plugin-dock-title';
      title.textContent = plugin.name;
      card.append(title, frame);
      dock().appendChild(card);
      track(frame);
    }
  }
  if (!placements.length) {
    // No UI: a hidden frame, so the plugin can still run (it just can't show anything).
    const frame = makeFrame('hidden');
    frame.style.cssText = 'position:absolute;width:0;height:0;border:0;visibility:hidden';
    document.body.appendChild(frame);
    track(frame);
  }
  if (granted.has('ui.toolbar') && plugin.toolbar?.length) {
    addToolbarItems(plugin, (pluginId, itemId) => {
      // Exactly one frame hears the click, and the click carries no authority of its own.
      primaryFrame.get(pluginId)?.contentWindow?.postMessage({ type: 'os.event', event: 'toolbar', data: { id: itemId } }, '*');
    });
  }
}

export function isSandboxedPluginRunning(id: string): boolean {
  for (const r of running.values()) if (r.plugin.id === id) return true;
  return !!document.querySelector(`iframe[data-plugin-id="${CSS.escape(id)}"]`);
}

/** Removes a sandboxed plugin's frames, dock cards and library entries -- on revoke/deny, without a reload. */
export function stopSandboxedPlugin(id: string): void {
  for (const [win, r] of Array.from(running.entries())) {
    if (r.plugin.id === id) { running.delete(win); watched.delete(win); }
  }
  document.querySelectorAll(`iframe[data-plugin-id="${CSS.escape(id)}"]`).forEach((frame) => {
    (frame.closest('.plugin-dock-card') ?? frame.closest('.plugin-source') ?? frame).remove();
  });
  unregisterLibrarySource(id);
  removeToolbarItems(id);
  primaryFrame.delete(id);
  syncVideoPickerTab();
}
