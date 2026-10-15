/**
 * OpenSanctuary / OS-Next Unified API Client
 * Type-safe HTTP request client with automated JSON serialization, error handling,
 * and high-level endpoints for church presentation operations.
 */

import { hostTokenHeader, resolveHostSessionToken } from './host_session.ts';

/** Mirrors `ReleaseInfo` in `src/network/updater.rs`. */
export interface ReleaseInfo {
  version: string;
  asset_name: string;
  download_url: string;
  checksums_url: string;
  checksums_sig_url: string;
}

/** Mirrors `UpdateCheckResult` in `src/network/updater.rs`. */
export interface UpdateCheckResult {
  checked_at_ms: number;
  current_version: string;
  latest: ReleaseInfo | null;
  error: string | null;
}

export class ApiError extends Error {
  status: number;
  statusText: string;
  body: any;

  constructor(message: string, status: number, statusText: string = '', body?: any) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.statusText = statusText;
    this.body = body;
  }
}

export interface RequestOptions extends Omit<RequestInit, 'body'> {
  body?: any;
}

/**
 * Universal fetch wrapper with automated Content-Type handling and error parsing.
 */
export async function apiFetch<T = any>(
  endpoint: string,
  options: RequestOptions = {}
): Promise<T> {
  const headers = new Headers(options.headers || {});
  let body = options.body;

  if (body !== undefined && body !== null) {
    if (typeof body === 'object' && !(body instanceof FormData) && !(body instanceof Blob) && !(body instanceof ArrayBuffer)) {
      if (!headers.has('Content-Type')) {
        headers.set('Content-Type', 'application/json');
      }
      body = JSON.stringify(body);
    }
  }

  const res = await fetch(endpoint, {
    ...options,
    headers,
    body,
  });

  if (!res.ok) {
    let errorDetail = '';
    let parsedBody: any = null;
    try {
      const text = await res.text();
      try {
        parsedBody = JSON.parse(text);
        errorDetail = parsedBody.message || parsedBody.error || text;
      } catch {
        errorDetail = text;
      }
    } catch (_) {
      errorDetail = res.statusText || `HTTP ${res.status}`;
    }

    throw new ApiError(
      errorDetail || `Request failed with status ${res.status}`,
      res.status,
      res.statusText,
      parsedBody
    );
  }

  if (res.status === 204) {
    return undefined as unknown as T;
  }

  // Parse response
  const contentType = (res.headers && typeof res.headers.get === 'function')
    ? (res.headers.get('content-type') || '')
    : '';
  if (contentType.includes('application/json') || (!contentType && typeof res.json === 'function')) {
    const text = typeof res.text === 'function' ? await res.text() : '';
    if (!text || !text.trim()) return undefined as unknown as T;
    try {
      return JSON.parse(text) as T;
    } catch {
      // The server returned 2xx but a non-JSON body (an HTML error page, a
      // truncated response, a misbehaving proxy) -- handing that raw text
      // back typed as T would silently feed garbage to every caller that
      // expects a real object. Throw so callers' existing error handling
      // (try/catch or .catch()) sees this the same way a real failure would.
      throw new Error(`Invalid JSON response from ${endpoint}`);
    }
  }
  return (typeof res.text === 'function' ? await res.text() : await res.json()) as unknown as T;
}

