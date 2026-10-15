# Plugin permission system design for OpenSanctuary (OS-Next)

Status: **implemented.** This began as a design (written by an Opus design pass) and is kept as the reference for how plugin permissions work; sections 0, 6 and 8 and the "native" remarks in 5 and 7 are history. What exists today:

- **Manifest, grants, digest pinning, signer pinning, audit log, scoped plugin tokens**, and a console review screen (Settings > Plugins) where only the console can approve, deny, revoke or **override** (an Advanced mode that grants exactly the permissions the operator picks, behind a warning; see section 4).
- **Two kinds of plugin:** *sandboxed web* (an isolated frame behind a permission-checked bridge: the recommended kind) and *console script* (`ui.dom`: runs in the console page, Critical). **There are no native plugins:** loading compiled libraries into the server (the `OsPlugin` trait, the ABI 2 `PluginHost` API, `commands.intercept`, `commands.modify`, `resources.provide`, `native.code` and the filesystem/process/listen permissions that only made sense for them) was removed, as were the pre-manifest forms before it. A manifest with a `backend` section is refused.
- **Permissions in force:** every id in the table in section 2 marked enforced. (`settings.read` and `settings.write` were reserved ids that nothing ever acted on; they were removed rather than left looking like real grants.)
- **Features built on it:** placements (`ui.placements`) and ribbon buttons (`ui.toolbar`), `net.fetch` with limited headers, `library.addSong/addScriptures/addMedia/addMediaUrl`, `process.run` with host-managed signed tools, `notifications.post` and shared keyring namespaces (`keyring.service`). The bundled plugins (`web/plugins/`) are the worked examples. The guide for writing a plugin is `docs/PLUGINS.md`; this document is the permission model behind it.
- **Commands** from REST, WebSocket and a sandboxed plugin's `sendCommand` all go through `src/api/command_pipeline.rs`: shorthand expansion, parse, execute, persist, broadcast.
---

## 0. What existed before this system (history)

> History: this section describes the code before the permission system, including the native plugin loader that has since been removed.

| Surface | Mechanism | Identity | Authorization today |
|---|---|---|---|
| Native backend plugins | `PluginManager::load_plugins_from_dir(paths::plugin_dir())` in `main.rs:139`. It loads every `*.so/.dll/.dylib` in `~/.config/OpenSanctuary/plugins/` through `libloading`, calls `create_plugin() -> *mut dyn OsPlugin`, and keeps the libraries mapped forever. | `OsPlugin::name()`, which the plugin chooses itself. | None. Every loaded plugin gets all three hooks: `on_command` (veto), `on_state_change`, and `provide_resources(category)`. |
| Hook dispatch | `check_command` in `ws.rs:580` and `show.rs:162`. `notify_state_change` in `broadcast_snapshot` (`ws.rs:270`). `collect_resources` in `merge_with_plugin_resources` (`routes/common.rs:23`), called from `library.rs` for songs, scriptures, media, themes and presentations. | n/a | `catch_unwind` per call. Four commands are "safety-critical" and cannot be vetoed. |
| Web plugins | `web/src/core/plugins.ts`. `main.ts:34` hard-codes `loadLocalPlugin("/plugins/hello_world.js")`, which runs in the console's own JS realm. | `module.default.name` | None. The plugin can read `window.__OS_HOST_TOKEN__` and the `console_tok_…` value in localStorage, and can call `sendCommand`. |
| Plugin tokens | `POST /api/keyring/plugin/token`, gated by host token through `host_token_gate_middleware`/`needs_host_token`. It checks the name against `web_dir/plugins/<stem>` (`is_known_plugin_name`). `AppState.plugin_tokens: HashMap<token, name>` lives in memory, never expires and has no size cap. | Asserted plugin name | The only thing a token can do is reach the keyring namespace `OpenSanctuary:Plugin:<name>`, with ownership tracked in `keyring_metadata.owner = "plugin:<name>"`. |
| Console auth | `is_console_credential`: the per-run `host_session_token`, or a persistent `console_tok_…` stored as a SHA-256 in `paired_devices` with `platform='console'`. Paired remotes (`mobile-remote`) may send show commands only. Displays may do nothing. | | All non-GET `/api/*` requests require host-token auth by default (`routes/mod.rs:80`). |

**Problems I found along the way.** They matter to this design:

