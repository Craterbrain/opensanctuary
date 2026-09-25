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
  let mockTransport: KeyringTransport;
  let client: KeyringClient;

  beforeEach(() => {
    mockStore = new Map<string, string>();
    mockOwnership = new Map<string, string>();
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
      expect(KEYRING_SERVICES.PEXELS).toBe('OpenSanctuary:Pexels');
      expect(KEYRING_SERVICES.PIXABAY).toBe('OpenSanctuary:Pixabay');
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
      expect(client.setSecret('', 'user', 'sec')).rejects.toThrow('Keyring service must not be empty');
      expect(client.setSecret('   ', 'user', 'sec')).rejects.toThrow('Keyring service must not be empty');
      expect(client.setSecret('svc', '', 'sec')).rejects.toThrow('Keyring account must not be empty');
      expect(client.setSecret('svc', '   ', 'sec')).rejects.toThrow('Keyring account must not be empty');

      // getSecret/hasSecret/deleteSecret return null/false for empty input
      expect(await client.getSecret('', 'user')).toBeNull();
      expect(await client.hasSecret('svc', '')).toBe(false);
      expect(await client.deleteSecret('', '')).toBe(false);
    });
  });

  describe('API Key Specialized Helpers', () => {
    test('stores and retrieves API keys under standard api_key account', async () => {
      const pexelsKey = 'px_live_998877665544332211';
      await client.setApiKey(KEYRING_SERVICES.PEXELS, pexelsKey);

      expect(await client.hasApiKey(KEYRING_SERVICES.PEXELS)).toBe(true);
      expect(await client.getApiKey(KEYRING_SERVICES.PEXELS)).toBe(pexelsKey);

      // Verify that the underlying key in store is keyed under account 'api_key'
      const raw = await client.getSecret(KEYRING_SERVICES.PEXELS, 'api_key');
      expect(raw).toBe(pexelsKey);

      // Delete API key
      const deleted = await client.deleteApiKey(KEYRING_SERVICES.PEXELS);
      expect(deleted).toBe(true);
      expect(await client.hasApiKey(KEYRING_SERVICES.PEXELS)).toBe(false);
      expect(await client.getApiKey(KEYRING_SERVICES.PEXELS)).toBeNull();
    });

    test('handles multiple API keys independently across different services', async () => {
      await client.setApiKey(KEYRING_SERVICES.PEXELS, 'key_pexels');
      await client.setApiKey(KEYRING_SERVICES.PIXABAY, 'key_pixabay');

      expect(await client.getApiKey(KEYRING_SERVICES.PEXELS)).toBe('key_pexels');
      expect(await client.getApiKey(KEYRING_SERVICES.PIXABAY)).toBe('key_pixabay');

      await client.deleteApiKey(KEYRING_SERVICES.PEXELS);
      expect(await client.getApiKey(KEYRING_SERVICES.PEXELS)).toBeNull();
      expect(await client.getApiKey(KEYRING_SERVICES.PIXABAY)).toBe('key_pixabay');
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
      expect(
        client.setUserCredentials(KEYRING_SERVICES.CCLI, { username: '' })
      ).rejects.toThrow('Username must not be empty');

      expect(
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
      expect(failingClient.setSecret('service', 'account', 'pass')).rejects.toThrow(
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

  describe('Plugin Ownership & Credential Isolation Boundary', () => {
    test('acquires plugin capability tokens via acquirePluginToken', async () => {
      const token = await client.acquirePluginToken('obs_stream');
      expect(token).toBe('token_for_obs_stream');

      expect(client.acquirePluginToken('')).rejects.toThrow('Plugin name must not be empty');
      expect(client.acquirePluginToken('   ')).rejects.toThrow('Plugin name must not be empty');
    });

    test('plugin can create, view, check existence, and delete its own secrets', async () => {
      const pluginVault = client.forPlugin('my_plugin');

      // 1. Create own secret
      const saved = await pluginVault.setApiKey('my_secret_token_123');
      expect(saved).toBe(true);

      // 2. View own secret
      const retrieved = await pluginVault.getApiKey();
      expect(retrieved).toBe('my_secret_token_123');

      // 3. Check existence of own secret
      const exists = await pluginVault.hasApiKey();
      expect(exists).toBe(true);

      // 4. Delete own secret
      const deleted = await pluginVault.deleteApiKey();
      expect(deleted).toBe(true);

      // 5. Verify post-deletion state
      expect(await pluginVault.hasApiKey()).toBe(false);
      expect(await pluginVault.getApiKey()).toBeNull();
    });

    test('plugin A cannot view secrets created by plugin B', async () => {
      const vaultB = client.forPlugin('plugin_b');
      await vaultB.setApiKey('secret_of_plugin_b');

      // Verify plugin B can read it
      expect(await vaultB.getApiKey()).toBe('secret_of_plugin_b');

      // Plugin A acquires its token
      const tokenA = await client.acquirePluginToken('plugin_a');

      // Plugin A attempts to view Plugin B's secret
      const forbiddenView = await client.getSecret(
        'OpenSanctuary:Plugin:plugin_b',
        'api_key',
        { 'X-Plugin-Token': tokenA }
      );
      expect(forbiddenView).toBeNull();

      // Plugin A checks existence of Plugin B's secret (should report false to prevent secret probing)
      const forbiddenExists = await client.hasSecret(
        'OpenSanctuary:Plugin:plugin_b',
        'api_key',
        { 'X-Plugin-Token': tokenA }
      );
      expect(forbiddenExists).toBe(false);
    });

    test('plugin A cannot delete secrets created by plugin B', async () => {
      const vaultB = client.forPlugin('plugin_b');
      await vaultB.setApiKey('vital_secret_b');

      const tokenA = await client.acquirePluginToken('plugin_a');

      // Plugin A attempts to delete Plugin B's secret
      const deleteAttempt = await client.deleteSecret(
        'OpenSanctuary:Plugin:plugin_b',
        'api_key',
        { 'X-Plugin-Token': tokenA }
      );
      expect(deleteAttempt).toBe(false);

      // Verify Plugin B's secret remains intact and undamaged
      expect(await vaultB.getApiKey()).toBe('vital_secret_b');
    });

    test('plugin A cannot overwrite or create secrets inside plugin B namespace', async () => {
      const vaultB = client.forPlugin('plugin_b');
      await vaultB.setApiKey('original_b');

      const tokenA = await client.acquirePluginToken('plugin_a');

      // Attempt to tamper with plugin B's secret
      expect(
        client.setSecret(
          'OpenSanctuary:Plugin:plugin_b',
          'api_key',
          'malicious_overwrite',
          { 'X-Plugin-Token': tokenA }
        )
      ).rejects.toThrow('cannot access namespace');

      // Verify original value unchanged
      expect(await vaultB.getApiKey()).toBe('original_b');
    });

    test('plugin cannot view host secrets (e.g. Pexels API Key or Admin credentials)', async () => {
      // Host saves host-level credential
      await client.setApiKey(KEYRING_SERVICES.PEXELS, 'official_pexels_host_key_999');

      const tokenPlugin = await client.acquirePluginToken('untrusted_plugin');

      // Plugin attempts to read host Pexels key
      const attempt = await client.getSecret(
        KEYRING_SERVICES.PEXELS,
        'api_key',
        { 'X-Plugin-Token': tokenPlugin }
      );
      expect(attempt).toBeNull();

      const existsCheck = await client.hasSecret(
        KEYRING_SERVICES.PEXELS,
        'api_key',
        { 'X-Plugin-Token': tokenPlugin }
      );
      expect(existsCheck).toBe(false);
    });

    test('plugin cannot delete host secrets', async () => {
      // Host stores CCLI credentials
      await client.setUserCredentials(KEYRING_SERVICES.CCLI, {
        username: 'senior_pastor',
        password: 'ChurchKey_2026!',
      });

      const tokenPlugin = await client.acquirePluginToken('rogue_plugin');

      // Plugin attempts to delete host credentials
      const deleteResult = await client.deleteSecret(
        KEYRING_SERVICES.CCLI,
        'credentials',
        { 'X-Plugin-Token': tokenPlugin }
      );
      expect(deleteResult).toBe(false);

      // Verify host credentials remain untouched
      const hostCreds = await client.getUserCredentials(KEYRING_SERVICES.CCLI);
      expect(hostCreds?.username).toBe('senior_pastor');
      expect(hostCreds?.password).toBe('ChurchKey_2026!');
    });

    test('plugin cannot overwrite host secrets', async () => {
      await client.setApiKey(KEYRING_SERVICES.PIXABAY, 'valid_host_pixabay_key');

      const tokenPlugin = await client.acquirePluginToken('rogue_plugin');

      expect(
        client.setSecret(
          KEYRING_SERVICES.PIXABAY,
          'api_key',
          'overwritten_by_plugin',
          { 'X-Plugin-Token': tokenPlugin }
        )
      ).rejects.toThrow('cannot access namespace');

      expect(await client.getApiKey(KEYRING_SERVICES.PIXABAY)).toBe('valid_host_pixabay_key');
    });

    test('rejects invalid or forged plugin tokens with 401 Unauthorized', async () => {
      expect(
        client.setSecret(
          'OpenSanctuary:Plugin:legit_plugin',
          'api_key',
          'secret',
          { 'X-Plugin-Token': 'forged_garbage_token' }
        )
      ).rejects.toThrow('Invalid plugin token');
    });
  });
});
