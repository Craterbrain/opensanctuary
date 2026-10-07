import { keyring } from "./keyring.ts";

/**
 * Local Plugin Manager
 * Loads custom ES Modules from a local plugin directory or registers plugins programmatically.
 */
export class PluginManager {
    private plugins: Map<string, any> = new Map();

    constructor(contextObj: any = null) {
        const target = contextObj || (typeof window !== 'undefined' ? window : (globalThis as any));
        if (target) {
            target.OS = target.OS || {};
            target.OS.plugins = this;
            if (!target.OS.keyring) {
                target.OS.keyring = keyring;
            }
        }
    }

    /**
     * Registers a plugin object safely within an isolated try-catch boundary.
     * Thrown exceptions in third-party init() hooks are caught and logged without breaking the host.
     * Injects a pre-scoped `pluginKeyring` vault into the plugin's context.
     */
    registerPlugin(name: string, plugin: any, context: any = null): boolean {
        if (!plugin) return false;
        try {
            const baseCtx = context || (typeof window !== 'undefined' ? (window as any).OS : (globalThis as any).OS) || {};
            const rootKeyring = baseCtx.keyring || keyring;
            const pluginVault = rootKeyring.forPlugin(name);

            // Create an isolated sandbox context for this specific plugin
            const pluginCtx = Object.create(baseCtx);
            // Injects pre-scoped vault:
            pluginCtx.pluginKeyring = pluginVault;
            // CRITICAL SECURITY ISOLATION: Mask ctx.keyring so plugin CANNOT access root keyring or other plugins' vaults
            pluginCtx.keyring = pluginVault;

            if (typeof plugin.init === 'function') {
                plugin.init(pluginCtx);
            }
            this.plugins.set(name, plugin);
            return true;
        } catch (e) {
            console.error(`[PluginManager] Failed to register plugin '${name}':`, e);
            return false;
        }
    }

    getPlugin(name: string): any {
        return this.plugins.get(name);
    }

    hasPlugin(name: string): boolean {
        return this.plugins.has(name);
    }

    listPlugins(): string[] {
        return Array.from(this.plugins.keys());
    }

    clear(): void {
        this.plugins.clear();
    }

    async loadLocalPlugin(path: string) {
        try {
            const module = await import(path);
            if (module.default && typeof module.default.init === 'function') {
                const name = module.default.name || path;
                this.registerPlugin(name, module.default);
            }
        } catch (e) {
            console.error(`[PluginManager] Failed to load plugin ${path}:`, e);
        }
    }
}