1. **WS and REST hand plugins different text.** WS calls `check_command(&safe_text)` with the envelope as it was before shorthand expansion, `{"cmd":{…}}` with credentials removed. REST passes the command after expansion. The same plugin therefore sees two different shapes.
2. **The safety-critical bypass is a substring match** (`cmd.contains("SetAlert")`). For example, `AddToSchedule` with a song titled "SetAlert" skips every veto. It should match on the parsed variant instead.
3. **`plugin_tokens` grows without bound and has no TTL.**
4. **Keyring identity mismatch for `hello_world`.** `registerPlugin` scopes the vault with `forPlugin(module.default.name)`, which is `"Hello World Plugin"`. The server only accepts the file stem, `"hello_world"`. As written, `hello_world`'s keyring token request would get a 404.
5. **The native ABI is already fragile.** `*mut dyn OsPlugin` crosses an `extern "C"` boundary using a Rust trait-object vtable. When `provide_resources` was added, it silently changed the vtable for any `.so` built earlier. Nothing checks for this, so a mismatched plugin is undefined behaviour.
6. **`examples/` holds no plugin.** It contains only `sign_release.rs`, the minisign release signer. The only in-repo plugin is `web/plugins/hello_world.js`. The native "examples" are the test structs in `tests/core_tests.rs:904` and `:973`.

---

## 1. Threat model and honest limits

**Assets:** what goes out live to the projector and stream, the schedule and library, keyring secrets, the host token and console credentials, and the server machine itself.

**What a permission system can enforce, by plugin type:**

