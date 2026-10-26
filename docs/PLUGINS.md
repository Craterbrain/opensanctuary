# OS-Next Plugin System

_Last edited: 2026-10-22_

A plugin extends OpenSanctuary in one of two ways, and **nothing runs until the operator approves it**
in Settings > Plugins:

- **Sandboxed web plugins** (the recommended kind): a folder in `web/plugins/` with a `plugin.json` and
  an HTML page that runs in an isolated frame and reaches the app only through a permission-checked
  bridge. Everything below about permissions, placements, the notification center, ribbon buttons, shared
  secrets and running tools is about these. The bundled ones (Song Lyrics, Online Bibles, Video
  Downloader, Stock Images, a demo) are the worked examples.
- **Console scripts** (`ui.dom`): a plain `.js` module that runs inside the operator console's own page with
  the same access as the operator. Critical tier; see "Frontend plugins" at the end.

There are **no native plugins**: loading compiled libraries into the server (the `OsPlugin` trait, the ABI 2
`PluginHost` API, `commands.intercept` / `commands.modify` / `resources.provide`) was removed. Online Bible
downloads, lyrics lookup, video download and stock-photo search are all sandboxed plugins now.

---

## Permissions (read this first)

A plugin does nothing until the operator approves it in **Settings > Plugins**. The full
design, threat model and remaining phases are in [PLUGIN_PERMISSIONS.md](PLUGIN_PERMISSIONS.md).

