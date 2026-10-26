# Plugin permission system design for OpenSanctuary (OS-Next)

Status: **implemented.** This began as a design (written by an Opus design pass) and is kept as the reference for how plugin permissions work. What exists today:

- **Manifest, grants, digest pinning, signer pinning, audit log, scoped plugin tokens**, and a console review screen (Settings > Plugins) where only the console can approve, deny, revoke or **override** (an Advanced mode that grants exactly the permissions the operator picks, behind a warning; see section 4).
- **Two kinds of plugin:** *sandboxed web* (an isolated frame behind a permission-checked bridge: the recommended kind) and *console script* (`ui.dom`: runs in the console page, Critical). **There are no native plugins:** loading compiled libraries into the server (the `OsPlugin` trait, the ABI 2 `PluginHost` API, `commands.intercept`, `commands.modify`, `resources.provide`, `native.code` and the filesystem/process/listen permissions that only made sense for them) was removed, as were the pre-manifest forms before it. A manifest with a `backend` section is refused.
- **Permissions in force:** every id in the table in section 2 marked enforced. (`settings.read` and `settings.write` were reserved ids that nothing ever acted on; they were removed rather than left looking like real grants.)
- **Features built on it:** placements (`ui.placements`) and ribbon buttons (`ui.toolbar`), `net.fetch` with limited headers, `library.addSong/addScriptures/addMedia/addMediaUrl`, `process.run` with host-managed signed tools, `notifications.post` and shared keyring namespaces (`keyring.service`). The bundled plugins (`web/plugins/`) are the worked examples. The guide for writing a plugin is `docs/PLUGINS.md`; this document is the permission model behind it.
- **Commands** from REST, WebSocket and a sandboxed plugin's `sendCommand` all go through `src/api/command_pipeline.rs`: shorthand expansion, parse, execute, persist, broadcast.
---

## 1. Threat model and honest limits

**Assets:** what goes out live to the projector and stream, the schedule and library, keyring secrets, the host token and console credentials, and the server machine itself.

**What a permission system can enforce, by plugin type:**

| Plugin type | Enforceable? | Why |
|---|---|---|
| **Same-realm web plugins (`ui.dom`)** | **Nothing, beyond whether the script loads at all.** | Same origin and same JS realm as the console. The plugin can take the host token or console credential and act as the console, including approving other plugins. |
| **Sandboxed web plugins (iframe without `allow-same-origin`)** | **Fully.** | Opaque origin with no access to the parent's storage or host token. It only talks through a `postMessage` bridge that the host checks against grants. It authenticates to the server with a scoped plugin token. A per-plugin CSP `connect-src` enforces `network.outbound`. |
| **Out-of-process HTTP/WS clients holding a plugin token** | **Fully, for the routes that require auth.** | Scoped tokens are checked on each request. Caveat: library and state GETs are deliberately zero-auth (for the live, stage and remote pages), so a `library.read` permission over HTTP restricts nothing. |

**Threats in scope:**
- (a) The operator approves a plugin without knowing what it asks for. Consent and plain-language tiers address this.
- (b) A plugin binary or script is swapped or updated after approval and runs on the old consent. Digest pinning and optional signature TOFU address this.
- (c) A buggy but honest plugin uses host features it did not declare. Hook and host-API gating addresses this.
- (d) A plugin token is used beyond its purpose, whether stolen or misused. Scopes, TTLs and revocation address this.
- (e) A device or remote credential tries to grant plugin permissions. Only console credentials may grant.

