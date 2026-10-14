# OS-Next Plugin System

_Last edited: 2026-09-16 20:04_

OS-Next has **two separate, independent plugin systems** — one in the Rust backend, one in the
TypeScript frontend. They don't share code, a manifest format, or a lifecycle. Pick the one that
matches what you're extending: intercept/observe the show engine (backend), or extend the
operator UI (frontend).

Online Bible downloads and passage lookup used to be a built-in `BibleProvider` registry; they are now
the **Online Bibles** plugin (`web/plugins/online_bibles/`, see "Sandboxed web plugins" below).

`MediaSearchProvider` (`src/storage/media_search_providers.rs`, built-in `PexelsProvider` and
`PixabayProvider`) is a stateless trait + registry
for live external API integrations that need a query and a per-provider API key, which neither
`OsPlugin` (synchronous, no query, dylib-only) nor the frontend `PluginManager` fit well. It backs
the Media tab's "Online Images" search (`GET /api/media/online/search`) and isn't a general
extension point either.

---

## Permissions (read this first)

A plugin does nothing until the operator approves it in **Settings > Plugins**. The full
design, threat model and remaining phases are in [PLUGIN_PERMISSIONS.md](PLUGIN_PERMISSIONS.md).

- **What is enforced.** A backend plugin's hooks are only called if it was granted the
  matching permission: `commands.intercept` (`on_command`), `show.observe`
  (`on_state_change`), `resources.provide` for a category (`provide_resources`). A web
  plugin script is only served from `/plugins/` while approved, and its keyring token is only
  minted if it was granted `keyring.own`. Emergency commands (`ToggleBlackout`,
  `ToggleClearText`, `ToggleLogo`, `SetAlert`) can never be vetoed, decided on the parsed
  command, not on text that merely mentions them.
- **What is not.** A native library runs inside the server with its full privileges, so
  approving one is a trust decision: the list of permissions describes what the plugin
  *says* it will do, and `network.*`, `filesystem.*` and `process.*` are shown as "declared,
  not enforced". A web plugin that runs in the console page itself (`ui.dom`) has the same
  access as the operator.
- **Pinning.** Approval is pinned to a SHA-256 of what was reviewed. If the file changes it
  stops running (`changed`) until it is reviewed again. Only a console (the host token or a
  persistent console credential) can approve, deny or revoke; paired displays, remotes and
  plugin tokens cannot. Every decision is recorded in the audit log.
- **Manifest.** Put a `plugin.json` next to the plugin (see `web/plugins/hello_world.plugin.json`):
  `manifest_version`, `id`, `name`, `permissions` (each with an optional `reason`), and a
  `backend.libraries` map of `"<os>-<arch>"` to `{path, sha256}` for a native plugin, laid out as
  `<plugins dir>/<id>/plugin.json` + the library, or a `web.entry` for a console script. Unknown
  permissions are rejected, never ignored.
- **ABI 2 (`PluginHost`).** A native plugin built with `os_next::export_plugin!` gets a host
  handle in its constructor instead of reaching around the server. Each call is checked against
  its live grant: `snapshot()` (`show.observe`), `send_command()` (`commands.send`, only the
  granted command names, rate-limited, queued and run through the normal veto path, never
  inline), `library_list()` (`library.read`), `keyring_*` (`keyring.own`), `data_dir()`
  (`filesystem.plugin_data`). See `examples/plugin_observer.rs`, a real plugin used as the
  template and by the dlopen test. The interface is a Rust trait object, so the plugin must be
  built with the same compiler and OS-Next version as the host; the host refuses a library whose
  `OS_PLUGIN_HOST_VERSION` differs. The frozen v1 `OsPlugin` trait never changes.
  **Update a plugin by replacing the file, not overwriting it in place:** the loaded library is
  memory-mapped, and truncating it crashes the server.