- **What is enforced.** Every permission a sandboxed plugin holds is enforced by the host: its frame
  can only reach the hosts it was granted (the frame's CSP and the server-side `net.fetch`), every
  bridge call is checked against the live grant, and a scoped token can never carry more than the
  grant. A console script is only served from `/plugins/` while approved, and its keyring token is
  only minted if it was granted `keyring.own`.
- **What is not.** A console script (`ui.dom`) runs in the console page with the same access as the
  operator, so approving one is a trust decision; the list of permissions describes what it *says* it
  will do. That is why sandboxed plugins are the recommended kind.
- **Pinning.** Approval is pinned to a SHA-256 of what was reviewed. If the file changes it
  stops running (`changed`) until it is reviewed again. Only a console (the host token or a
  persistent console credential) can approve, deny or revoke; paired displays, remotes and
  plugin tokens cannot. Every decision is recorded in the audit log.
- **Manifest.** Put a `plugin.json` next to the plugin (see `web/plugins/hello_world.plugin.json` for a
  console script, or any folder in `web/plugins/` for a sandboxed one): `manifest_version`, `id`,
  `name`, `permissions` (each with an optional `reason`), and a `web` section (`entry` and
  `sandbox: true` for a sandboxed page, or the `.js` file name for a console script). A sandboxed plugin
  is a folder `web/plugins/<id>/` holding `plugin.json` and the page; a console script is
  `<id>.js` + `<id>.plugin.json`. Unknown permissions are rejected, never ignored, and a manifest with a
  `backend` section (a native library) is refused.
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
- **Online Bibles is a plugin.** Downloading translations (the project's own public-domain collection,
  [`Craterbrain/opensanctuary-bibles`](https://github.com/Craterbrain/opensanctuary-bibles): 64 translations
  in 30 languages), and any public GitHub repository laid out like `arron-taylor/bible-versions`, and looking up a
  passage or a whole book (bible-api.com) is `web/plugins/online_bibles/`. (Bolls.life, ChurchApps and the curated list were removed in 1.2.0:
  licensed Bibles such as the NIV come from the YouVersion plugin.) It asks for three hosts (`bible-api.com`, `api.github.com`,
  `raw.githubusercontent.com`),
  `library.write` on `scriptures` (`library.addScriptures`: up to 500 chapters per call, each 1-400 verses of up to 16,000 bytes (one
  oversize verse refuses the whole call); the host derives each chapter's id and verse
  range, so re-downloading a translation replaces its chapters; the same grant can therefore also replace
  chapters of a translation you already have, which its review text says) and optionally `AddToSchedule`.
  The first-run wizard's Bible step turns it on for you, listing what it can reach.
- **Where a sandboxed plugin's page appears, and how it looks.** A manifest's `ui.placements` choose:
  `library-source` (a "plugin" entry in a Resource Library tab's sidebar, shown in place of the list
  and preview, as Song Lyrics/Online Bibles/the downloader do), `video-picker` (the background picker's
  Videos tab) or `dock` (a small floating panel, the default). **Placements are also how the console
  advertises a plugin before it's approved:** until the operator approves an installed plugin, each
  `library-source` placement shows a muted "plugin · off" entry with its `label`, a `video-picker`
  placement makes the picker's plugin tab appear with a note, and the Store lists it with its manifest
  `description` -- each opening its review card in Settings > Plugins (`web/src/core/plugin_hints.ts`).
  The console never names a particular plugin, so a plugin that isn't installed leaves no trace, and
  one the operator denied or revoked isn't advertised again. Give a plugin a `description`: it's what
  the Store and the first-time setup show. The page loads `__os/theme.css` and
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
- **Ribbon buttons: `ui.toolbar`.** A sandboxed plugin can declare up to two buttons for the
  console's top ribbon. In `plugin.json`, request `ui.toolbar` and list them under
  `ui.toolbar`: `{ "id", "label", "icon", "tooltip" }`. They are fixed once approved and the review
  screen shows each one with its real icon and label. Rules, so a plugin button can't pass for a
  built-in one: the label is 1 to 16 letters, digits, spaces and `- . & ' ( ) +` (no emoji, control,
  bidi or zero-width characters) and can't be a built-in button's name (Go Live, Black, Clear, Save,
  Notices and so on); the icon is the *name* of a file in `web/icons/crimson/`, never one of the
  live-control icons; unknown keys are rejected. The console draws them in their own **Plugins**
  group (caption, dashed top border, a puzzle badge on each button, and "<plugin name> (plugin): ..."
  in the tooltip), never among the built-in controls; past six buttons across all plugins the rest
  go into a "More" menu. A click arrives as `OS.on('toolbar', ({ id }) => ...)` in exactly one frame of
  the plugin (debounced to one per 500 ms), carries no authority of its own, and the button gives
  up focus so Enter still means Go Live. The plugin can call `OS.call('ui.toolbar.update',
  { id, disabled?, pressed? })` to grey a button out or show it pressed, nothing else. Revoking or
  denying a plugin removes its buttons immediately. See the Sandboxed Demo Panel for a working one.
- **Overriding what a plugin asks for (Advanced).** On any plugin's card in Settings > Plugins,
  "Advanced: override what this plugin asks for" opens an editor, behind a warning that this may
  break the plugin or give it more access than its author intended. Every permission it asked for
  can be switched off (even one it marked required) and its lists (hosts, commands, library
  categories, paths, ports) edited to narrow or widen them, and any permission the host knows
  that it did *not* ask for can be switched on. Applying it needs a tick on "I understand this may
  break the plugin", then sends the exact set to grant (`POST /api/plugins/<id>/approve` with
  `override`; the permission list comes from `GET /api/plugins/permissions`). The host checks
  what you typed (known ids, scopes the permission actually takes, valid hosts and categories;
  `ui.dom` is refused for a sandboxed page) and
  enforces exactly the result everywhere a normal grant applies: the frame's network policy,
  scoped tokens, command and library checks. The card then shows a "Permissions overridden" badge
  and what differs (extra, withheld, re-scoped), the audit log records the same, and approving
  again without the override clears it. Only the console can do this, like any approval.
- **Sharing a keyring namespace: `keyring.service`.** A plugin normally sees only its own secrets
  (`keyring.own`). Two plugins that need the same one (say a Pexels key) can share a namespace
  instead: both request `keyring.service` with `services: ["OpenSanctuary:Shared:<name>"]` (1 to 4,
  lowercase letters, digits, `.`, `_`, `-`, exact; nothing else is ever grantable, so the System
  namespace, any plugin's own namespace and the app's own services stay out of reach) and
  `access: "read"` (the default) or `"write"`. It is Critical tier, for sandboxed plugins
  (not console scripts, which already have console access). The review screen says which shared area
  a plugin can read, who else shares it, and warns when the plugin can also reach the network. The
  rules: a plugin can only read rows tagged as shared (`shared:plugin:<id>` or `shared:host`), so a
  secret that happened to be stored under a shared-looking name before this existed is never exposed;
  with write access it can only create, replace or delete its *own* rows, so a co-sharer can't swap
  another's key; the console can manage every shared row (Settings > Plugins > Shared secrets lists
  them, never the values, with a Delete for each). In a sandboxed plugin:
  `OS.call('keyring.get', { service: 'OpenSanctuary:Shared:pexels', account: 'api_key' })`
  (`keyring.set` / `keyring.delete` likewise; with no `service` it is the plugin's own namespace and
  needs `keyring.own`). The rules live in one policy (`src/api/keyring_policy.rs`). The first use and each refusal are audited
  (never the secret), and the check always uses the plugin's live grant, so a revoke stops it at once.
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
  request until something new arrives, for a client that wants it).