export const api = {
  /** Generic GET request */
  get: <T = any>(url: string, init?: RequestOptions): Promise<T> =>
    apiFetch<T>(url, { ...init, method: 'GET' }),

  /** Generic POST request */
  post: <T = any>(url: string, body?: any, init?: RequestOptions): Promise<T> =>
    apiFetch<T>(url, { ...init, method: 'POST', body }),

  /** Generic PUT request */
  put: <T = any>(url: string, body?: any, init?: RequestOptions): Promise<T> =>
    apiFetch<T>(url, { ...init, method: 'PUT', body }),

  /** Generic DELETE request */
  delete: <T = any>(url: string, init?: RequestOptions): Promise<T> =>
    apiFetch<T>(url, { ...init, method: 'DELETE' }),

  /** Sends an engine command to /api/command. Attaches the host token the
   * same way `pairing.*` below does — /api/command requires it for any
   * caller not presenting a paired-device token (see
   * docs/CLIENT_PAIRING.md "Console (host) authentication"). */
  sendCommand: async <T = any>(cmd: any): Promise<T> => {
    await resolveHostSessionToken();
    return apiFetch<T>('/api/command', { method: 'POST', body: cmd, headers: hostTokenHeader() });
  },

  // High-level Domain Resources
  themes: {
    list: () => api.get<any[]>('/api/themes'),
    save: (theme: any) => api.post('/api/themes', theme),
    // Console-only now -- see src/api/routes.rs's `delete_theme`.
    delete: async (name: string) => {
      await resolveHostSessionToken();
      return api.delete(`/api/themes/${encodeURIComponent(name)}`, { headers: hostTokenHeader() });
    },
  },

  media: {
    list: () => api.get<any[]>('/api/media'),
    save: (item: any) => api.post('/api/media', item),
  },

  settings: {
    get: () => api.get<Record<string, any>>('/api/settings'),
    // Console-only now -- see src/api/routes.rs's `post_settings`.
    save: async (settings: Record<string, any>) => {
      await resolveHostSessionToken();
      return api.post('/api/settings', settings, { headers: hostTokenHeader() });
    },
  },

  bibles: {
  },

  network: {
    info: () => api.get<any>('/api/network/info'),
    interfaces: () => api.get<any>('/api/network/interfaces'),
    saveInterfaces: (data: { enabled_interfaces: string[]; broadcast_all: boolean }) =>
      api.post<any>('/api/network/interfaces', data),
    broadcastOption12: (hostname?: string) =>
      api.post<any>('/api/network/broadcast-option12', hostname ? { hostname } : {}),
    checkHostname: (name: string) =>
      api.get<any>(`/api/network/check-hostname?name=${encodeURIComponent(name)}`),
    checkPort: (port: number) =>
      api.get<any>(`/api/network/check-port?port=${port}`),
    checkPublicUrl: (url: string) =>
      api.get<any>(`/api/network/check-public-url?url=${encodeURIComponent(url)}`, { headers: hostTokenHeader() }),
    toggleDedicatedMac: (enable: boolean, hostname?: string, confirmConflict?: boolean) =>
      api.post<any>('/api/network/dedicated-mac/toggle', { enable, hostname, confirm_conflict: !!confirmConflict }),
    regenerateTls: () =>
      api.post<{ success: boolean; message: string }>('/api/network/tls/regenerate'),
  },

  displays: {
    list: () => api.get<any>('/api/displays'),
    refresh: () => api.post<any>('/api/displays/refresh'),
    open: (data: { id: string; content_path: string; monitor_index: number; resolution: any; always_on_top: boolean }) =>
      api.post<any>('/api/displays/open', data),
    close: (data: { id: string }) => api.post<any>('/api/displays/close', data),
  },

  system: {
    serverInfo: () => api.get<any>('/api/server-info', { headers: hostTokenHeader() }),
    displays: () => api.get<any>('/api/displays'),
    time: () => api.get<{ server_time_ms: number }>('/api/time'),
    clipboard: () => api.get<{ text: string }>('/api/system/clipboard'),
    pickFolder: () => api.post<{ available: boolean; path: string | null }>('/api/system/pick-folder'),
    pickFile: () => api.post<{ available: boolean; path: string | null }>('/api/system/pick-file'),
    // Console-only server-side now, same as `pairing.*` above -- moving (and
    // optionally deleting) the whole data directory is not something an
    // unauthenticated LAN caller should ever be able to trigger.
    moveDataDir: async (target_dir: string, delete_source: boolean = false) => {
      await resolveHostSessionToken();
      return api.post<{
        ok: boolean;
        source_dir: string;
        target_dir: string;
        files_copied: number;
        sha_verified: boolean;
        source_deleted: boolean;
        restart_required: boolean;
      }>('/api/system/move-data-dir', { target_dir, delete_source }, { headers: hostTokenHeader() });
    },
    // Console-only, same reasoning as moveDataDir above -- this opens a
    // window on the server's own machine, not the caller's.
    revealDataDir: async () => {
      await resolveHostSessionToken();
      return api.post<{ ok: boolean; error?: string }>('/api/system/reveal-data-dir', {}, { headers: hostTokenHeader() });
    },
    // First-time setup's "Use it instead" action (docs/first-time.md step 2)
    // for a legacy_library_detected path from /api/server-info.
    adoptLegacyLibrary: async (legacyDir: string) => {
      await resolveHostSessionToken();
      return api.post<{ ok: boolean; restart_required?: boolean; error?: string }>(
        '/api/system/adopt-legacy-library',
        { legacy_dir: legacyDir },
        { headers: hostTokenHeader() }
      );
    },
  },

  reports: {
    // Console-only (docs/CCLI_REPORTING.md): song titles/authors/usage
    // counts across an arbitrary date range, same sensitivity level as the
    // other host-token-gated settings/report actions above.
    ccliUsage: async (startMs: number, endMs: number) => {
      await resolveHostSessionToken();
      return api.get<{ ok: boolean; rows?: any[]; error?: string }>(
        `/api/reports/ccli-usage?start=${startMs}&end=${endMs}`,
        { headers: hostTokenHeader() }
      );
    },
    // CSV download needs the raw Response (for the blob), not api.get's
    // parsed-JSON return -- same reasoning as the .ewsx export in
    // app_ui.ts's performSave, plus the host-token header that route didn't
    // need but this one does.
    downloadCcliCsv: async (startMs: number, endMs: number): Promise<void> => {
      await resolveHostSessionToken();
      const res = await fetch(`/api/reports/ccli-csv?start=${startMs}&end=${endMs}`, { headers: hostTokenHeader() });
      if (!res.ok) {
        throw new Error(await res.text());
      }
      const blob = await res.blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = 'ccli-usage-report.csv';
      document.body.appendChild(a);
      a.click();
      a.remove();
      URL.revokeObjectURL(url);
    },
    // Opens the CCLI.com upload-assist window (docs/CCLI_REPORTING.md) --
    // console-only, same reasoning as revealDataDir above: this reads a
    // stored credential and opens a window on the server's own machine.
    openCcliAssist: async (startMs: number, endMs: number) => {
      await resolveHostSessionToken();
      return api.post<{ ok: boolean; error?: string }>(
        `/api/reports/ccli-open-assist?start=${startMs}&end=${endMs}`,
        {},
        { headers: hostTokenHeader() }
      );
    },
  },

  updates: {
    // Console-only now -- opens a caller-supplied local path with the OS's
    // default handler, so this must never be reachable without the host
    // token (see `apply_local_update` in src/api/routes.rs).
    applyLocal: async (data: { path: string; force?: boolean }) => {
      await resolveHostSessionToken();
      return api.post<{
        ok: boolean;
        checksum: { status: string; expected?: string; actual?: string; reason?: string };
        needs_confirmation?: boolean;
        error?: string;
      }>('/api/updates/apply-local', data, { headers: hostTokenHeader() });
    },

    // The automated check/download flow (docs/update.md) -- same
    // console-only gating as applyLocal above, and the same
    // ReleaseInfo/UpdateCheckResult shape src/network/updater.rs serializes.
    status: async () => {
      await resolveHostSessionToken();
      return api.get<{ status: UpdateCheckResult | null }>('/api/updates/status', { headers: hostTokenHeader() });
    },
    // `throttled: true` means the server skipped a real GitHub call and
    // returned the still-fresh cached result -- see MIN_SECONDS_BETWEEN_REAL_CHECKS
    // in src/api/routes.rs. Not an error; just didn't re-check the network.
    check: async () => {
      await resolveHostSessionToken();
      return api.post<{ status: UpdateCheckResult; throttled?: boolean }>('/api/updates/check', undefined, { headers: hostTokenHeader() });
    },
    downloadAndInstall: async () => {
      await resolveHostSessionToken();
      return api.post<{ ok: boolean; version?: string; error?: string }>(
        '/api/updates/download-and-install', undefined, { headers: hostTokenHeader() },
      );
    },
  },

  // Minting new trust (session/remote tokens) and exposing an existing
  // listing/secret (getDevices, which returns device tokens) is console-only
  // server-side now. These calls attach the host token when this page can
  // resolve one (native console webview, or a browser tab actually on
  // 127.0.0.1/localhost); see
  // web/src/core/host_session.ts. `authorizeDevice`, `getStatus`, and
  // `verifyDevice` stay unauthenticated by design — they're called by a
  // technician's phone or an unpaired device that has no host token to send.
  plugins: {
    list: async () => {
      await resolveHostSessionToken();
      return api.get<PluginInfo[]>('/api/plugins', { headers: hostTokenHeader() });
    },
    approve: async (id: string, digest: string, granted: string[], opts: { trustPublisher?: boolean; confirmRevokedPublisher?: boolean } = {}) => {
      await resolveHostSessionToken();
      return api.post<PluginInfo>(`/api/plugins/${encodeURIComponent(id)}/approve`, {
        digest, granted, trust_publisher: !!opts.trustPublisher, confirm_revoked_publisher: !!opts.confirmRevokedPublisher,
      }, { headers: hostTokenHeader() });
    },
    publishers: async () => {
      await resolveHostSessionToken();
      return api.get<PluginPublisher[]>('/api/plugins/publishers', { headers: hostTokenHeader() });
    },
    trustPublisher: async (keyId: string) => {
      await resolveHostSessionToken();
      return api.post<{ success: boolean }>(`/api/plugins/publishers/${encodeURIComponent(keyId)}/trust`, {}, { headers: hostTokenHeader() });
    },
    untrustPublisher: async (keyId: string) => {
      await resolveHostSessionToken();
      return api.post<{ success: boolean }>(`/api/plugins/publishers/${encodeURIComponent(keyId)}/untrust`, {}, { headers: hostTokenHeader() });
    },
    revokePublisher: async (keyId: string) => {
      await resolveHostSessionToken();
      return api.post<{ success: boolean; stopped: string[] }>(`/api/plugins/publishers/${encodeURIComponent(keyId)}/revoke`, {}, { headers: hostTokenHeader() });
    },
    /** The Advanced override: grant exactly these permissions instead of what the plugin asked for. */
    approveOverride: async (id: string, digest: string, grants: PluginGrantInput[]) => {
      await resolveHostSessionToken();
      return api.post<PluginInfo>(`/api/plugins/${encodeURIComponent(id)}/approve`, { digest, override: grants }, { headers: hostTokenHeader() });
    },
    sharedSecrets: async () => {
      await resolveHostSessionToken();
      return (await api.get<{ entries: Array<{ service: string; account: string; owner: string }> }>('/api/keyring/shared', { headers: hostTokenHeader() })).entries;
    },
    deleteSharedSecret: async (service: string, account: string) => {
      await resolveHostSessionToken();
      return api.post<{ success: boolean }>('/api/keyring/delete', { service, account }, { headers: hostTokenHeader() });
    },
    permissionCatalog: async () => {
      await resolveHostSessionToken();
      return api.get<PluginCatalogEntry[]>('/api/plugins/permissions', { headers: hostTokenHeader() });
    },
    /** Installs a plugin package (.zip). Nothing is granted: it appears as "needs review". `replace` updates an installed one. */
    install: async (file: Blob, replace = false): Promise<PluginInfo> => {
      await resolveHostSessionToken();
      const res = await fetch(`/api/plugins/install${replace ? '?replace=true' : ''}`, {
        method: 'POST', headers: { ...hostTokenHeader(), 'content-type': 'application/zip' }, body: file,
      });
      const body = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(body?.error || `install failed (${res.status})`);
      return body as PluginInfo;
    },
    /** Removes an installed plugin: files, approval, saved secrets. Built-in plugins can only be revoked. */
    uninstall: async (id: string) => {
      await resolveHostSessionToken();
      return api.delete<{ success: boolean; secrets_deleted: number }>(`/api/plugins/${encodeURIComponent(id)}`, { headers: hostTokenHeader() });
    },
    deny: async (id: string) => {
      await resolveHostSessionToken();
      return api.post<PluginInfo>(`/api/plugins/${encodeURIComponent(id)}/deny`, {}, { headers: hostTokenHeader() });
    },
    revoke: async (id: string) => {
      await resolveHostSessionToken();
      return api.post<PluginInfo>(`/api/plugins/${encodeURIComponent(id)}/revoke`, {}, { headers: hostTokenHeader() });
    },
    token: async (id: string, scopes?: Array<{ id: string; commands?: string[]; hosts?: string[]; categories?: string[] }>) => {
      await resolveHostSessionToken();
      return api.post<{ success: boolean; token: string; service: string }>(
        `/api/plugins/${encodeURIComponent(id)}/token`, scopes ? { scopes } : {}, { headers: hostTokenHeader() });
    },
    audit: async () => {
      await resolveHostSessionToken();
      return api.get<PluginAuditEntry[]>('/api/plugins/audit?limit=100', { headers: hostTokenHeader() });
    },
  },
  pairing: {
    createSession: async () => {
      await resolveHostSessionToken();
      return api.post<{ session_token: string; expires_in: number }>('/api/pairing/session', undefined, {
        headers: hostTokenHeader(),
      });
    },
    authorizeDevice: (data: { session_token: string; device_id: string; name: string; platform: string }) =>
      api.post<{ success: boolean; device_id: string; name: string; token: string }>('/api/pairing/authorize', data),
    getStatus: (deviceId: string) =>
      api.get<{ paired: boolean; device_id: string; name?: string; token?: string }>(`/api/pairing/status?device_id=${encodeURIComponent(deviceId)}`),
    getDevices: async () => {
      await resolveHostSessionToken();
      return api.get<Array<{ id: string; name: string; platform: string; paired_at: number; last_seen: number; token: string }>>('/api/pairing/devices', {
        headers: hostTokenHeader(),
      });
    },
    unpairDevice: async (id: string) => {
      await resolveHostSessionToken();
      return api.delete<{ success: boolean }>(`/api/pairing/devices/${encodeURIComponent(id)}`, {
        headers: hostTokenHeader(),
      });
    },
    remoteSession: async () => {
      await resolveHostSessionToken();
      return api.post<{ success: boolean; device_id: string; token: string }>('/api/pairing/remote-session', undefined, {
        headers: hostTokenHeader(),
      });
    },
    verifyDevice: (token: string) =>
      api.post<{ valid: boolean; device_id?: string; name?: string; platform?: string }>('/api/pairing/verify', { token }),
  },

  // ADB-driven Android TV sideload (docs/CLIENT_PAIRING.md) -- console-only,
  // same host-token gating as the pairing.* group above.
  tvProvision: {
    status: async () => {
      await resolveHostSessionToken();
      return api.get<{ adb_available: boolean; adb_version: string | null; apk_available: boolean }>(
        '/api/tv-provision/status', { headers: hostTokenHeader() },
      );
    },
    start: async (data: { ip: string; port?: number; is_stage_mode?: boolean; device_name?: string }) => {
      await resolveHostSessionToken();
      return api.post<{ task_id: string }>('/api/tv-provision/start', data, { headers: hostTokenHeader() });
    },
    progress: async (taskId: string) => {
      await resolveHostSessionToken();
      return api.get<{
        progress: {
          task_id: string;
          status: string;
          message: string;
          is_complete: boolean;
          error: string | null;
        } | null;
      }>(`/api/tv-provision/progress/${encodeURIComponent(taskId)}`, { headers: hostTokenHeader() });
    },
  },
};

