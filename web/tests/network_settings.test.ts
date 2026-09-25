import { describe, test, expect, beforeEach } from 'bun:test';
import { SETTINGS_CATEGORIES, SETTINGS_SCHEMA } from '../src/core/settings_schema.ts';
import {
  saveNetworkSettings,
  type NetworkInterfaceItem,
  type NetworkState,
} from '../src/ui/settings_dialog.ts';

describe('Network Settings Schema & Management', () => {
  test('SETTINGS_CATEGORIES includes network category', () => {
    const netCat = SETTINGS_CATEGORIES.find(c => c.id === 'network');
    expect(netCat).toBeDefined();
    expect(netCat?.label).toBe('Network');
    expect(netCat?.icon).toBe('🌐');
  });

  test('SETTINGS_SCHEMA includes networkHostname, networkPort, networkDedicatedMac, networkBroadcastAll', () => {
    const hostnameDef = SETTINGS_SCHEMA.find(s => s.key === 'networkHostname');
    expect(hostnameDef).toBeDefined();
    expect(hostnameDef?.category).toBe('network');
    expect(hostnameDef?.label).toContain('DHCP Option 12');

    const portDef = SETTINGS_SCHEMA.find(s => s.key === 'networkPort');
    expect(portDef).toBeDefined();
    expect(portDef?.category).toBe('network');

    const macDef = SETTINGS_SCHEMA.find(s => s.key === 'networkDedicatedMac');
    expect(macDef).toBeDefined();
    expect(macDef?.category).toBe('network');

    const bcastDef = SETTINGS_SCHEMA.find(s => s.key === 'networkBroadcastAll');
    expect(bcastDef).toBeDefined();
    expect(bcastDef?.category).toBe('network');
  });

  test('saveNetworkSettings persists to /api/settings and /api/network/interfaces', async () => {
    const mockElements: Record<string, any> = {
      'network-hostname-input': { value: 'sanctuary-main' },
      'network-port-input': { value: '8095' },
      'network-broadcast-all-toggle': { checked: true },
    };

    const mockCheckboxes = [
      { checked: true, dataset: { iface: 'eth0' } },
      { checked: false, dataset: { iface: 'wlan0' } },
    ];

    const originalDoc = (globalThis as any).document;
    (globalThis as any).document = {
      getElementById: (id: string) => mockElements[id] || null,
      querySelectorAll: (sel: string) => (sel === '.network-iface-checkbox' ? mockCheckboxes : []),
      createElement: () => ({
        className: '',
        innerHTML: '',
        textContent: '',
        style: {},
        appendChild: () => {},
        addEventListener: () => {},
      }),
    };

    const apiCalls: { url: string; method: string; body: any }[] = [];

    const originalFetch = globalThis.fetch;
    globalThis.fetch = (async (input: any, init?: any) => {
      const url = typeof input === 'string' ? input : input.url;
      const method = init?.method || 'GET';
      const body = init?.body ? JSON.parse(init.body) : null;
      apiCalls.push({ url, method, body });
      return {
        ok: true,
        json: async () => ({ success: true }),
      } as any;
    }) as any;

    try {
      const success = await saveNetworkSettings();
      expect(success).toBe(true);

      // Verify POST /api/settings
      const settingsCall = apiCalls.find(c => c.url === '/api/settings');
      expect(settingsCall).toBeDefined();
      expect(settingsCall?.method).toBe('POST');
      expect(settingsCall?.body.networkHostname).toBe('sanctuary-main');
      expect(settingsCall?.body.networkPort).toBe('8095');
      expect(settingsCall?.body.networkBroadcastAll).toBe('true');

      // Verify POST /api/network/interfaces
      const ifaceCall = apiCalls.find(c => c.url === '/api/network/interfaces');
      expect(ifaceCall).toBeDefined();
      expect(ifaceCall?.method).toBe('POST');
      expect(ifaceCall?.body.enabled_interfaces).toEqual(['eth0']);
      expect(ifaceCall?.body.broadcast_all).toBe(true);
    } finally {
      globalThis.fetch = originalFetch;
      (globalThis as any).document = originalDoc;
    }
  });
});
