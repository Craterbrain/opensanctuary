/**
 * OpenSanctuary OS-Native Keyring Client
 *
 * Provides typed, hardware-backed credential storage via the backend Keyring service.
 * Supports secure storage for:
 * - Plugins (dedicated scoped vaults per plugin with zero collision and strict ownership isolation)
 * - Dynamic external providers (media, liturgy, cloud sync, streaming)
 * - API keys, OAuth tokens, and webhook secrets
 * - User credentials (username & password)
 * - Custom service / account secrets
 *
 * Security & Sandbox Guarantees:
 * - Plugins can ONLY view and delete credentials they created.
 * - Plugins cannot view or tamper with host credentials or credentials belonging to other plugins.
 * - Hardware vaults: FreeDesktop Secret Service over D-Bus (Linux), Windows Credential Manager (Windows).
 */

import { resolveHostSessionToken } from './host_session.ts';

export interface UserCredentials {
  username: string;
  password?: string;
}

export interface KeyringResponse {
  success: boolean;
  error?: string;
}

export interface KeyringGetSecretResponse extends KeyringResponse {
  secret?: string | null;
}

export interface KeyringHasSecretResponse extends KeyringResponse {
  exists?: boolean;
}

export interface KeyringDeleteSecretResponse extends KeyringResponse {
  deleted?: boolean;
}

export interface KeyringTokenResponse extends KeyringResponse {
  token?: string;
  service?: string;
}

/**
 * Well-known service namespaces in OpenSanctuary (built-in suggestions).
 * Any arbitrary string service namespace can be used by plugins and custom providers.
 */
export const KEYRING_SERVICES = {
  PEXELS: 'OpenSanctuary:Pexels',
  PIXABAY: 'OpenSanctuary:Pixabay',
  CCLI: 'OpenSanctuary:CCLI',
  CLOUD_SYNC: 'OpenSanctuary:CloudSync',
  GENERAL: 'OpenSanctuary:General',
} as const;

export type KeyringServiceId = typeof KEYRING_SERVICES[keyof typeof KEYRING_SERVICES] | (string & {});

/**
 * Optional transport adapter interface for testing or custom endpoints.
 */
export interface KeyringTransport {
  post<T = unknown>(endpoint: string, data: unknown, headers?: Record<string, string>): Promise<T>;
}

/**
 * Scoped Keyring interface providing vault operations pre-bound to a specific service or plugin namespace.
 */
export interface ScopedKeyring {
  readonly service: string;

  setSecret(account: string, secret: string): Promise<boolean>;
  getSecret(account: string): Promise<string | null>;
  hasSecret(account: string): Promise<boolean>;
  deleteSecret(account: string): Promise<boolean>;

  setApiKey(apiKey: string): Promise<boolean>;
  getApiKey(): Promise<string | null>;
  hasApiKey(): Promise<boolean>;
  deleteApiKey(): Promise<boolean>;

  setUserCredentials(creds: UserCredentials): Promise<boolean>;
  getUserCredentials(): Promise<UserCredentials | null>;
  hasUserCredentials(): Promise<boolean>;
  deleteUserCredentials(): Promise<boolean>;
}

export class DefaultScopedKeyring implements ScopedKeyring {
  private tokenPromise?: Promise<string>;

  constructor(
    public readonly service: string,
    private readonly client: KeyringClient,
    private pluginToken?: string
  ) {}

  private async getAuthHeaders(): Promise<Record<string, string> | undefined> {
    if (this.pluginToken) {
      return { 'X-Plugin-Token': this.pluginToken };
    }
    if (this.service.startsWith('OpenSanctuary:Plugin:')) {
      const pluginName = this.service.slice('OpenSanctuary:Plugin:'.length);
      if (!this.tokenPromise) {
        this.tokenPromise = this.client.acquirePluginToken(pluginName).catch(() => '');
      }
      const tok = await this.tokenPromise;
      if (tok) {
        this.pluginToken = tok;
        return { 'X-Plugin-Token': tok };
      }
    }
    return undefined;
  }

  async setSecret(account: string, secret: string): Promise<boolean> {
    const headers = await this.getAuthHeaders();
    return this.client.setSecret(this.service, account, secret, headers);
  }

  async getSecret(account: string): Promise<string | null> {
    const headers = await this.getAuthHeaders();
    return this.client.getSecret(this.service, account, headers);
  }

  async hasSecret(account: string): Promise<boolean> {
    const headers = await this.getAuthHeaders();
    return this.client.hasSecret(this.service, account, headers);
  }

  async deleteSecret(account: string): Promise<boolean> {
    const headers = await this.getAuthHeaders();
    return this.client.deleteSecret(this.service, account, headers);
  }

  async setApiKey(apiKey: string): Promise<boolean> {
    return this.setSecret('api_key', apiKey);
  }

  async getApiKey(): Promise<string | null> {
    return this.getSecret('api_key');
  }

  async hasApiKey(): Promise<boolean> {
    return this.hasSecret('api_key');
  }

  async deleteApiKey(): Promise<boolean> {
    return this.deleteSecret('api_key');
  }