export interface PluginPermissionInfo {
  id: string;
  tier: 'low' | 'medium' | 'high' | 'critical';
  enforcement: string;
  reason?: string;
  optional?: boolean;
  categories?: string[];
  commands?: string[];
  hosts?: string[];
  keys?: string[];
  paths?: string[];
  ports?: number[];
  services?: string[];
  access?: 'read' | 'write';
}

/** One permission to grant by hand in the Advanced override (the scope lists are optional). */
export interface PluginGrantInput {
  id: string;
  services?: string[];
  access?: 'read' | 'write';
  categories?: string[];
  commands?: string[];
  hosts?: string[];
  keys?: string[];
  paths?: string[];
  ports?: number[];
}

/** A permission the host knows, from GET /api/plugins/permissions. */
export interface PluginCatalogEntry {
  id: string;
  tier: 'low' | 'medium' | 'high' | 'critical';
  enforcement: string;
  scope: 'categories' | 'commands' | 'hosts' | 'paths' | 'ports' | 'keys' | 'services' | 'none';
  categories: string[];
}

export type PluginTrustLevel = 'builtin' | 'trusted' | 'known' | 'new' | 'unsigned' | 'invalid' | 'revoked';

/** A publisher (signing key) the operator trusted or revoked. */
export interface PluginPublisher {
  key_id: string;
  name: string;
  trusted: boolean;
  revoked: boolean;
  added_at: number;
  added_by: string;
  plugins: string[];
}