- **Sandboxed web plugins** (`web.sandbox: true`, a folder `web/plugins/<id>/` with `plugin.json`
  and an HTML entry page; see `web/plugins/sandbox_demo/`). The page runs in an iframe with
  `sandbox="allow-scripts"` and no `allow-same-origin`, so it has an opaque origin: no access to the
  console, its storage, the host token or the console credential. The server serves it from
  `/plugin-frame/<id>/` only while approved and the whole folder still hashes to what was reviewed
  (symlinks and oversized folders are refused), with a CSP that forbids everything except the
  `network.outbound` hosts it was granted. It talks to the console only through `postMessage`
  (`getState`, `sendCommand`, `keyring.*`, `net.fetch`, `library.addSong`, `ui.resize`, `log`, and a
  `state` event with `show.observe`); the console's bridge checks each request against the grant and holds the
  plugin's scoped token, and the server re-checks commands and keyring access against that token.
  This is the only plugin type where the permission list is actually enforced end to end; prefer it
  over a same-realm script (`ui.dom`) whenever the plugin only needs a panel and a few actions.
- **Network and library access for a sandboxed plugin.** `net.fetch {url}` (needs
  `network.outbound` with a `hosts` list) makes the *server* fetch the page, because the plugin
  can't connect anywhere itself and a browser couldn't read a cross-origin page anyway. The host
  allows https only, no credentials in the URL, only the granted hosts (`example.com`,
  `*.example.com`, optionally `:port`), and only addresses that resolve to public IPs (the client is
  pinned to the address it checked); every redirect hop is re-checked, bodies are capped at 2 MB, and
  a refusal is audited. `library.addSong {song}` (needs `library.write` on `songs`) stores a song; the
  host mints its id and applies size limits. A sandboxed frame can't submit a `<form>`, so use buttons.
- **Online Bibles is a plugin.** Downloading translations (ChurchApps / Bolls.life / the OpenLP community
  list, and any public GitHub repository laid out like `arron-taylor/bible-versions`) and looking up a
  passage or a whole book (bible-api.com) is `web/plugins/online_bibles/`. It asks for those five hosts,
  `library.write` on `scriptures` (`library.addScriptures`: the host derives each chapter's id and verse
  range, so re-downloading a translation replaces its chapters; the same grant can therefore also replace
  chapters of a translation you already have, which its review text says) and optionally `AddToSchedule`.
  The first-run wizard's Bible step turns it on for you, listing what it can reach.
- **Where a sandboxed plugin's page appears, and how it looks.** A manifest's `ui.placements` choose:
  `library-source` (a "plugin" entry in a Resource Library tab's sidebar, shown in place of the list
  and preview, as Genius/Online Bibles/the downloader do), `video-picker` (the background picker's
  Videos tab) or `dock` (a small floating panel, the default). The page loads `__os/theme.css` and
  `__os/sdk.js` (served by the host to any approved sandboxed plugin): the console sends its design
  tokens (`--os-*`) to the frame and again whenever the theme changes, and the stylesheet gives
  `.btn`, inputs, `.os-list`, `.os-tabs`, `.os-badge`, `.card` and so on the console's own look, so
  a plugin built from them is indistinguishable from built-in UI. **Icons, not emoji:** a page that
  loads `__os/icons.js` and then `__os/sdk.js` gets the console's own SVG icons exactly as the console
  does: emoji that have an icon in `web/icons/emoji-map.json` (and `{icon:name}` tokens) are swapped
  for `<img class="ui-icon">` pointing at `__os/icons/<set>/<name>.svg`, the file in the current
  icon set, and follow the operator's icon-set choice. `OS.icon(name)` returns the markup for one.
  Text that can't hold an image (`placeholder=`, `title=`, `<option>`) stays plain. Need an icon that
  doesn't exist? Add it to `web/icons/crimson/` and run `bun icons/derive_sets.ts` (see
  `web/icons/README.md`), then map its emoji in `emoji-map.json`. `OS.call(method, params)` is the
  bridge; `OS.on('state', ...)` the event stream; `OS.slot` says where the page is shown.
