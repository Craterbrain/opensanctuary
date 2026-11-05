import { describe, test, expect, beforeEach } from 'bun:test';
import {
  KeyringClient,
  KEYRING_SERVICES,
  type KeyringTransport,
  type UserCredentials,
} from '../src/core/keyring';

describe('OS-Native Keyring Client (Linux & Windows Vault)', () => {
  let mockStore: Map<string, string>;
  let mockOwnership: Map<string, string>;
  let mockCalls: { endpoint: string; token?: string }[] = [];
  let mockTransport: KeyringTransport;
  let client: KeyringClient;

  beforeEach(() => {
    mockStore = new Map<string, string>();
    mockOwnership = new Map<string, string>();
    mockCalls = [];
    mockTransport = {
      post: async <T>(endpoint: string, data: any, headers?: Record<string, string>): Promise<T> => {
        if (endpoint === '/api/keyring/plugin/token') {
          const { plugin_name } = data;
          if (!plugin_name) throw new Error('plugin_name must not be empty');
          return { success: true, token: `token_for_${plugin_name}` } as T;
        }

        const { service, account, secret } = data;
        const key = `${service}::${account}`;
        const pluginToken = headers?.['X-Plugin-Token'];
        mockCalls.push({ endpoint, token: pluginToken });

        let callerTag = 'host';
        if (pluginToken) {
          if (pluginToken.startsWith('token_for_')) {
            const pluginName = pluginToken.replace('token_for_', '');
            callerTag = `plugin:${pluginName}`;
            const expectedService = `OpenSanctuary:Plugin:${pluginName}`;
            if (service !== expectedService) {
              throw new Error(`403 Forbidden: Plugin '${pluginName}' cannot access namespace '${service}'`);
            }
          } else {
            throw new Error('401 Unauthorized: Invalid plugin token');
          }
        }

        if (endpoint === '/api/keyring/set') {
          if (!service || !account) {
            throw new Error('service and account must not be empty');
          }
          const existingOwner = mockOwnership.get(key);
          if (existingOwner && existingOwner !== callerTag) {
            throw new Error(`403 Forbidden: Secret owned by '${existingOwner}'`);
          }
          mockStore.set(key, secret);
          mockOwnership.set(key, callerTag);
          return { success: true } as T;
        }

        if (endpoint === '/api/keyring/get') {
          if (!service || !account) {
            throw new Error('service and account must not be empty');
          }
          const owner = mockOwnership.get(key);
          if (callerTag.startsWith('plugin:') && owner !== callerTag) {
            throw new Error(`403 Forbidden: Plugin can only view secrets it created`);
          }
          const val = mockStore.get(key);
          return { success: true, secret: val !== undefined ? val : null } as T;
        }

        if (endpoint === '/api/keyring/has') {
          if (!service || !account) {
            throw new Error('service and account must not be empty');
          }
          const owner = mockOwnership.get(key);
          if (callerTag.startsWith('plugin:') && owner !== callerTag) {
            return { success: true, exists: false } as T;
          }
          const exists = mockStore.has(key);
          return { success: true, exists } as T;
        }

        if (endpoint === '/api/keyring/delete') {
          if (!service || !account) {
            throw new Error('service and account must not be empty');
          }
          const owner = mockOwnership.get(key);
          if (callerTag.startsWith('plugin:') && owner !== callerTag) {
            throw new Error(`403 Forbidden: Plugin can only delete secrets it created`);
          }
          const deleted = mockStore.delete(key);
          mockOwnership.delete(key);
          return { success: true, deleted } as T;
        }

        throw new Error(`Unhandled endpoint: ${endpoint}`);
      },
    };
    client = new KeyringClient(mockTransport);
  });

  describe('Standard Service Identifiers', () => {
    test('defines expected standard service namespaces', () => {
      expect(KEYRING_SERVICES.CCLI).toBe('OpenSanctuary:CCLI');
      expect(KEYRING_SERVICES.CLOUD_SYNC).toBe('OpenSanctuary:CloudSync');
      expect(KEYRING_SERVICES.GENERAL).toBe('OpenSanctuary:General');
    });
  });

  describe('Core Secret CRUD Operations', () => {
    test('sets and gets secrets successfully', async () => {
      const setOk = await client.setSecret('test_service', 'test_user', 'super_secret_token_123');
      expect(setOk).toBe(true);

      const retrieved = await client.getSecret('test_service', 'test_user');
      expect(retrieved).toBe('super_secret_token_123');
    });

    test('checks existence with hasSecret', async () => {
      expect(await client.hasSecret('test_service', 'unknown')).toBe(false);

      await client.setSecret('test_service', 'known', 'value');
      expect(await client.hasSecret('test_service', 'known')).toBe(true);
    });

    test('deletes secrets cleanly with deleteSecret', async () => {
      await client.setSecret('test_service', 'to_delete', 'value');
      expect(await client.hasSecret('test_service', 'to_delete')).toBe(true);

      const deleted = await client.deleteSecret('test_service', 'to_delete');
      expect(deleted).toBe(true);

      expect(await client.hasSecret('test_service', 'to_delete')).toBe(false);
      expect(await client.getSecret('test_service', 'to_delete')).toBeNull();

      // Deleting again should return false (did not exist)
      const deletedAgain = await client.deleteSecret('test_service', 'to_delete');
      expect(deletedAgain).toBe(false);
    });

    test('validates input and throws on empty service or account', async () => {
      await expect(client.setSecret('', 'user', 'sec')).rejects.toThrow('Keyring service must not be empty');
      await expect(client.setSecret('   ', 'user', 'sec')).rejects.toThrow('Keyring service must not be empty');
      await expect(client.setSecret('svc', '', 'sec')).rejects.toThrow('Keyring account must not be empty');
      await expect(client.setSecret('svc', '   ', 'sec')).rejects.toThrow('Keyring account must not be empty');

      // getSecret/hasSecret/deleteSecret return null/false for empty input
      expect(await client.getSecret('', 'user')).toBeNull();
      expect(await client.hasSecret('svc', '')).toBe(false);
      expect(await client.deleteSecret('', '')).toBe(false);
    });
  });

  describe('API Key Specialized Helpers', () => {
    test('stores and retrieves API keys under standard api_key account', async () => {
      const pexelsKey = 'px_live_998877665544332211';
      await client.setApiKey(KEYRING_SERVICES.GENERAL, pexelsKey);

      expect(await client.hasApiKey(KEYRING_SERVICES.GENERAL)).toBe(true);
      expect(await client.getApiKey(KEYRING_SERVICES.GENERAL)).toBe(pexelsKey);

      // Verify that the underlying key in store is keyed under account 'api_key'
      const raw = await client.getSecret(KEYRING_SERVICES.GENERAL, 'api_key');
      expect(raw).toBe(pexelsKey);

      // Delete API key
      const deleted = await client.deleteApiKey(KEYRING_SERVICES.GENERAL);
      expect(deleted).toBe(true);
      expect(await client.hasApiKey(KEYRING_SERVICES.GENERAL)).toBe(false);
      expect(await client.getApiKey(KEYRING_SERVICES.GENERAL)).toBeNull();
    });

    test('handles multiple API keys independently across different services', async () => {
      await client.setApiKey(KEYRING_SERVICES.GENERAL, 'key_pexels');
      await client.setApiKey(KEYRING_SERVICES.CLOUD_SYNC, 'key_pixabay');

      expect(await client.getApiKey(KEYRING_SERVICES.GENERAL)).toBe('key_pexels');
      expect(await client.getApiKey(KEYRING_SERVICES.CLOUD_SYNC)).toBe('key_pixabay');

      await client.deleteApiKey(KEYRING_SERVICES.GENERAL);
      expect(await client.getApiKey(KEYRING_SERVICES.GENERAL)).toBeNull();
      expect(await client.getApiKey(KEYRING_SERVICES.CLOUD_SYNC)).toBe('key_pixabay');
    });
  });

  describe('User Credentials Specialized Helpers (Username & Password)', () => {
    test('stores and retrieves username and password as a secure bundle', async () => {
      const creds: UserCredentials = {
        username: 'pastor_john',
        password: 'SecureP@ssw0rd!#2026_Church',
      };

      const ok = await client.setUserCredentials(KEYRING_SERVICES.CCLI, creds);
      expect(ok).toBe(true);

      expect(await client.hasUserCredentials(KEYRING_SERVICES.CCLI)).toBe(true);

      const retrieved = await client.getUserCredentials(KEYRING_SERVICES.CCLI);
      expect(retrieved).not.toBeNull();
      expect(retrieved?.username).toBe('pastor_john');
      expect(retrieved?.password).toBe('SecureP@ssw0rd!#2026_Church');
    });

    test('supports username without password', async () => {
      const creds: UserCredentials = {
        username: 'media_team_operator',
      };

      await client.setUserCredentials(KEYRING_SERVICES.CLOUD_SYNC, creds);
      const retrieved = await client.getUserCredentials(KEYRING_SERVICES.CLOUD_SYNC);

      expect(retrieved?.username).toBe('media_team_operator');
      expect(retrieved?.password).toBe('');
    });

    test('validates username requirement', async () => {
      await expect(
        client.setUserCredentials(KEYRING_SERVICES.CCLI, { username: '' })
      ).rejects.toThrow('Username must not be empty');

      await expect(
        client.setUserCredentials(KEYRING_SERVICES.CCLI, { username: '   ' })
      ).rejects.toThrow('Username must not be empty');
    });

    test('deletes user credentials cleanly', async () => {
      await client.setUserCredentials(KEYRING_SERVICES.CCLI, {
        username: 'admin',
        password: 'secret_password',
      });
      expect(await client.hasUserCredentials(KEYRING_SERVICES.CCLI)).toBe(true);

      const deleted = await client.deleteUserCredentials(KEYRING_SERVICES.CCLI);
      expect(deleted).toBe(true);

      expect(await client.hasUserCredentials(KEYRING_SERVICES.CCLI)).toBe(false);
      expect(await client.getUserCredentials(KEYRING_SERVICES.CCLI)).toBeNull();
    });

    test('handles fallback when secret is plain string rather than JSON', async () => {
      // If a plain string was stored previously
      await client.setSecret(KEYRING_SERVICES.CCLI, 'credentials', 'legacy_username_only');

      const retrieved = await client.getUserCredentials(KEYRING_SERVICES.CCLI);
      expect(retrieved).not.toBeNull();
      expect(retrieved?.username).toBe('legacy_username_only');
      expect(retrieved?.password).toBeUndefined();
    });

    test('handles complex characters, unicode, and symbols in passwords', async () => {
      const complexPass = '🔒P@ss:wörđ—"quotes" & <tags> / 100% #! ⛪';
      await client.setUserCredentials('OpenSanctuary:CustomTest', {
        username: 'test_user_ñ',
        password: complexPass,
      });

      const retrieved = await client.getUserCredentials('OpenSanctuary:CustomTest');
      expect(retrieved?.username).toBe('test_user_ñ');
      expect(retrieved?.password).toBe(complexPass);
    });
  });

  describe('Error and Resilience Handling', () => {
    test('handles transport failure gracefully on getSecret and hasSecret', async () => {
      const faultyTransport: KeyringTransport = {
        post: async () => {
          throw new Error('500 Internal Server Error: Keyring daemon unreachable');
        },
      };
      const failingClient = new KeyringClient(faultyTransport);

      // getSecret should catch and return null rather than crashing
      const res = await failingClient.getSecret('service', 'account');
      expect(res).toBeNull();

      // hasSecret should catch and return false rather than crashing
      const exists = await failingClient.hasSecret('service', 'account');
      expect(exists).toBe(false);

      // deleteSecret should catch and return false rather than crashing
      const deleted = await failingClient.deleteSecret('service', 'account');
      expect(deleted).toBe(false);

      // setSecret should throw so the caller knows save failed
      await expect(failingClient.setSecret('service', 'account', 'pass')).rejects.toThrow(
        'Keyring daemon unreachable'
      );
    });
  });

  describe('Scoped Keyring & Plugin Architecture', () => {
    test('creates isolated scoped vaults for plugins via forPlugin', async () => {
      const streamDeckVault = client.forPlugin('streamdeck_pro');
      const obsVault = client.forPlugin('obs_studio_remote');

      expect(streamDeckVault.service).toBe('OpenSanctuary:Plugin:streamdeck_pro');
      expect(obsVault.service).toBe('OpenSanctuary:Plugin:obs_studio_remote');

      // Set API keys in both plugin vaults
      await streamDeckVault.setApiKey('sd_token_abcdef');
      await obsVault.setApiKey('obs_token_987654');

      // Verify each plugin only reads its own key
      expect(await streamDeckVault.getApiKey()).toBe('sd_token_abcdef');
      expect(await obsVault.getApiKey()).toBe('obs_token_987654');

      // Deleting streamdeck key doesn't affect OBS
      await streamDeckVault.deleteApiKey();
      expect(await streamDeckVault.hasApiKey()).toBe(false);
      expect(await obsVault.hasApiKey()).toBe(true);
      expect(await obsVault.getApiKey()).toBe('obs_token_987654');
    });

    test('supports user credentials inside scoped plugin vaults', async () => {
      const planningCenter = client.forPlugin('planning_center_sync');

      await planningCenter.setUserCredentials({
        username: 'worship_leader_sam',
        password: 'PCO_OAuth_Secret_Key!',
      });

      expect(await planningCenter.hasUserCredentials()).toBe(true);
      const creds = await planningCenter.getUserCredentials();
      expect(creds?.username).toBe('worship_leader_sam');
      expect(creds?.password).toBe('PCO_OAuth_Secret_Key!');

      // Direct custom secrets
      await planningCenter.setSecret('refresh_token', 'rt_5544332211');
      expect(await planningCenter.getSecret('refresh_token')).toBe('rt_5544332211');
    });

    test('creates dynamic provider vaults via forProvider without hardcoded enum limitations', async () => {
      // Unsplash (not in predefined list)
      const unsplash = client.forProvider('unsplash');
      expect(unsplash.service).toBe('OpenSanctuary:Provider:unsplash');

      await unsplash.setApiKey('un_key_112233');
      expect(await unsplash.getApiKey()).toBe('un_key_112233');

      // Faithlife / SongSelect / Custom Church Backend
      const songSelect = client.forProvider('ccli_songselect_v2');
      await songSelect.setUserCredentials({
        username: 'church_admin_lic',
        password: 'songselect_pwd',
      });
      const creds = await songSelect.getUserCredentials();
      expect(creds?.username).toBe('church_admin_lic');
      expect(creds?.password).toBe('songselect_pwd');
    });

    test('arbitrary custom namespaces via client.scope()', async () => {
      const customVault = client.scope('CustomOrg:CustomApp:ModuleX');
      await customVault.setSecret('token', 'module_secret');
      expect(await customVault.getSecret('token')).toBe('module_secret');
      expect(await customVault.hasSecret('token')).toBe(true);
      await customVault.deleteSecret('token');
      expect(await customVault.hasSecret('token')).toBe(false);
    });
  });

  // Who may read, write or delete what is decided by the server (tests/core_tests/plugins.rs,
  // e2e_shared_keyring, e2e_cross_plugin_escalation). The client's part: a plugin vault always
  // sends that plugin's token, and a refusal reads as "nothing there" instead of an exception.
  describe('Plugin vaults', () => {
    test('a plugin vault sends its plugin token on every call', async () => {
      expect(await client.acquirePluginToken('obs_stream')).toBe('token_for_obs_stream');
      await expect(client.acquirePluginToken('   ')).rejects.toThrow('Plugin name must not be empty');

      const vault = client.forPlugin('my_plugin');
      mockCalls = [];
      expect(await vault.setApiKey('mine')).toBe(true);
      expect(await vault.getApiKey()).toBe('mine');
      expect(await vault.hasApiKey()).toBe(true);
      expect(await vault.deleteApiKey()).toBe(true);
      expect(await vault.getApiKey()).toBeNull();
      // Every keyring call carried the plugin's own token (without it the server treats the caller as the host).
      const keyringCalls = mockCalls.filter((c) => c.endpoint !== '/api/keyring/plugin/token');
      expect(keyringCalls.length).toBeGreaterThanOrEqual(5);
      expect(keyringCalls.every((c) => c.token === 'token_for_my_plugin')).toBe(true);
    });

    test('a refused read, check or delete is null/false; a refused write throws', async () => {
      await client.forPlugin('plugin_b').setApiKey('secret_b');
      const asA = { 'X-Plugin-Token': await client.acquirePluginToken('plugin_a') };
      const b = 'OpenSanctuary:Plugin:plugin_b';

      expect(await client.getSecret(b, 'api_key', asA)).toBeNull();
      expect(await client.hasSecret(b, 'api_key', asA)).toBe(false);
      expect(await client.deleteSecret(b, 'api_key', asA)).toBe(false);
      await expect(client.setSecret(b, 'api_key', 'overwrite', asA)).rejects.toThrow('cannot access namespace');
      expect(await client.getSecret(b, 'api_key', { 'X-Plugin-Token': 'forged' })).toBeNull();
      expect(await client.forPlugin('plugin_b').getApiKey()).toBe('secret_b');
    });
  });
});
