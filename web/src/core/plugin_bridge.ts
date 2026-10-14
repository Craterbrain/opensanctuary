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
 *   keyring.get|set|delete        keyring.own (the plugin's own namespace)
 *   net.fetch { url }             network.outbound: the *server* fetches it, only from granted hosts
 *   library.addSong { song }      library.write on "songs"; refreshes the library list
 *   ui.resize { height }          ui.panel
 *   log { message }
 * console -> plugin: `{ type: 'os.response', id, ok, result|error }`
 *   and, with show.observe, `{ type: 'os.event', event: 'state', data }` for each state update.
 */
import { api, type PluginInfo } from "./api_client.ts";

interface Running {
  plugin: PluginInfo;
  frame: HTMLIFrameElement;
  granted: Set<string>;
  commands: string[];
  token: string | null;
}

function grantedList(p: PluginInfo, id: string, field: 'hosts' | 'categories'): string[] {
  return p.granted.filter(g => g.id === id).flatMap(g => (g[field] as string[] | undefined) ?? []);
}

const running = new Map<Window, Running>();
let listenerInstalled = false;
let stateSocket: WebSocket | null = null;

function grantedCommands(p: PluginInfo): string[] {
  return p.granted.filter(g => g.id === 'commands.send').flatMap(g => g.commands ?? []);
}

function commandName(cmd: unknown): string | null {
  if (typeof cmd === 'string') return cmd;
  if (cmd && typeof cmd === 'object') {
    const keys = Object.keys(cmd as object);
    if (keys.length === 1) return keys[0];
  }
  return null;
}

async function mintToken(r: Running): Promise<string | null> {
  const scopes: Array<{ id: string; commands?: string[]; hosts?: string[]; categories?: string[] }> = [];
  if (r.granted.has('keyring.own')) scopes.push({ id: 'keyring.own' });
  if (r.granted.has('commands.send')) scopes.push({ id: 'commands.send', commands: r.commands });
  if (r.granted.has('network.outbound')) scopes.push({ id: 'network.outbound', hosts: grantedList(r.plugin, 'network.outbound', 'hosts') });
  if (r.granted.has('library.write')) scopes.push({ id: 'library.write', categories: grantedList(r.plugin, 'library.write', 'categories') });
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
      need('keyring.own');
      const op = method.split('.')[1];
      const body: Record<string, unknown> = { service: `OpenSanctuary:Plugin:${r.plugin.id}`, account: String(params?.account ?? '') };
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
        body: JSON.stringify({ url: String(params?.url ?? '') }),
      }));
      const body = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(body?.error || `fetch refused (${res.status})`);
      return { status: body.status, contentType: body.content_type, body: body.body, url: body.url };
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
    if (!r || !msg || msg.type !== 'os.request' || typeof msg.method !== 'string') return;
    const reply = (body: Record<string, unknown>) =>
      r.frame.contentWindow?.postMessage({ type: 'os.response', id: msg.id, ...body }, '*');
    try {
      reply({ ok: true, result: await handle(r, msg.method, msg.params) });
    } catch (e: any) {
      reply({ ok: false, error: String(e?.message ?? e) });
    }
  });
}

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

/** Starts one approved sandboxed plugin. */
export function startSandboxedPlugin(plugin: PluginInfo): void {
  installListener();
  const granted = new Set(plugin.granted.map(g => g.id));
  const entry = plugin.entry || 'index.html';
  const frame = document.createElement('iframe');
  frame.setAttribute('sandbox', 'allow-scripts'); // deliberately NOT allow-same-origin
  frame.setAttribute('referrerpolicy', 'no-referrer');
  frame.dataset.pluginId = plugin.id;
  frame.src = `/plugin-frame/${encodeURIComponent(plugin.id)}/${entry.split('/').map(encodeURIComponent).join('/')}`;
  if (granted.has('ui.panel')) {
    const card = document.createElement('div');
    card.className = 'plugin-dock-card';
    const title = document.createElement('div');
    title.className = 'plugin-dock-title';
    title.textContent = plugin.name;
    card.append(title, frame);
    dock().appendChild(card);
  } else {
    frame.style.cssText = 'position:absolute;width:0;height:0;border:0;visibility:hidden';
    document.body.appendChild(frame);
  }
  frame.addEventListener('load', () => {
    const w = frame.contentWindow;
    if (w) running.set(w, { plugin, frame, granted, commands: grantedCommands(plugin), token: null });
    if (granted.has('show.observe')) ensureStateSocket();
  });
}