- **Running a command-line tool: `process.run`.** A sandboxed plugin can't run programs, but it can
  declare in `plugin.json` the exact commands it needs (`commands`: the tool, an argument template
  and typed parameters) and the tools they use (`tools`: a GitHub release, the asset per OS, the
  checksums asset, its detached OpenPGP signature, and the public key's file in the plugin folder).
  The review screen shows each command line and each tool's source and key fingerprint. The host
  installs and updates the tool (`tool.status` / `tool.update`, signature- then checksum-verified),
  and runs a declared command on request (`process.start`, `process.poll`, `process.cancel`,
  `process.release`): no shell, parameters validated by type (`url` http(s) only, `enum`, `bool`,
  bounded `string`), a value used as a whole argument can't start with `-`, one throwaway job folder
  per run, at most two jobs per plugin, output kept as capped lines. A finished file reaches the
  library only through `library.addMedia`, which accepts a plain file name from *that job's* folder
  (right extension, moved not copied). What the tool itself then does (for example reaching the
  network) is not confined, which is exactly what the Critical-tier `process.run` grant tells you.
- **Notifications: `notifications.post`.** The console has one notification center (the bell in
  the ribbon, with an unread badge) that collects everything: its own toasts, notices from the host
  (for example a finished tool update) and notifications from plugins. A sandboxed plugin granted
  `notifications.post` calls `OS.notify(title, message, { level, key, sticky })`
  (`level`: `info`, `success`, `warning`, `error`). The host stamps the source with the plugin's
  name, so a plugin can't pose as the host or another plugin. Text is plain (no markup), clamped
  to 80 / 400 characters, and a plugin is limited to 10 per 10 seconds. A later post with the same
  `key` replaces the earlier one (progress, "still failing") instead of stacking; `sticky` keeps a
  warning or error on screen until dismissed. The operator can mute a source (it still lands in the
  list, with no pop-up), dismiss entries or clear them all. Notifications are kept in memory, the
  last 200, and are not an audit trail. Host side: `src/api/notifications.rs` (the bus; the host's
  own code posts with `post_host`), routes in `src/api/routes/notifications.rs`; the console
  polls `GET /api/notifications?after=<id>` every 2 seconds (`&wait=<s>` makes the host hold the
  request until something new arrives, for a client that wants it). Native plugins don't have this call yet.
- **Stock Images is a plugin.** Searching Pexels and Pixabay (the Media library's "Online Images")
  is `web/plugins/stock_media/`, one plugin for both sites: a sandboxed page in the Media library
  sidebar with a search box, a source toggle per site, a thumbnail grid and an "API keys" section.
  You use your own free key from each site; the plugin keeps them in its own keyring namespace
  (`keyring.own`), so they are no longer Options settings and the old built-in search and import
  routes are gone (keys entered there before are not carried over: paste them into the plugin once).
  It asks for the two search APIs and their image hosts (`api.pexels.com`, `images.pexels.com`,
  `pixabay.com`, `cdn.pixabay.com`), `library.write` on media, and optionally `notifications.post`.
  Three host features exist for it, available to any plugin with the matching grant:
  `net.fetch` takes optional `headers` (only `Authorization`, `Accept`, `Accept-Language`; Pexels
  wants its key in `Authorization`); `library.addMediaUrl { url, title, id? }` has the *host*
  download a picture or video from a granted host (https, public addresses, 64 MB cap) into the
  Media library, with the file type taken from the downloaded bytes and the name chosen by the
  host; and a plugin frame's `img-src` allows `https://` pictures from the hosts it was granted
  for `network.outbound` (nothing new beyond what `connect-src` already allowed), so thumbnails
  load. A refused fetch is audited without its query string, because Pixabay puts the key there.
- **Video Downloader is a plugin.** Downloading with yt-dlp (YouTube and 1000+ other sites, optional
  browser cookies, SponsorBlock, audio-only) is `web/plugins/ytdlp/`: a sandboxed page in the Media
  library sidebar and in the video picker, built on `process.run` above. The old built-in downloader,
  updater and routes are gone.
- **Genius lyrics is a plugin.** Searching Genius and importing lyrics used to be built into the
  server and the Song Library panel; it is now `web/plugins/genius_lyrics/` (a sandboxed panel that
  asks only for `genius.com`, `library.write` on songs, and optionally `AddToSchedule`/`GoLive`). Approve
  it in Settings > Plugins and it appears as a panel. It is also the worked example of a plugin that
  needs the network.
- **Existing plugins.** Loose `.so`/`.dll`/`.dylib` files already installed when permissions
  shipped are approved once with the legacy v1 permission set and flagged for review; anything
  that appears afterwards starts as pending. A web script with no manifest is treated as asking
  for full console access.

---

## Backend plugins (Rust, native dynamic libraries)

**What a plugin is:** a native shared library (`.so` on Linux, `.dll` on Windows, `.dylib` on
macOS) that exports one C symbol, loaded into the server process via
[`libloading`](https://docs.rs/libloading).

**Where it lives:** `src/core/plugins.rs`.

### The trait

```rust
pub trait OsPlugin: Send + Sync {
    fn name(&self) -> &'static str;
    fn on_state_change(&self, state: &crate::core::models::ShowState);
    fn on_command(&self, cmd: &str) -> bool; // Return false to intercept/block

    // Optional — see "Providing Resource Library items" below. Defaults to
    // contributing nothing, so existing plugins compile and run unchanged.
    fn provide_resources(&self, category: &str) -> Vec<serde_json::Value> { Vec::new() }
}
```

- `on_command` is called with the **raw incoming JSON command text**, *before* it's deserialized
  or executed. Return `false` to veto the command — it's dropped entirely. Every loaded plugin
  gets a vote; any single `false` blocks it.
- `on_state_change` is called *after* a command has executed, with the resulting `ShowState`
  snapshot. Observation only — it can't change anything.
- Matching on `cmd` today is a plain substring check (`cmd.contains(...)`) against the raw JSON,
  not a real parse. Match on the command's `event_type`/`data` tag text accordingly.

### Safety-critical bypass

`PluginManager::is_safety_critical_command` (`plugins.rs:68`) hardcodes four commands that no
plugin can ever veto: `ToggleBlackout`, `ToggleClearText`, `ToggleLogo`, `SetAlert`. These always
pass through regardless of what any plugin's `on_command` returns.

### Sandboxing

Every call into a plugin — both hooks — is wrapped in `std::panic::catch_unwind`. A panicking
plugin is logged via `tracing::error!` and otherwise ignored: the command is permitted, or the
state-change notification is just skipped for that plugin. **The host never crashes.** This is
exercised by `tests/core_tests.rs::test_backend_plugin_manager_lifecycle_and_interception`.

### Loading and registration

- `PluginManager::load_plugins_from_dir(dir)` creates `dir` if it doesn't exist, then scans it
  (non-recursively) for `.so`/`.dll`/`.dylib` files and loads each one.
- `main.rs` wires this to `~/.config/OpenSanctuary/plugins/`, scanned **once at server startup**.
  There is no hot-reload and no runtime add/remove API — drop a plugin file in that directory and
  restart the server.
- `load_plugin` expects the library to export exactly:
  ```rust
  #[no_mangle]
  pub extern "C" fn create_plugin() -> *mut dyn OsPlugin { ... }
  ```
  The manager calls it, wraps the raw pointer in a `Box`, and keeps the `Library` handle alive for
  the process's lifetime (required — dropping it would invalidate the vtable).
- `register_plugin(Box<dyn OsPlugin>)` registers an already-constructed plugin in-process, with no
  dynamic-library loading. This is the path integration tests use, and also a valid way to embed a
  first-party plugin directly into the binary instead of shipping it as a separate `.so`.

### How to write one

1. New crate, `crate-type = ["cdylib"]` in `Cargo.toml`, depending on `os-next` as a library for
   the `OsPlugin` trait and `ShowState` type.
2. Implement `OsPlugin`, then export the constructor:
   ```rust
   struct MyPlugin;
   impl OsPlugin for MyPlugin {
       fn name(&self) -> &'static str { "My Plugin" }
       fn on_state_change(&self, state: &ShowState) { /* e.g. drive DMX/lighting off state.is_blackout */ }
       fn on_command(&self, cmd: &str) -> bool { true /* observe-only; veto by returning false */ }
   }

   #[no_mangle]
   pub extern "C" fn create_plugin() -> *mut dyn OsPlugin {
       Box::into_raw(Box::new(MyPlugin))
   }
   ```
3. Build it, copy the resulting `.so`/`.dll`/`.dylib` into `~/.config/OpenSanctuary/plugins/`,
   restart the server.

`tests/core_tests.rs` (`TestDmxPlugin`, `PanickingPlugin` in the same test) is the best in-repo
reference, though it's registered in-process via `register_plugin` rather than built as a real
cdylib.

### Providing Resource Library items (Songs, Scriptures, Media, Presentations, Themes)

A backend plugin can contribute rows into the operator's Resource Library tabs by overriding
`provide_resources(category) -> Vec<serde_json::Value>`. `category` is one of the five tab/route
names — `"songs"`, `"scriptures"`, `"media"`, `"presentations"`, `"themes"` — and is called once
per `GET /api/<category>` request. Return an empty `Vec` (the default) for any category this
plugin doesn't provide.

- **Shape:** each returned `Value` should be a JSON object shaped like that category's normal
  database row (e.g. a `Song`-shaped object for `"songs"`: `{"id", "title", "author", "slides", ...}`
  — see the struct definitions in `src/core/models.rs`). Using plain JSON rather than the real Rust
  structs means a plugin isn't coupled to OS-Next's exact internal type definitions — the frontend
  consumes these as loosely-typed JS objects, so only the fields a given tab's rendering actually
  reads need to be present (check the relevant branch in `web/src/app_core.ts`'s catalog rendering
  for what it expects). At minimum, give every item a unique `id` distinct
  from anything already in the database, so schedule/undo logic (which tracks items by ID) doesn't
  collide with a real library row.
- **Merging:** contributed items are appended *after* the database's own rows, unfiltered, sorted,
  or deduplicated against them — a plugin is responsible for not re-offering something already in
  the local library if that matters for its use case.
- **Search:** when the operator has an active search query in a tab, plugin-contributed items are
  filtered by a simple case-insensitive substring match over each item's serialized JSON (not the
  richer per-field matching the database's own `search_songs`/`search_scriptures` use) — so a
  query matches if it appears anywhere in the item's JSON, including field names. `"media"`,
  `"presentations"`, and `"themes"` don't support search at the route level at all (their
  `GET` handlers take no query), so plugin items for those three always appear unfiltered.
- **Sandboxing:** same as the other two hooks — wrapped in `catch_unwind`; a panicking
  `provide_resources` call is logged and contributes nothing for that request, it doesn't fail the
  whole `GET`.

Example — a plugin that surfaces songs from an external catalog:

```rust
impl OsPlugin for MyLibraryPlugin {
    fn name(&self) -> &'static str { "My External Song Library" }
    fn on_state_change(&self, _state: &ShowState) {}
    fn on_command(&self, _cmd: &str) -> bool { true }

    fn provide_resources(&self, category: &str) -> Vec<serde_json::Value> {
        if category != "songs" { return Vec::new(); }
        self.fetch_catalog() // however this plugin sources its data
            .into_iter()
            .map(|song| serde_json::json!({
                "id": format!("mylib_{}", song.id),
                "title": song.title,
                "author": song.author,
            }))
            .collect()
    }
}
```

---

## Frontend plugins (TypeScript/JS, dynamic ES modules)

**What a plugin is:** a plain object with a `name` and an `init(OS)` function, loaded via a
dynamic `import()` of an ES module and run in the same page/origin as the app — full DOM and
`window` access, no formal sandbox beyond catching a throwing `init`.

**Where it lives:** `web/src/core/plugins.ts`.

### The shape

There's no declared TypeScript interface (the registry is `Map<string, any>`); by convention a
plugin module's default export looks like:

```ts
export default {
    name: "My Plugin",
    init: (OS) => {
        // one-shot setup: DOM manipulation, event listeners, etc.
    }
};
```

### The `OS` context object

Passed into `init`. Defined once in `web/src/main.ts`:

```ts
{ plugins: pluginManager, clock: clock, VideoSyncController: VideoSyncController }
```

Nothing else is added to it. In practice, most of the app's internals a plugin would want
(`sendCommand`, `currentSnapshot`, various DOM element references) are separately exposed on
`window`/`globalThis` by `app_core.ts`, outside the `OS` namespace — a plugin can reach them
directly, there's just no curated/stable API surface for it yet.

### Lifecycle and sandboxing

There is **no command-interception or state-change hook** on the frontend (unlike the backend) —
a plugin gets exactly one call, `init(OS)`, at load time. `registerPlugin` wraps that call in a
`try/catch`: a throwing `init` is caught and logged, registration returns `false`, and the
`PluginManager` keeps working normally. Covered by `web/tests/plugins_runtime.test.ts`.

### Loading and registration

- `PluginManager` is a module-level singleton constructed once in `main.ts`, which stamps itself
  onto `window.OS.plugins` at construction time.
- `main.ts` calls `pluginManager.loadLocalPlugin("/plugins/hello_world.js")` once at startup — a
  single hardcoded path, not a directory scan or manifest.
- `loadLocalPlugin(path)` does `await import(path)`; if `module.default.init` is a function, it
  registers it under `module.default.name || path`.
- `registerPlugin(name, plugin, context?)` is also public — call it directly (e.g. from the
  console, or from another already-loaded module) to register a plugin without going through
  `loadLocalPlugin`/a file path at all.

### How to write one

`web/plugins/hello_world.js` is a real, working, in-repo example (also present in the packaged
Windows build) — the canonical template:

```js
export default {
    name: "Hello World Plugin",
    init: (OS) => {
        console.log("Hello World Plugin initialized!");
        const toolbar = document.querySelector('.main-header');
        if (toolbar) {
            const btn = document.createElement('button');
            btn.className = 'btn';
            btn.textContent = '🌟 Plugin Button';
            btn.onclick = () => alert("Hello from the dynamically loaded TypeScript/JS Plugin!");
            toolbar.appendChild(btn);
        }
    }
};
```

To ship a new one: drop a `.js` ES module under `web/plugins/`, then either add a
`pluginManager.loadLocalPlugin("/plugins/your_plugin.js")` call in `main.ts` (matching the
existing hard-coded pattern), or call `loadLocalPlugin`/`registerPlugin` from anywhere else that
runs after the app has booted.

---

## Choosing between them

| | Backend (`OsPlugin`) | Frontend (`PluginManager`) |
|---|---|---|
| Can veto/block a command | Yes (`on_command` → `false`) | No |
| Observes engine state | Yes (`on_state_change`) | No hook — poll `window` internals yourself |
| Provides Resource Library items | Yes (`provide_resources`) | No |
| Extends the operator UI | No | Yes (full DOM access) |
| Sandboxing | `catch_unwind` per call, every call | `try/catch` around `init` only |
| Load timing | Once at server startup, from a config directory | Once at page load, from a hardcoded path list |
| Distribution | Compiled native library per OS | Plain `.js` file |
