import { describe, expect, test, beforeEach } from "bun:test";
import { PluginManager } from "../src/core/plugins.ts";

describe("Frontend PluginManager & Error Sandbox", () => {
    let mockGlobal: any;
    let manager: PluginManager;

    beforeEach(() => {
        mockGlobal = { OS: {} };
        manager = new PluginManager(mockGlobal);
    });

    test("binds plugins instance to mock OS runtime context", () => {
        expect(mockGlobal.OS.plugins).toBe(manager);
    });

    test("successfully registers and initializes a well-behaved plugin", () => {
        let initialized = false;
        let receivedContext: any = null;

        const plugin = {
            name: "Test StreamDeck Bridge",
            init: (OS: any) => {
                initialized = true;
                receivedContext = OS;
            }
        };

        const success = manager.registerPlugin("streamdeck", plugin);
        expect(success).toBe(true);
        expect(initialized).toBe(true);
        expect(manager.hasPlugin("streamdeck")).toBe(true);
        expect(manager.getPlugin("streamdeck")).toBe(plugin);
        expect(manager.listPlugins()).toContain("streamdeck");
    });

    test("sandboxes plugins that throw exceptions in init() without crashing host", () => {
        const panickingPlugin = {
            name: "Faulty Plugin",
            init: () => {
                throw new Error("Simulated plugin crash on startup!");
            }
        };

        // Should return false and not throw an uncaught exception
        const success = manager.registerPlugin("faulty", panickingPlugin);
        expect(success).toBe(false);

        // Host manager continues to function properly
        const normalPlugin = {
            name: "Healthy Plugin",
            init: () => {}
        };
        expect(manager.registerPlugin("healthy", normalPlugin)).toBe(true);
        expect(manager.hasPlugin("healthy")).toBe(true);
    });

    test("injects pre-scoped pluginKeyring and global keyring into plugin init context", () => {
        let capturedCtx: any = null;

        const plugin = {
            name: "CloudSyncPlugin",
            init: (ctx: any) => {
                capturedCtx = ctx;
            }
        };

        const ok = manager.registerPlugin("cloud_sync", plugin);
        expect(ok).toBe(true);
        expect(capturedCtx).not.toBeNull();
        expect(capturedCtx.keyring).toBeDefined();
        expect(capturedCtx.pluginKeyring).toBeDefined();
        expect(capturedCtx.pluginKeyring.service).toBe("OpenSanctuary:Plugin:cloud_sync");
        expect(capturedCtx.keyring.service).toBe("OpenSanctuary:Plugin:cloud_sync");
        expect(capturedCtx.keyring).toBe(capturedCtx.pluginKeyring);
        expect(typeof capturedCtx.pluginKeyring.setApiKey).toBe("function");
        expect(typeof capturedCtx.pluginKeyring.getApiKey).toBe("function");
        expect(typeof capturedCtx.pluginKeyring.setUserCredentials).toBe("function");
    });

    test("clears plugin registry cleanly", () => {
        manager.registerPlugin("p1", { init: () => {} });
        manager.registerPlugin("p2", { init: () => {} });
        expect(manager.listPlugins().length).toBe(2);

        manager.clear();
        expect(manager.listPlugins().length).toBe(0);
        expect(manager.hasPlugin("p1")).toBe(false);
    });
});