export interface PluginInfo {
  shared_with?: Array<{ service: string; plugins: Array<{ id: string; name: string; access: string; state: string }> }>;
  overridden?: boolean;
  override_diff?: { added: string[]; removed: string[]; changed: string[] } | null;
  id: string;
  kind: 'web' | 'sandbox';
  sandboxed?: boolean;
  /** Installed by the operator (so it can be updated or removed), as opposed to shipping with the app. */
  removable?: boolean;
  /** How far the operator has reason to trust the publisher; display only, it never widens what a plugin may do. */
  trust?: { level: PluginTrustLevel; publisher_name: string | null; key_id: string | null; can_trust: boolean };
  /** For an approved plugin that has since changed: how it differs from the version you reviewed. */
  changes?: {
    from_version: string | null;
    to_version: string | null;
    direction: 'upgrade' | 'downgrade' | 'same' | 'unknown';
    files: { added: string[]; removed: string[]; modified: string[] } | null;
    permissions: { added: string[]; removed: string[]; changed: string[]; added_tiers: Array<{ id: string; tier: string }> };
  } | null;
  entry?: string | null;
  name: string;
  version: string | null;
  state: 'pending' | 'active' | 'changed' | 'denied' | 'revoked' | 'incompatible';
  error: string | null;
  digest: string;
  signed: boolean;
  /** What the signature covers: nothing, plugin.json only, or every file. */
  signature_scope?: 'unsigned' | 'manifest' | 'full';
  /** Fully signed when you approved it, less than that now. */
  signature_weaker?: boolean;
  signer_id: string | null;
  signer_changed: boolean;
  approved_digest: string | null;
  requested: PluginPermissionInfo[];
  granted: PluginPermissionInfo[];
  decided_at: number | null;
  decided_by: string | null;
  placements?: Array<{ slot: string; label?: string; tab?: string }>;
  toolbar?: Array<{ id: string; label: string; icon: string; tooltip?: string }>;
  commands?: Array<{ id: string; tool: string; line: string; params: Array<{ name: string; type: string; values: string[]; optional: boolean }> }>;
  tools?: Array<{ id: string; github: string; assets: Record<string, string>; checksums: string; signature: string; system_command: string | null; key_sha256: string }>;
}

export interface PluginAuditEntry {
  id: number;
  ts_ms: number;
  plugin_id: string;
  actor: string;
  action: string;
  detail_json: string;
}
