# OS-Next Plugin System

_Last edited: 2026-09-16 20:04_

OS-Next has **two separate, independent plugin systems** — one in the Rust backend, one in the
TypeScript frontend. They don't share code, a manifest format, or a lifecycle. Pick the one that
matches what you're extending: intercept/observe the show engine (backend), or extend the
operator UI (frontend).

There's also a narrower, unrelated plugin-shaped concept — `BibleProvider` in
`src/storage/bible_providers.rs` — for pluggable *Bible data sources* only (see its own doc
comments). It is not a general extension point and isn't covered here.

`MediaSearchProvider` (`src/storage/media_search_providers.rs`, built-in `PexelsProvider` and
`PixabayProvider`) follows the same pattern as `BibleProvider` — a stateless trait + registry
for live external API integrations that need a query and a per-provider API key, which neither
`OsPlugin` (synchronous, no query, dylib-only) nor the frontend `PluginManager` fit well. It backs
the Media tab's "Online Images" search (`GET /api/media/online/search`) and isn't a general
extension point either.

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
