/**
 * OpenSanctuary / OS-Next Unified API Client
 * Type-safe HTTP request client with automated JSON serialization, error handling,
 * and high-level endpoints for church presentation operations.
 */

import { hostTokenHeader, resolveHostSessionToken } from './host_session.ts';

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
      return text as unknown as T;
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

  /** Sends an engine command to /api/command */
  sendCommand: <T = any>(cmd: any): Promise<T> =>
    apiFetch<T>('/api/command', { method: 'POST', body: cmd }),

  // High-level Domain Resources
  themes: {
    list: () => api.get<any[]>('/api/themes'),
    save: (theme: any) => api.post('/api/themes', theme),
    delete: (name: string) => api.delete(`/api/themes/${encodeURIComponent(name)}`),
  },

  media: {
    list: () => api.get<any[]>('/api/media'),
    save: (item: any) => api.post('/api/media', item),
  },

  settings: {
    get: () => api.get<Record<string, any>>('/api/settings'),
    save: (settings: Record<string, any>) => api.post('/api/settings', settings),
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
  },

  // Minting new trust (session/remote tokens) and exposing an existing
  // listing/secret (getDevices, which returns device tokens) is console-only
  // server-side now — see docs/GEMINI_COMMIT_REVIEW_2026-09-22.md #3. These
  // calls attach the host token when this page can resolve one (native
  // console webview, or a browser tab actually on 127.0.0.1/localhost); see
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
};