  async setUserCredentials(creds: UserCredentials): Promise<boolean> {
    if (!creds.username || !creds.username.trim()) {
      throw new Error('Username must not be empty');
    }
    const payload = JSON.stringify({
      username: creds.username.trim(),
      password: creds.password ?? '',
    });
    return this.setSecret('credentials', payload);
  }

  async getUserCredentials(): Promise<UserCredentials | null> {
    const raw = await this.getSecret('credentials');
    if (!raw) return null;

    try {
      const parsed = JSON.parse(raw);
      if (typeof parsed === 'object' && parsed !== null && 'username' in parsed) {
        return {
          username: String(parsed.username),
          password: parsed.password != null ? String(parsed.password) : undefined,
        };
      }
    } catch {
      return { username: raw };
    }
    return null;
  }

  async hasUserCredentials(): Promise<boolean> {
    return this.hasSecret('credentials');
  }

  async deleteUserCredentials(): Promise<boolean> {
    return this.deleteSecret('credentials');
  }
}

export class KeyringClient {
  private transport: KeyringTransport;
  private hostToken?: string;

  constructor(transport?: KeyringTransport, hostToken?: string) {
    this.hostToken = hostToken;
    this.transport = transport || {
      post: async <T>(endpoint: string, data: unknown, headers?: Record<string, string>): Promise<T> => {
        const reqHeaders: Record<string, string> = {
          'Content-Type': 'application/json',
          ...(headers || {}),
        };
        const res = await fetch(endpoint, {
          method: 'POST',
          headers: reqHeaders,
          body: JSON.stringify(data),
        });
        if (!res.ok) {
          const text = await res.text();
          let parsed: any;
          try {
            parsed = JSON.parse(text);
          } catch {
            parsed = { error: text || res.statusText };
          }
          throw new Error(parsed?.error || `HTTP ${res.status}: ${res.statusText}`);
        }
        return (await res.json()) as T;
      },
    };
  }

  /**
   * Overrides the transport for testing / mock environments.
   */
  setTransport(transport: KeyringTransport): void {
    this.transport = transport;
  }

  /**
   * Sets host authentication session token.
   */
  setHostToken(token: string): void {
    this.hostToken = token;
  }

  getHostToken(): string | undefined {
    return this.hostToken;
  }

  /**
   * Acquires a scoped authorization token for a plugin from the backend.
   */
  async acquirePluginToken(pluginName: string): Promise<string> {
    if (!pluginName || !pluginName.trim()) {
      throw new Error('Plugin name must not be empty');
    }
    const res = await this.transport.post<KeyringTokenResponse>('/api/keyring/plugin/token', {
      plugin_name: pluginName.trim(),
    });
    if (!res.success || !res.token) {
      throw new Error(res.error || `Failed to acquire token for plugin '${pluginName}'`);
    }
    return res.token;
  }

  /**
   * Returns a ScopedKeyring bound to any arbitrary service namespace.
   */
  scope(service: string, token?: string): ScopedKeyring {
    if (!service || !service.trim()) {
      throw new Error('Keyring service must not be empty');
    }
    return new DefaultScopedKeyring(service.trim(), this, token);
  }

  /**
   * Creates an isolated, hardware-backed vault specifically for a plugin.
   * Scoped to 'OpenSanctuary:Plugin:<pluginName>'.
   * Secrets in this vault can ONLY be viewed and deleted by this plugin.
   */
  forPlugin(pluginName: string, token?: string): ScopedKeyring {
    if (!pluginName || !pluginName.trim()) {
      throw new Error('Plugin name must not be empty');
    }
    return this.scope(`OpenSanctuary:Plugin:${pluginName.trim()}`, token);
  }

  /**
   * Creates a vault for an external media, liturgy, or cloud provider.
   * Scoped to 'OpenSanctuary:Provider:<providerName>'.
   */
  forProvider(providerName: string, token?: string): ScopedKeyring {
    if (!providerName || !providerName.trim()) {
      throw new Error('Provider name must not be empty');
    }
    return this.scope(`OpenSanctuary:Provider:${providerName.trim()}`, token);
  }

  /**
   * Stores an arbitrary secret for a service and account in the OS keyring.
   */
  async setSecret(service: string, account: string, secret: string, headers?: Record<string, string>): Promise<boolean> {
    if (!service || !service.trim()) {
      throw new Error('Keyring service must not be empty');
    }
    if (!account || !account.trim()) {
      throw new Error('Keyring account must not be empty');
    }

    const reqHeaders: Record<string, string> = { ...(headers || {}) };
    if (!reqHeaders['X-Plugin-Token'] && !reqHeaders['X-Host-Token'] && this.hostToken) {
      reqHeaders['X-Host-Token'] = this.hostToken;
    }

    const res = await this.transport.post<KeyringResponse>('/api/keyring/set', {
      service: service.trim(),
      account: account.trim(),
      secret,
    }, reqHeaders);
    return !!res.success;
  }

