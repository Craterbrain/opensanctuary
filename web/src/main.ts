// OpenSanctuary Modular Frontend
// Main Entry Point

import { PluginManager } from "./core/plugins.ts";
import { clock } from "./core/timesync.ts";
import { VideoSyncController } from "./core/video_sync.ts";
import { keyring } from "./core/keyring.ts";
import { startEmojiIcons } from "./core/emoji_icons.ts";

// 0. Swap emoji for the themed SVG icons (static markup now, rendered content as it appears)
startEmojiIcons();

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

    // 4. Load demo plugin to prove modular architecture works!
    pluginManager.loadLocalPlugin("/plugins/hello_world.js");
});