**Threats out of scope (stated in the UI and docs):**
- A local attacker with the OS user's write access. They can edit `library.db` grants, the plugin directory, or autostart entries.

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
| `network.outbound` | `hosts[]`, optional `headers[]` (extra request headers for `net.fetch`; see PLUGINS.md "Extra request headers") | Medium | **E** (iframe CSP, host fetch) | The server fetches only from these hosts, sending `Authorization`/`Accept`/`Accept-Language` and the declared headers. A header that routes or authenticates the request itself (`Host`, `Cookie`, `Proxy-*`, the app's own tokens...) can never be declared. |
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
- If `plugin.json.minisig` exists, it is verified with `minisign_verify::Signature`. The signature covers the bytes of `plugin.json`. If the manifest lists every other file's SHA-256 in `files`, the one signature covers the whole package (*fully signed*, and the host refuses a package whose files differ from the list); without `files` it covers the manifest only (*manifest signed*, and the review screen says so). Digest pinning, below, separately catches any change after approval. See `docs/PLUGIN_SIGNING.md` for what comes next.
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
GET    /api/plugins                    -> [{id,name,version,kind,status,signed,signer_key_id,observed_digest,approved_digest,requested,granted,tiers}]
GET    /api/plugins/permissions        -> every permission id the host knows (for the Advanced override, section 4)
POST   /api/plugins/{id}/approve       {digest, granted:[perm…]} | {digest, override:[perm…]}   // granted ⊆ requested; override replaces it outright
POST   /api/plugins/{id}/deny          {digest}
POST   /api/plugins/{id}/revoke        {}
DELETE /api/plugins/{id}               // uninstall
POST   /api/plugins/install            // upload+install a package (docs/PLUGIN_SIGNING.md's install path)
GET    /api/plugins/audit?limit=200
POST   /api/plugins/{id}/token         {scopes?:[perm…]}           // console mints scoped token for a web plugin
GET    /api/plugins/publishers         // trust store (docs/PLUGIN_SIGNING.md section 4)
POST   /api/plugins/publishers/{key}/trust|untrust|revoke
GET    /api/plugins/revocations        // revocation-list status (docs/PLUGIN_SIGNING.md section 5)
```
`/plugin-frame/{id}/*` (the sandboxed iframe) and `/api/plugin-host/*` (fetch, library writes, `process.run`, signed-tool status/update) are the plugin-facing side of this, not console grant management; see `docs/PLUGINS.md`.

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

**Commands.** REST (`/api/command`), WebSocket (`/ws`) and a sandboxed plugin's `sendCommand` all go through `src/api/command_pipeline.rs`: shorthand expansion, parse, origin check, execute, persist, broadcast. A plugin's origin is checked against its live grant (`commands.send` must allow the parsed variant, not a substring of the text). Plugin commands skip the one-console lock but are attributed in the log and audit table.

**Plugin tokens** (`src/api/ws.rs`, `routes/keyring.rs`):
- A token carries the plugin id, the scopes it was minted with and an expiry (12 hours, at most 8 per plugin).
- A call is allowed by `scopes ∩ live grant`, so revoking a plugin kills its tokens at once.
- `/api/command` accepts `x-plugin-token`; `/ws` accepts a `{cmd, plugin_token}` envelope; the keyring routes require `keyring.own` (or `keyring.service` for shared namespaces).

**Web loading gate.**
- The console fetches `GET /api/plugins?kind=web&active=1` with the host token and loads only the listed entries. Nothing is auto-loaded.
- A middleware ahead of the static file server answers `/plugins/*` only for files that belong to an active, digest-matching plugin; anything else gets a 404.
- A plugin's keyring is scoped by its manifest `id`, not by its display name.

**Sandbox.** `/plugin-frame/<id>/…` is served with a per-plugin CSP (`default-src 'none'; script-src 'self'; connect-src <network.outbound hosts>`) and `sandbox allow-scripts`. A `postMessage` bridge (`web/src/core/plugin_bridge.ts`) mints a scoped token through the console and proxies `sendCommand` and state events, checking `show.observe` and `commands.send`. Same-realm loading exists only under `ui.dom` (Critical).

**Tests** live next to the code: manifest and digest unit tests in `src/core/plugin_permissions.rs`, the grant, token and gate integration tests in `tests/core_tests/plugins.rs`, and the loader and keyring scoping tests in `web/tests/`.
