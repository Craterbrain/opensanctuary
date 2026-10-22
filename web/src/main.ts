// OpenSanctuary Modular Frontend
// Main Entry Point

import { PluginManager } from "./core/plugins.ts";
import { clock } from "./core/timesync.ts";
import { VideoSyncController } from "./core/video_sync.ts";
import { keyring } from "./core/keyring.ts";
import { resolveHostSessionToken } from "./core/host_session.ts";
import { startSandboxedPlugin, stopSandboxedPlugin, isSandboxedPluginRunning } from "./core/plugin_bridge.ts";
import { refreshPluginHints } from "./core/plugin_hints.ts";
import { startEmojiIcons } from "./core/emoji_icons.ts";
import { previousSessionHung, markPluginsStarting, watchConsoleHealth, showSafeModeBanner } from "./core/plugin_guard.ts";
import { openSettingsAt } from "./ui/settings_dialog.ts";
import { initNotificationBell, startNotificationFeed } from "./ui/notification_center.ts";

// 0. Swap emoji for the themed SVG icons (static markup now, rendered content as it appears)
startEmojiIcons();

// A plugin was approved/revoked in Settings > Plugins (or the first-run wizard): a sandboxed
// plugin starts or stops right away. (A same-realm console script needs a page reload.)
window.addEventListener("os-plugin-changed", (ev) => {
  void refreshPluginHints();
  const plugin = (ev as CustomEvent).detail?.plugin;
  if (!plugin || plugin.kind !== "sandbox") return;
  if (plugin.state === "active") {
    if (!isSandboxedPluginRunning(plugin.id)) startSandboxedPlugin(plugin);
  } else {
    stopSandboxedPlugin(plugin.id);
  }
});

// 1. Initialize Plugin System
export const pluginManager = new PluginManager();

// Expose core modules globally for legacy app migration
(window as any).OS = {
    plugins: pluginManager,
    clock: clock,
    VideoSyncController: VideoSyncController,
    keyring: keyring,
};

// 2. Synchronize Network Clocks (NTP-style)
clock.synchronize(5).then(async () => {
    // 3. Load Core Application after time is synchronized
    //    app_core.ts  — engine, state, schedule, rendering (~4448 lines)
    //    app_ui.ts    — search, keyboard, menus, editors, importers (~2775 lines)
    await import("./app_core.ts");
    await import("./app_ui.ts");
    (window as any).__APP_READY__ = true;

    // 4. Load only the web plugins the operator has approved (Settings > Plugins). The
    //    server also refuses to serve any other file under /plugins/.
    (async () => {
        try {
            await resolveHostSessionToken();
            // Notifications from the app and plugins (console pages only; others get a 403 and stop)
            const bell = document.getElementById("btn-notifications");
            if (bell) { initNotificationBell(bell); void startNotificationFeed(); }
            const res = await fetch("/api/plugins?kind=web&active=1");
            if (!res.ok) return; // no console credential on this page (live/stage/remote)
            // Installed plugins not turned on yet: hints in the places they'd appear (core/plugin_hints.ts).
            void refreshPluginHints();
            const list = await res.json();
            const approved = (Array.isArray(list) ? list : []).filter((p: any) => p && typeof p.id === "string");
            const startAll = async () => {
                if (approved.length) { markPluginsStarting(); watchConsoleHealth(); }
                for (const p of approved) {
                    if (p.kind === "sandbox") startSandboxedPlugin(p);
                    else await pluginManager.loadLocalPlugin(`/plugins/${p.id}.js`, p.id);
                }
            };
            // The last session ended without ever becoming healthy: a plugin may have hung the console.
            // Keep every plugin paused until the operator says otherwise (nothing is revoked).
            if (approved.length && previousSessionHung()) {
                showSafeModeBanner(() => { void startAll(); }, () => openSettingsAt("plugins"));
            } else {
                await startAll();
            }
        } catch (e) {
            console.warn("[Plugins] could not load approved plugins:", e);
        }
    })();
});