  /**
   * Retrieves a secret for a service and account from the OS keyring.
   * Returns null if no secret exists or if access is forbidden.
   */
  async getSecret(service: string, account: string, headers?: Record<string, string>): Promise<string | null> {
    if (!service || !service.trim() || !account || !account.trim()) {
      return null;
    }

    const reqHeaders: Record<string, string> = { ...(headers || {}) };
    if (!reqHeaders['X-Plugin-Token'] && !reqHeaders['X-Host-Token'] && this.hostToken) {
      reqHeaders['X-Host-Token'] = this.hostToken;
    }

    try {
      const res = await this.transport.post<KeyringGetSecretResponse>('/api/keyring/get', {
        service: service.trim(),
        account: account.trim(),
      }, reqHeaders);
      return res.success && res.secret != null ? res.secret : null;
    } catch {
      return null;
    }
  }

  /**
   * Checks if a secret exists in the OS keyring.
   */
  async hasSecret(service: string, account: string, headers?: Record<string, string>): Promise<boolean> {
    if (!service || !service.trim() || !account || !account.trim()) {
      return false;
    }

    const reqHeaders: Record<string, string> = { ...(headers || {}) };
    if (!reqHeaders['X-Plugin-Token'] && !reqHeaders['X-Host-Token'] && this.hostToken) {
      reqHeaders['X-Host-Token'] = this.hostToken;
    }

    try {
      const res = await this.transport.post<KeyringHasSecretResponse>('/api/keyring/has', {
        service: service.trim(),
        account: account.trim(),
      }, reqHeaders);
      return !!(res.success && res.exists);
    } catch {
      return false;
    }
  }

  /**
   * Deletes a secret from the OS keyring.
   * Returns true if deleted, false if it did not exist or caller lacks ownership.
   */
  async deleteSecret(service: string, account: string, headers?: Record<string, string>): Promise<boolean> {
    if (!service || !service.trim() || !account || !account.trim()) {
      return false;
    }

    const reqHeaders: Record<string, string> = { ...(headers || {}) };
    if (!reqHeaders['X-Plugin-Token'] && !reqHeaders['X-Host-Token'] && this.hostToken) {
      reqHeaders['X-Host-Token'] = this.hostToken;
    }

    try {
      const res = await this.transport.post<KeyringDeleteSecretResponse>('/api/keyring/delete', {
        service: service.trim(),
        account: account.trim(),
      }, reqHeaders);
      return !!(res.success && res.deleted);
    } catch {
      return false;
    }
  }

  // --- Specialized API Key Helpers ---

  async setApiKey(service: string, apiKey: string, headers?: Record<string, string>): Promise<boolean> {
    return this.setSecret(service, 'api_key', apiKey, headers);
  }

  async getApiKey(service: string, headers?: Record<string, string>): Promise<string | null> {
    return this.getSecret(service, 'api_key', headers);
  }

  async hasApiKey(service: string, headers?: Record<string, string>): Promise<boolean> {
    return this.hasSecret(service, 'api_key', headers);
  }

  async deleteApiKey(service: string, headers?: Record<string, string>): Promise<boolean> {
    return this.deleteSecret(service, 'api_key', headers);
  }

  // --- Specialized User Credential Helpers ---

  async setUserCredentials(service: string, creds: UserCredentials, headers?: Record<string, string>): Promise<boolean> {
    if (!creds.username || !creds.username.trim()) {
      throw new Error('Username must not be empty');
    }
    const payload = JSON.stringify({
      username: creds.username.trim(),
      password: creds.password ?? '',
    });
    return this.setSecret(service, 'credentials', payload, headers);
  }

  async getUserCredentials(service: string, headers?: Record<string, string>): Promise<UserCredentials | null> {
    const raw = await this.getSecret(service, 'credentials', headers);
    if (!raw) return null;

    try {
      const parsed = JSON.parse(raw);
      if (typeof parsed === 'object' && parsed !== null && 'username' in parsed) {
        return {
          username: String(parsed.username),
          password: parsed.password != null ? String(parsed.password) : undefined,
        };
      }
    } catch {
      return { username: raw };
    }
    return null;
  }

  async hasUserCredentials(service: string, headers?: Record<string, string>): Promise<boolean> {
    return this.hasSecret(service, 'credentials', headers);
  }

  async deleteUserCredentials(service: string, headers?: Record<string, string>): Promise<boolean> {
    return this.deleteSecret(service, 'credentials', headers);
  }
}

/**
 * Singleton instance of KeyringClient for application-wide host use.
 */
export const keyring = new KeyringClient();

// Auto-register on global OS runtime context for host use
if (typeof window !== 'undefined') {
  (window as any).OS = (window as any).OS || {};
  (window as any).OS.keyring = keyring;
}
if (typeof globalThis !== 'undefined') {
  (globalThis as any).OS = (globalThis as any).OS || {};
  (globalThis as any).OS.keyring = keyring;
}

// Give the singleton its host token as soon as it's resolvable — see
// web/src/core/host_session.ts for how (and why) that's the only real
// source, never a network response an arbitrary LAN client could forge.
resolveHostSessionToken().then((token) => {
  if (token) keyring.setHostToken(token);
});