- **Stock Images is a plugin.** Searching Pexels and Pixabay (the Media library's "Online Images")
  is `web/plugins/stock_media/`, one plugin for both sites: a sandboxed page in the Media library
  sidebar with a search box, a source toggle per site, a thumbnail grid and an "API keys" section.
  You use your own free key from each site; the plugin keeps them in its own keyring namespace
  (`keyring.own`), so they are no longer Options settings and the old built-in search and import
  routes are gone (keys entered there before are not carried over: paste them into the plugin once).
  It asks for the two search APIs and their image hosts (`api.pexels.com`, `images.pexels.com`,
  `pixabay.com`, `cdn.pixabay.com`), `library.write` on media, and optionally `notifications.post`.
  Three host features exist for it, available to any plugin with the matching grant:
  `net.fetch` replies with `cacheSecs` as well (how long the publisher lets the response be kept); it takes optional `headers` (`Authorization`, `Accept` and `Accept-Language` always; any other header must be declared in the manifest, see "Extra request headers" below; Pexels
  wants its key in `Authorization`, YouVersion in `X-YVP-App-Key`); `library.addMediaUrl { url, title, id? }` has the *host*
  download a picture or video from a granted host (https, public addresses, 64 MB cap) into the
  Media library, with the file type taken from the downloaded bytes and the name chosen by the
  host; and a plugin frame's `img-src` allows `https://` pictures from the hosts it was granted
  for `network.outbound` (nothing new beyond what `connect-src` already allowed), so thumbnails
  load. A refused fetch is audited without its query string, because Pixabay puts the key there.
- **Video Downloader is a plugin.** Downloading with yt-dlp (YouTube and 1000+ other sites, optional
  browser cookies, SponsorBlock, audio-only) is `web/plugins/ytdlp/`: a sandboxed page in the Media
  library sidebar and in the video picker, built on `process.run` above. The old built-in downloader,
  updater and routes are gone.
- **Song lyrics is a plugin.** Searching for lyrics and importing them used to be built into the
  server and the Song Library panel; it is now `web/plugins/genius_lyrics/` (id kept from when it only
  searched Genius; the name is Song Lyrics). It searches LRCLIB, an open community lyrics database, and
  Genius together, so Genius's bot filter refusing a request never hides LRCLIB's results. It is a sandboxed
  panel that asks only for `lrclib.net` and `genius.com`, `library.write` on songs, and optionally
  `AddToSchedule`/`GoLive`. Approve
  it in Settings > Plugins and it appears as a panel. It is also the worked example of a plugin that
  needs the network.
- **No pre-manifest or native plugins.** A bare `.js` script in `web/plugins/` with no `.plugin.json`, the
  one-time "grandfathering" that auto-approved old plugins, and native plugins (compiled libraries, ABI 1
  or 2, and everything built on them) are all gone. Bare scripts are ignored with a warning in the log;
  a manifest with a `backend` section is refused with an explanation. Every plugin starts as pending until
  the operator approves it. Rows an older version recorded for the removed kinds are deleted from the
  plugin list on the next scan (the audit log keeps its history).

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

There is **no command-interception or state-change hook** for a console script —
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

## Versions and updates

`version` in `plugin.json` must be a semantic version (`1.2.3`; empty is allowed). `host_api` (a semver requirement such as `>=0.2.0, <0.3.0`) says which OpenSanctuary versions it works with; outside that range the plugin shows as incompatible.

When an approved plugin's files change (a new version dropped in, an **Update…**, or an edit), it stops until you review it again, and the card shows **what changed** against the version you approved: `v1.0.0 → v1.1.0`, whether that is an upgrade, a **downgrade** (an older version than you approved, shown with a warning), or the same version number with different files (also warned), the files edited, added and removed, and any **new permissions** with their risk level, or wider scope on existing ones. Permissions you declined last time (optional ones) are not reported as new again. The manifest counts as a file, so a version bump shows `plugin.json` as edited. Plugins approved before this was recorded show the version and permission changes but not the file list.

---

## Signing your plugin (optional)

A signature tells the operator *who* released a package and that no file was changed since; it never grants anything, and an unsigned plugin works the same way. To sign:

1. Finish your plugin. Any later edit to any file means repeating steps 2 to 4.
2. `os-next --hash-plugin my_plugin` writes `"files": { "<path>": "<sha256>", ... }` into `plugin.json`, one entry for every file except `plugin.json` and its signature.
3. Put your [minisign](https://jedisct1.github.io/minisign/) public key (the second line of your `.pub` file) in `plugin.json` as `"publisher_key"`.
4. `cd my_plugin && minisign -Sm plugin.json` writes `plugin.json.minisig`. (`plugin.json` is signed byte for byte, so don't reformat it afterwards.)
5. `os-next --check-plugin my_plugin` should say **fully signed**.

The review screen then says *signed, covers every file*, with the publisher's name from `publisher` in `plugin.json`. A signed `plugin.json` that doesn't list `files` says *manifest signed; the other files aren't covered*, and a package whose files differ from the list (edited, missing, or an extra file) is refused outright, signed or not. Design and what comes next: [PLUGIN_SIGNING.md](PLUGIN_SIGNING.md).

---

## Publishers and trust

Every plugin card shows a badge for how far its publisher (the minisign key that signed it) is known to you:

| Badge | Meaning |
|---|---|
| **Built-in** | Ships with OpenSanctuary |
| **Trusted publisher** | Signed by a key you chose to trust |
| **Known publisher** | Signed by a key you have approved a plugin from before, but never chose to trust |
| **New publisher** | Signed by a key you haven't seen before |
| **Unsigned** | Nothing says who released it |
| **Invalid signature** | The signature doesn't verify; it can't be installed or run |
| **Revoked publisher** | Signed by a key you revoked |

Tick **Trust everything this publisher signs** when you approve a signed plugin (or press **Trust publisher** later) to trust a key. **A badge is information, not permission.** Trust never approves anything and never skips the review screen: a trusted publisher's update still stops at "changed" and shows what changed, and a signature says who released a package, not that it is harmless.

**Rotating your key.** If your signing key changes, ship `plugin.rotation.json` (`{ "old_key": "<old public key>", "new_key": "<new public key>" }`) in the plugin folder, signed by the **old** key (`minisign -Sm plugin.rotation.json -x plugin.rotation.json.minisig`), and sign `plugin.json` with the new one. Operators who trusted your old key then see no warning and their trust carries over. Without the statement, a key change is shown as a different publisher.

**Revocation.** OpenSanctuary can be given a list of publisher keys to distrust (`revoked_keys.json`, signed by the OpenSanctuary release key). A revoked key's plugins stop, and approving one asks for confirmation. Only the release key can sign a list, the list can only move forward, and the release key itself can't be revoked.

Under **Publishers** you can **Stop trusting** a key or **Revoke** it. Revoking stops every plugin that key signed (as if you had revoked each one), marks its other plugins "Revoked publisher", and makes approving one of them ask for an extra confirmation. Revoked keys can't be trusted again from the card. Everything here is console-only and in the audit log.

---

## Installing and removing plugins

**Settings → Plugins → Install plugin…** takes a **package**: a `.zip` holding the plugin's files, with `plugin.json` at the top (or inside one wrapping folder, which is stripped). Installed plugins live in `<data dir>/plugins/<id>/`, so they survive app updates and work when the app itself is installed read-only. The ones that ship with the app are in `web/plugins/`.

- **Installing grants nothing.** The plugin appears as "needs review" and does not run until you approve it, like any other.
- **The package is not trusted.** It is checked before anything is kept: no paths that leave the folder, no symbolic links, at most 300 files and 25 MB once unpacked (counted as written, so a zip bomb that lies about its size is stopped), and the result must pass the same checks as a plugin found on disk. A failed install leaves nothing behind.
- **Built-in plugins can't be shadowed.** A package whose id belongs to a bundled plugin is refused, and an installed folder with such an id is ignored.
- **Update…** on an installed plugin replaces its files. The new files hash differently, so the plugin shows "changed" and stays off until you review it.
- **Remove** deletes the files, the approval, the plugin's downloaded tools and the secrets it saved in the keyring (so a different plugin installed later under the same id can't read them). Bundled plugins can only be revoked.

Every install, update and removal is in the audit log. The API is `POST /api/plugins/install` (body: the zip; `?replace=true` to update) and `DELETE /api/plugins/:id`, both console-only.

To make a package: `cd my_plugin && zip -r ../my_plugin.zip .`

**Check a plugin before you ship it:** `os-next --check-plugin path/to/my_plugin` runs the checks the app runs on install (manifest, entry file, icons, permission names and scopes) and prints what the operator would be asked to approve, with each permission's risk level. It exits 0 if the plugin would load. The folder's name must be the plugin's id.

---

## When a plugin misbehaves

A plugin page runs in its own frame, but some browsers run that frame on the console's own thread, so a page stuck in an endless loop can freeze the whole tab. The console defends against this without trusting the plugin:

- **Watchdog.** The console pings every running frame; the SDK (`__os/sdk.js`) answers automatically. A frame that was answering and then goes silent for 15 seconds is removed for the rest of the session and you get a notice. It stays approved; reload to start it again. **Load the SDK**: a page that never answers a ping is not watched.
- **Safe mode.** If the console itself hangs, nothing on the page can help, so the next load does. Plugins are started behind a marker that is cleared after a clean unload or 8 healthy seconds. If the marker is still there at the next start, a banner says plugins are paused and every plugin (including console scripts) waits until you press **Resume plugins**. Nothing is revoked.
- **Load shedding.** A frame that does lots of small jobs keeps answering pings while the console crawls, because they share a thread. When the console's own 1-second timer runs late three ticks out of four, plugins are switched off one at a time (newest first) until it recovers. The last one switched off is blamed and stays off for the session with a notice; the ones that were only being tested are started again. If switching them all off doesn't help, the plugins weren't the cause and nothing is blamed. This can take a minute on a badly stalled console.
- **Limits that always apply:** manifests over 256 KB are rejected, notifications are rate-limited per plugin, a frame's height is clamped, and a frame may have at most 32 bridge requests outstanding at once (`MAX_IN_FLIGHT` in `web/src/core/plugin_bridge.ts`). Past that, a request fails straight away with `too many requests in flight` instead of queuing, so a flood can't keep the console busy. Run parallel work through a small pool (the Online Bibles download uses 5 workers) rather than firing every call at once.

---

## Choosing between them

| | Sandboxed web plugin | Console script (`ui.dom`) |
|---|---|---|
| Where it runs | An isolated frame (opaque origin) | Inside the console page itself |
| Access | Only what the operator granted, through a permission-checked bridge | Everything the operator has |
| Extends the operator UI | Through placements (library panels, the video picker, a dock) and ribbon buttons, in the app's own look | Full DOM access |
| Network | Only the granted hosts, fetched by the server | Whatever the page can do |
| Secrets | `keyring.own`, and `keyring.service` for shared namespaces | `keyring.own` |
| Risk tier | Per permission (Low to Critical) | Critical (`ui.dom`) |
| Distribution | A folder in `web/plugins/` | A `.js` file and a `.plugin.json` in `web/plugins/` |

Use a sandboxed plugin unless you genuinely need to reach into the console's own page.

## Licensed Bible text

A plugin can also list translations in the Scriptures sidebar without storing them (`library.setRemoteBibles`); the console searches them by reference only and the text is fetched when a result is added to the schedule. A plugin that serves text which may be shown but not kept (YouVersion's NIV, NASB...) marks what it stores with a `licensed` block;
the host then seals, expires and strips it from saved and exported files, and can ask the plugin to look it up again when a schedule
naming it is opened. How it works, the license terms behind it, the key release builds carry and the plugin API: [LICENSED_SCRIPTURE.md](LICENSED_SCRIPTURE.md).

## Extra request headers

`net.fetch` lets every plugin send `Authorization`, `Accept` and `Accept-Language`. A service that wants its key in another header
(YouVersion's `X-YVP-App-Key`) is declared in the manifest, next to the hosts:

```json
{ "id": "network.outbound", "hosts": ["api.youversion.com"], "headers": ["X-YVP-App-Key"], "reason": "..." }
```

* The operator sees it on the review screen ("sends X-YVP-App-Key") and approves it with the rest; adding a header later is a change that
  needs review again. The plugin's token carries only the approved headers, and only to the hosts it was granted.
* At most 8, plain names (letters, digits, `-`). Never allowed, declared or not: `Host`, `Cookie`, `Set-Cookie`, `Origin`, `Referer`,
  `User-Agent`, `Connection`, `Content-*`, `Accept-Encoding`, `Transfer-Encoding`, `Upgrade`, `TE`, `Via`, `Forwarded`, `Range`, `Proxy-*`,
  `X-Forwarded-*`, `X-Real-IP`, `Sec-*`, `If-*`, and the app's own `X-Host-Token`, `X-Plugin-Token`, `X-Device-Token`, `X-Console-Session-Id`.
  Header values are limited to 2048 bytes with no control characters.
