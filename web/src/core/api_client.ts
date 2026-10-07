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
    onlineCatalog: () => api.get<any>('/api/bibles/online/catalog'),
    downloadOnline: (data: { translation: string; provider: string; source_key?: string | null }) =>
      api.post<any>('/api/bibles/online/download', data),
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
    serverInfo: () => api.get<any>('/api/server-info'),
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

  // Background self-updater for the Media tab's "import from URL" tool
  // (src/network/ytdlp_updater.rs). Read-only polling -- never triggers a
  // check itself, just reads whatever the server's own 12h background task
  // last found.
  ytdlpUpdater: {
    status: async () => {
      await resolveHostSessionToken();
      return api.get<{
        status: { updated: boolean; error: boolean; version?: string | null; message: string } | null;
      }>('/api/ytdlp-updater/status', { headers: hostTokenHeader() });
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