| Plugin type | Enforceable? | Why |
|---|---|---|
| **Same-realm web plugins (today's model)** | **Nothing, beyond whether the script loads at all.** | Same origin and same JS realm as the console. The plugin can take the host token or console credential and act as the console, including approving other plugins. |
| **Sandboxed web plugins (iframe without `allow-same-origin`, phase 4)** | **Fully.** | Opaque origin with no access to the parent's storage or host token. It only talks through a `postMessage` bridge that the host checks against grants. It authenticates to the server with a scoped plugin token. A per-plugin CSP `connect-src` enforces `network.outbound`. |
| **Out-of-process HTTP/WS clients holding a plugin token** | **Fully, for the routes that require auth.** | Scoped tokens are checked on each request. Caveat: library and state GETs are deliberately zero-auth (for the live, stage and remote pages), so a `library.read` permission over HTTP restricts nothing. |

**Threats in scope:**
- (a) The operator approves a plugin without knowing what it asks for. Consent and plain-language tiers address this.
- (b) A plugin binary or script is swapped or updated after approval and runs on the old consent. Digest pinning and optional signature TOFU address this.
- (c) A buggy but honest plugin uses host features it did not declare. Hook and host-API gating addresses this.
- (d) A plugin token is used beyond its purpose, whether stolen or misused. Scopes, TTLs and revocation address this.
- (e) A device or remote credential tries to grant plugin permissions. Only console credentials may grant.

**Threats out of scope (stated in the UI and docs):**
- A local attacker with the OS user's write access. They can edit `library.db` grants, the plugin directory, or autostart entries.
- TOCTOU between hashing a file and `dlopen`, which needs the same local-write attacker.

**Recommendation:**
- **Enforce:** plugin-token scopes, the iframe bridge and frame policy, web-plugin load gating.
- **Console scripts are trust, not sandbox:** approving a `ui.dom` plugin gives it the console's own access; the prompt says so first.
---

## 2. Permission vocabulary

Every permission is denied unless granted. A permission is an `id` plus optional parameters. Tiers set the UI emphasis: Low and Medium are pre-checked if requested, High needs an explicit checkbox, Critical shows a red warning.

| id | Params | Tier | Enforced | Gates |
|---|---|---|---|---|
| `ui.dom` | — | Critical | consent-only | A same-realm (non-sandboxed) web plugin, equivalent to console access. |
| `show.observe` | — | Low | **E** | `getState` and the state event stream over the bridge. |
| `library.write` | `categories[]` | High | **E** | `library.addSong/addScriptures/addMedia/addMediaUrl`, per category. |
| `commands.send` | `commands[]`: ShowCommand variant names, or `"*"` | Medium if every listed variant is navigation (`NextSlide`, `PrevSlide`, `NextItem`, `PrevItem`, `JumpSlide`, `JumpSection`, `GoLive`, `StageItem`); otherwise High; `"*"` is High | **E** | `host.send_command()`, `/api/command` and `/ws` with a plugin credential. |
| `keyring.own` | — | Medium | **E** | The `OpenSanctuary:Plugin:<id>` namespace only. |
| `keyring.service` | `services[]` (`OpenSanctuary:Shared:<name>`), `access` (`read`\|`write`) | Critical | **E** | Shared keyring namespaces; owner rules in PLUGINS.md. Only the `Shared:` prefix is grantable, never System, another plugin's namespace or a host service. |
| `network.outbound` | `hosts[]` | Medium | **E** (iframe CSP) | |
| `process.run` | `commands[]` (ids of commands declared in the manifest) | Critical | **E** | Sandboxed plugins only: the host runs exactly the manifest's declared command templates, no shell, throwaway job folders, with host-managed signed tools. See PLUGINS.md. |
| `ui.panel` | — | Medium | **E** (slots) | |
| `ui.toolbar` | — | Medium | **E** (sandboxed web plugins) | Up to 2 ribbon buttons declared in `ui.toolbar`; see PLUGINS.md |
| `ui.clipboard` | — | Medium | **E** (sandboxed web plugins) | `ui.pasteText`: read the text on the system clipboard (it is often a password). Separate from `ui.panel` so a harmless panel can't read it. |
| `notifications.post` | — | Low | **E** (`POST /api/plugin-host/notify`) | Posts to the console's notification center; see PLUGINS.md |

**E** means the host enforces it. **Consent-only** means it is shown at consent time and recorded, but not technically enforced (a console script shares the page, so nothing could enforce it).  **Reserved** means the id is accepted but nothing acts on it yet.

**Which kinds can hold what.** `ui.dom`: console scripts only. Everything else: sandboxed web plugins (`keyring.service` is refused for a console script, which already has console authority). The review screen's Advanced override refuses combinations that make no sense for the plugin's kind.

**How scopes narrow.** A scoped plugin token (`POST /api/plugins/:id/token`) can carry `keyring.own`, `keyring.service`, `commands.send`, `network.outbound`, `library.write`, `process.run` and `notifications.post`, and never more than the live grant: `PermissionSet::narrowed_by` intersects every list (commands, hosts, categories, services) and takes the lower access for `keyring.service`. Asking for more than was granted is refused, not silently reduced. Every use re-checks the live grant, so a revoke stops a token at once.

**Unknown permission ids:** if the manifest's `manifest_version` is newer than the host supports, the plugin is refused with "requires newer OS-Next". Otherwise the manifest is rejected (fail closed).

---

## 3. Manifest

**Layout.** No new dependencies: JSON via serde_json, digests via `sha2`, signatures via `minisign-verify`, version ranges via `semver`.

```
<web_dir>/plugins/<id>/plugin.json            # a sandboxed plugin: the folder is the plugin (plugin.json + its pages)
<web_dir>/plugins/<id>/plugin.json.minisig    # optional detached signature over plugin.json bytes
<web_dir>/plugins/<stem>.js (+ <stem>.plugin.json)  # a console script and its manifest
```

**Example `plugin.json`** (a sandboxed plugin):

```json
{
  "manifest_version": 1,
  "id": "org.example.planning",
  "name": "Planning Sync",
  "version": "1.2.0",
  "publisher": "Example Church AV",
  "description": "Pulls this week's plan and stages it.",
  "host_api": ">=0.2.0, <0.3.0",
  "web": { "entry": "index.html", "sandbox": true },
  "permissions": [
    { "id": "ui.panel", "reason": "Shows its panel in the console" },
    { "id": "network.outbound", "hosts": ["api.example.org"], "reason": "Fetches the plan" },
    { "id": "library.write", "categories": ["songs"], "reason": "Adds the songs of the plan" },
    { "id": "commands.send", "commands": ["AddToSchedule"], "optional": true, "reason": "Adds them to the schedule" },
    { "id": "keyring.own", "reason": "Stores the API key" }
  ]
}
```

**Validation rules:**
- `id` matches `^[a-z0-9][a-z0-9._-]{1,63}$` and must equal the directory name.
- `host_api` must match `CARGO_PKG_VERSION`.
- Every file path (the entry page, a tool's key file, ...) must stay inside the plugin folder: no `..`, no absolute paths, and symlinks are rejected.

**Pinning:**
- `manifest_sha256` is the SHA-256 of the `plugin.json` bytes.
- `artifact_digest` is the SHA-256 over the sorted lines `"<relpath>\0<sha256>\n"` for every file in a sandboxed plugin's folder (or the single script file of a console script). Each file's hash must also equal the `sha256` the manifest declares for it.
- Approval records both values. On every boot, and on every web-plugin fetch, the host recomputes them. **Any mismatch moves the plugin to `changed`: it is not loaded, and the console asks for re-review.**

**Signatures (optional, TOFU):**
- If `plugin.json.minisig` exists, it is verified with `minisign_verify::Signature`. The manifest carries the artifact hashes, so one signature covers the binaries too.
- The signer's key id and public key come from a `publisher_key` field in the manifest. They are pinned on first approval.
- Later versions signed by the same key get a one-click "Approve update" when they request no new permissions. A different key, or a signed plugin that becomes unsigned, triggers a full re-prompt with a warning.
- Unsigned plugins are allowed and labelled "unsigned".

---

## 4. Grant and consent flow

**Discovery** (`main.rs`, which already runs after the `Database` opens):
1. Scan `web_dir/plugins`, and compute digests.
2. Upsert a row into `plugin_grants` with `observed_*` set. Log a `discovered` or `changed` audit event.
3. Start or serve a plugin only if `status='approved'` and `observed_digest = approved_digest`.

**Console UI:** a new Settings → Plugins tab.
- Lists the states active, pending, changed, denied, revoked and incompatible, with publisher, signed/unsigned, version and digest prefix.
- The review dialog opens with a red banner for `ui.dom` ("Runs inside the console page with the same access as you; the permissions below describe what it says it will do"). Below that, permissions are grouped by tier and each shows its `reason`. Consent-only items are tagged "declared, not enforced". Optional items have checkboxes.
- **Approve** sends back the digest the operator reviewed. The server refuses if the digest has changed since.
- **Revocations take effect immediately**: bridge calls and tokens stop working, and the frame is removed.

**HTTP API** (new `src/api/routes/plugins.rs`). All POSTs are already gated by `host_token_gate_middleware`; the GETs below get added to `needs_host_token`.

```
GET  /api/plugins                    -> [{id,name,version,kind,status,signed,signer_key_id,observed_digest,approved_digest,requested,granted,tiers}]
POST /api/plugins/{id}/approve       {digest, granted:[perm…]}   // granted ⊆ requested; required ones mandatory
POST /api/plugins/{id}/deny          {digest}
POST /api/plugins/{id}/revoke        {}
GET  /api/plugins/audit?limit=200
POST /api/plugins/{id}/token         {scopes?:[perm…]}           // console mints scoped token for a web plugin
```

**Who may grant:** only `require_host_token`, meaning the per-run host token or a `console`-platform credential. Device tokens (remotes, displays) and plugin tokens are never accepted on `/api/plugins/*`. That rule is explicit in the handler as well as in the middleware. `decided_by` records `"host"` or `"console:<paired_devices.id>"`.

**Honest note for the UI:** a granted `ui.dom` web plugin can approve other plugins, because it holds console authority. That is why `ui.dom` is Critical.

**Schema** (added to `init_main_schema` in `src/storage/db/core.rs`, using the existing `CREATE TABLE IF NOT EXISTS` pattern):

```sql
CREATE TABLE IF NOT EXISTS plugin_grants (
    plugin_id          TEXT PRIMARY KEY,
    kind               TEXT NOT NULL CHECK (kind IN ('native','web','sandbox'))  -- the table still allows 'native' (and the old legacy kinds) because SQLite can't alter it; nothing writes them now and rows of those kinds are deleted on scan
    name               TEXT NOT NULL,
    version            TEXT,
    source_path        TEXT NOT NULL,
    observed_manifest_sha256 TEXT,
    observed_digest    TEXT NOT NULL,
    observed_manifest_json   TEXT,          -- what's on disk now (for review)
    approved_manifest_sha256 TEXT,
    approved_digest    TEXT,
    signer_key_id      TEXT,                -- pinned on first approval (TOFU)
    signer_pubkey      TEXT,
    status             TEXT NOT NULL CHECK (status IN ('pending','approved','denied','revoked')),
    granted_json       TEXT NOT NULL DEFAULT '[]',
    decided_at         INTEGER,
    decided_by         TEXT,
    first_seen         INTEGER NOT NULL,
    last_seen          INTEGER NOT NULL
);
-- effective state: approved && observed_digest = approved_digest -> active; approved && mismatch -> 'changed'

CREATE TABLE IF NOT EXISTS plugin_audit (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    ts_ms       INTEGER NOT NULL,
    plugin_id   TEXT NOT NULL,
    actor       TEXT NOT NULL,     -- 'system' | 'host' | 'console:<device id>' | 'plugin:<id>'
    action      TEXT NOT NULL,     -- discovered|changed|approved|denied|revoked|token_issued|denied_call|load_failed|media_added|tool_updated|tool_update_failed|keyring_shared_access  (an override is an `approved` entry whose detail has an `override` diff)
    detail_json TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_plugin_audit_plugin ON plugin_audit(plugin_id, ts_ms);
```

- `denied_call` rows are coalesced to one per (plugin, permission) per run, so they cannot flood the table.
- The table is pruned to the last 5,000 rows on startup.
- New db module: `src/storage/db/plugins.rs`.


### Advanced override (operator)

The normal review grants a subset of what the plugin asked for. For the cases that isn't enough, each plugin card has an **Advanced: override what this plugin asks for** toggle, shown behind a warning that it may break the plugin or give it more access than its author intended, and applied only after the operator ticks "I understand this may break the plugin". It lets the operator:

- switch off any permission the plugin asked for, **even a required one**;
- edit a permission's scope list (hosts, commands, library categories, paths, ports, services and access);
- switch on **any permission the host knows** that the plugin did not ask for (the list comes from `GET /api/plugins/permissions`).

`POST /api/plugins/:id/approve` takes the result as `override` (the exact list to grant; `granted` is ignored when it is present). The host validates it with `plugin_permissions::validate_grant` (known ids; no duplicates; only scope lists the permission actually takes; valid hosts and categories; `ui.dom` only for console scripts; `keyring.service` only for shareable namespaces) and the digest check still applies. What is stored is what is enforced, everywhere a normal grant is: the frame's network policy, scoped tokens, command and library checks. The card shows a **Permissions overridden** badge with what differs (extra, withheld, re-scoped); the audit entry for the approval carries the same diff and the server logs a warning. Approving again the normal way clears the override. Only the console can do it, like any approval.

---

## 5. Enforcement points

> Parts of this section describe the native host API and hook dispatch (`OsPlugin`, `PluginHost`, `check_command`) from the original design; native plugins were removed, so read those as history. The token, bridge and web-loading parts are current.

**New module `src/core/plugin_permissions.rs`:**

```rust
pub enum Permission { ShowObserve, LibraryRead(CatSet), ResourcesProvide(CatSet), LibraryWrite(CatSet),
    CommandsSend(CommandAllow), CommandsIntercept, KeyringOwn, SettingsRead(Vec<String>), SettingsWrite(Vec<String>),
    NetworkOutbound(Vec<String>), NetworkListen(Vec<u16>), FsPluginData, FsRead(Vec<String>), FsWrite(Vec<String>),
    ProcessSpawn, UiPanel, UiToolbar, UiDom, NativeCode }
pub struct PermissionSet(Vec<Permission>);
impl PermissionSet {
    pub fn allows_command(&self, variant: &str) -> bool;
    pub fn provides(&self, category: &str) -> bool;
    pub fn has(&self, p: PermKind) -> bool;
}
pub struct PluginManifest { … }            // serde, + fn validate(&self, host_version) -> Result<(), ManifestError>
pub fn artifact_digest(dir: &Path, files: &[PathBuf]) -> io::Result<String>;
pub fn command_variant(v: &serde_json::Value) -> Option<&str>; // externally-tagged: string or single-key object
```

**`src/core/plugins.rs`:**
- `struct LoadedPlugin { id: String, grants: Arc<GrantCell>, plugin: Box<dyn OsPlugin> }`, where `GrantCell { enabled: AtomicBool, perms: RwLock<PermissionSet> }`. `PluginManager` stays `Arc` with no `AppState` type change, because mutation goes through `GrantCell`.
- `load_plugins_from_dir(dir)` becomes `load_approved(&self, candidates: Vec<ApprovedPlugin>)`. Discovery and digest checks move to `plugin_registry.rs`.
- `check_command(&self, cmd_json: &str, variant: Option<&str>)`:
  - safety-critical is decided by `variant ∈ {ToggleBlackout, ToggleClearText, ToggleLogo, SetAlert}`, not by substring;
  - only plugins that are `enabled && has(CommandsIntercept)` are consulted.
- `notify_state_change` calls only plugins with `ShowObserve`.
- `collect_resources(category)` calls only plugins with `provides(category)`. Each returned item is stamped `"_plugin": id`, and its `id` gets the prefix `"<plugin_id>:"` if it doesn't already have it.
- `set_grants(id, PermissionSet)` and `revoke(id)`.
- **Freeze `OsPlugin` (v1):** doc comment "never add methods; vtable is ABI". Add a separate ABI v2 entry point:

```rust
pub trait PluginHost: Send + Sync {
    fn plugin_id(&self) -> &str;
    fn has(&self, perm: PermKind) -> bool;
    fn snapshot(&self) -> Result<StateSnapshot, PluginError>;              // ShowObserve
    fn send_command(&self, cmd: ShowCommand) -> Result<(), PluginError>;   // CommandsSend(variant)
    fn library_list(&self, category: &str) -> Result<Vec<Value>, PluginError>;
    fn keyring_get(&self, account: &str) -> Result<Option<String>, PluginError>; // KeyringOwn
    fn keyring_set(&self, account: &str, secret: &str) -> Result<(), PluginError>;
    fn keyring_delete(&self, account: &str) -> Result<bool, PluginError>;
    fn data_dir(&self) -> Result<PathBuf, PluginError>;                    // creates data_dir/plugin-data/<id>
    fn log(&self, level: tracing::Level, msg: &str);
}
// v2 constructor symbol:
// #[no_mangle] extern "C" fn create_plugin_v2(host: *const Arc<dyn PluginHost>) -> *mut dyn OsPlugin
// plus os_next::export_plugin!(Type) macro emitting the ABI statics + constructor.
```

- **`send_command` never runs inline.** It does `try_send` into a bounded `tokio::mpsc` (capacity 64, about 20 commands/s per plugin). A task drains it through the shared pipeline below. This prevents re-entrancy (`on_state_change`, then `send_command`, then `notify_state_change`, and so on) and deadlocks.
- The host handle holds an `AppState` clone. That creates an Arc cycle through `plugin_manager`, which is acceptable because it lives for the whole process. Handles are attached after `AppState` is built in `main.rs`.

**Shared command pipeline.** New function in `src/api/ws.rs`:

```rust
pub enum CommandOrigin { Console { session_id: String }, Device { id: String }, Plugin { id: String } }
pub fn run_command_pipeline(app: &AppState, value: Value, origin: CommandOrigin) -> Result<StateSnapshot, CommandRejection>
```

It does shorthand expansion (the duplicated AddToSchedule/StageItem block in `ws.rs:530–578` and `show.rs`), then `command_variant`, then the origin check (for `Plugin`: enabled and `allows_command(variant)`), then `check_command(redacted_post_expansion_json, variant)`, then parse, `execute_command`, `save_event` and `broadcast_snapshot`. WS, REST and `PluginHost` all call it. This fixes problems 1 and 2 from section 0.

**Plugin tokens (`src/api/ws.rs`, `routes/keyring.rs`):**
- The `plugin_tokens` value type changes from `String` to `PluginTokenInfo { plugin_id, scopes: PermissionSet, expires_at: i64 }`. TTL is 12 hours, with at most 8 tokens per plugin.
- The map stays inferred as `HashMap::new()`, so the six or so `AppState { … }` literals in `core_tests.rs` need no edits.
- `resolve_plugin_token` becomes `resolve_plugin_token(&self, tok) -> Option<PluginCaller>`. It applies `scopes ∩ live grant`, so revoking a plugin kills its tokens at once even before the purge.
- `revoke_plugin_tokens(plugin_id)`.
- New `authenticate_plugin(headers, app, need: PermKind) -> Result<PluginCaller, AppError>`.
- `keyring.rs`:
  - `authenticate_keyring_caller`'s plugin branch requires `KeyringOwn`;
  - `keyring_plugin_token` checks `plugin_grants` (`status='approved'`, digest matches, `KeyringOwn` granted) instead of `is_known_plugin_name`, which is deleted;
  - the old route stays as an alias of `POST /api/plugins/{id}/token`, with scopes `[keyring.own]`.
- `/api/command` (`show.rs`): a third credential branch for `x-plugin-token`. `/ws`: a `{cmd, plugin_token}` envelope beside `device_token` and `host_token`. Both go to `run_command_pipeline(…, Plugin{id})`. Plugin commands skip the console lock, but are attributed in logs and audit.

**Web loading gate:**
- `main.ts` stops hard-coding the plugin. It fetches `GET /api/plugins?kind=web&active=1` with the host token and loads only the listed entries.
- Server-side, a small middleware ahead of `ServeDir` in `create_router` answers `/plugins/*` only for files that belong to an active, digest-matching web plugin. Anything else gets a 404.
- `registerPlugin` scopes the keyring by manifest `id`, not by display name (fixes problem 4).

---

## 6. Migration and back-compat

**Removed: legacy native files and web scripts.** This section once described synthesizing a plugin
for each bare `.so/.dll/.dylib` (and a bare `.js` with no manifest) and approving the native ones once
when permissions first shipped (`plugins.grandfathered_v1`). None of that exists now: those files are
ignored with a log warning, no plugin is ever approved automatically, and `plugin.json` must declare
`"abi": 2`. A directory-form plugin without `plugin.json` is also ignored.

**`web/plugins/hello_world.js`:**
- Ship `web/plugins/hello_world.plugin.json` with `id: "hello_world"` (so existing `plugin:hello_world` keyring rows keep matching), `kind: web`, and permissions `[ui.dom, keyring.own]`.
- Bundled plugins under the read-only `web_dir` are first-party but still `pending` by default. **Recommendation:** stop auto-loading the demo button. It becomes opt-in in Settings → Plugins, which also makes it a good live demo of the consent flow.

**Keyring:** owner tags `plugin:<id>` stay unchanged. Web plugin ids equal the file stem, which already matches.

**Docs:**
- `docs/PLUGINS.md` gets a manifest/permissions section and a threat-model box.
- `docs/CLIENT_PAIRING.md` gets a line saying only console credentials may grant plugin permissions.

**New example:**
- `examples/plugin_observer/` (or `[[example]] name="plugin_observer" crate-type=["cdylib"]`) with a manifest and `export_plugin!`.
- It is used by an optional real-dlopen test and as the template in the docs.

---

## 7. Testing plan

> Tests for native plugins (hooks, the dlopen test) were removed with them.

**Unit tests**, in `#[cfg(test)]` modules next to the code, like `routes/common.rs`:
- Manifest parsing and validation:
  - bad ids, path traversal and symlinks, an unknown permission with the same `manifest_version` (rejected) and a newer one (incompatible), `host_api` semver mismatch, an artifact whose hash differs from the declared hash.
- `artifact_digest` is deterministic and independent of order. Changing one byte changes it.
- Signatures: good, bad, wrong key, signed then unsigned. Use the `minisign` dev-dependency pattern already used in `updater.rs:423`.
- `PermissionSet`:
  - `allows_command` handles allowlists, `"*"` and navigation-tier classification;
  - `provides(category)`;
  - `command_variant` handles both string and object variants.
- `needs_host_token` covers the new `/api/plugins*` GETs and POSTs. Extend `gate_tests` in `routes/mod.rs`.

**Integration tests** in `tests/core_tests.rs`, following the existing `register_plugin` and `AppState`+`create_router`+`reqwest` pattern (e.g. `:4748`):
1. `test_plugin_dispatch_respects_grants`: a plugin without `commands.intercept` is never asked; one without `show.observe` gets no state changes; `resources.provide:[songs]` contributes to songs but not themes; revoking stops dispatch at once.
2. `test_safety_critical_uses_parsed_variant`: `AddToSchedule` with the title "SetAlert" can now be vetoed, and a real `SetAlert` still cannot.
3. `test_ws_and_rest_give_plugins_identical_command_text`.
4. `test_plugin_grant_routes_require_console`:
   - no credential gets 401, device token 401, plugin token 401;
   - host token approves;
   - a stale `digest` gets 409;
   - an audit row is written with the right `decided_by`.
5. `test_changed_digest_blocks_load`: build a temp `plugin_dir` with a dummy file, approve it, modify a byte, and check that discovery reports `changed` and the loader skips it.
6. ~~`test_legacy_grandfathering_runs_once`~~ (removed with grandfathering; replaced by `nothing_is_approved_automatically_and_a_changed_binary_stops_loading_until_reapproved` and `bare_libraries_and_scripts_are_ignored_and_old_rows_are_purged`).
7. `test_scoped_plugin_token`:
   - `keyring.own` allows `/api/keyring/set` in its own namespace;
   - `/api/command` with `NextSlide` (allowlisted) returns 200, `NewSchedule` returns 403, and there is no console-lock interference;
   - revoking makes the next call 401;
   - TTL expiry, by injecting `expires_at` in the past.
8. `test_plugin_host_send_command_is_queued_and_gated`: the in-process `PluginHost` with the queue drained by a test runtime; rejections are recorded as coalesced `denied_call` audit rows.
9. Optional, `#[ignore]`: `test_real_cdylib_abi_marker`, which builds `examples/plugin_observer` and `dlopen`s it, plus a negative test with an ABI mismatch.

**Web tests** (vitest, `web/tests/plugins_runtime.test.ts` and `keyring.test.ts`):
- The loader loads only the approved list.
- `registerPlugin` scopes the keyring by `id`.
- The token request uses the new route.

---

## 8. Phased implementation

> History: phases 1 to 3 include native gating and the native host API, which no longer exist.

Each phase ships on its own.

**Phase 0: hardening (small, no schema change)**
- `ws.rs`: compute the plugin-visible text after shorthand expansion. `plugins.rs`: safety-critical decided by parsed variant. `ws.rs`: TTL and cap on plugin tokens. `web/src/core/plugins.ts`: fix the keyring name used for `hello_world`.
- Tests 2 and 3.
- Risk: a plugin that matched on the `{"cmd":…}` envelope text over WS changes behaviour. That is unlikely and is called out in the changelog.

**Phase 1: manifest, grants and native gating (the core)**
- New: `src/core/plugin_permissions.rs`, `src/core/plugin_registry.rs` (discovery, digests), `src/storage/db/plugins.rs`, `src/api/routes/plugins.rs`, and a Settings → Plugins UI module in `web/src/`.
- Changed:
  - `plugins.rs`: `LoadedPlugin` and `GrantCell`, gated dispatch, `set_grants`/`revoke`;
  - `main.rs`: discovery and load from approved candidates;
  - `db/core.rs`: the two tables;
  - `routes/mod.rs`: merge the router and update `needs_host_token`;
  - `routes/common.rs`: `merge_with_plugin_resources` provenance stamping;
  - `web/src/main.ts` and the `/plugins/*` serve gate;
  - `hello_world.plugin.json`, Cargo deb assets, docs.
- Tests 1, 4, 5 and 6, plus the unit tests.
- Risks:
  - forgetting to grandfather breaks a church's existing plugin on upgrade (covered by test 6);
  - digest work at startup is cheap (a few MB of SHA-256).

**Phase 2: scoped tokens and signatures**
- `ws.rs`: the `PluginTokenInfo` and `resolve` changes. `keyring.rs`: grant-based checks; delete `is_known_plugin_name`. `routes/plugins.rs`: the `/token` route. `show.rs` and `ws.rs`: the plugin credential branch. The `run_command_pipeline` refactor goes here.
- Minisign TOFU in `plugin_registry.rs`.
- Test 7 and the signature unit tests.
- Risk: the pipeline refactor touches the live-show hot path. Land it in its own PR with the existing WS and REST command tests unchanged.

**Phase 3: native host API v2**
- `plugins.rs`: the `PluginHost` trait, `create_plugin_v2`, ABI statics check, `export_plugin!` macro. A new `AppPluginHost` in `src/api/plugin_host.rs` with a bounded command queue task.
- `examples/plugin_observer` and its manifest. Docs.
- Tests 8 and 9.
- Risks:
  - ABI misuse is still undefined behaviour if someone hand-rolls the statics;
  - a blocking plugin hook still stalls a tokio worker. A possible follow-up is `spawn_blocking` with a timeout per hook.

**Phase 4: sandboxed web plugins**
- `/plugin-frame/<id>/…` served with a per-plugin CSP: `default-src 'none'; script-src 'self'; connect-src <network.outbound hosts>`, plus `sandbox allow-scripts`.
- A `postMessage` bridge in `web/src/core/plugin_bridge.ts`. It mints a scoped token through the console, proxies `sendCommand` and state events, and checks `show.observe` and `commands.send`.
- `ui.panel` and `ui.toolbar` slots (both implemented).
- Same-realm loading stays available only under `ui.dom` (Critical).
- Risks: the largest phase; the `strict` CSP mode and `frame_mode` settings interact with this and need testing.

---

## Open questions for the owner, with recommendations

1. **Grandfather existing legacy native plugins on upgrade?** *Was* "yes, once"; the whole legacy path has since been removed, so nothing is grandfathered.
2. **Must native-plugin approval require the per-run host token, or is a persistent console credential enough?** Recommend **a console credential is enough** for now, because installing still needs file access to the server. If an "install from file" upload API is ever added, that route must require the per-run host token.
3. **Signed same-key updates that request no new permissions: auto-approve or one-click?** Recommend **one-click**, never silent. A live service shouldn't change code without someone seeing it.
4. **Do plugin-originated commands bypass the one-console lock?** Recommend **yes**, attributed in audit. Plugins aren't consoles, and a DMX panic button shouldn't fail because a console is connected.
5. **Should library and state GETs require auth so that `library.read` means something over HTTP?** Recommend **no**. The live, stage and remote pages depend on zero-auth reads, so document it as informational.
6. **Copy the library to a private cache before `dlopen` to close the TOCTOU window?** Recommend **no**. It breaks plugins that load sibling libraries, and the attacker it stops can already edit `library.db`.
7. **HMAC grant rows with a key in the OS keyring?** Recommend **no**, for the same reason as question 6. It adds complexity for little real protection.
8. **Keep auto-loading `hello_world.js` by default?** Recommend **no**. Make it opt-in through the new consent screen.
9. **Offer `commands.modify` (rewriting commands) or `keyring.service` (shared namespaces) in v1?** The recommendation was to reserve both until a real plugin needed them; both have since been implemented, with the safeguards in docs/PLUGINS.md (same-kind rewrites only, never the emergency controls, native plugins only; shared namespaces limited to `OpenSanctuary:Shared:`).

**New dependencies: none.** Everything uses serde_json, sha2, minisign-verify (minisign is already a dev-dependency), semver, uuid, rusqlite and libloading, which are all already present.