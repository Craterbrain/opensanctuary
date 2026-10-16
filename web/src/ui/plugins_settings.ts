/**
 * Settings > Plugins: review what each installed plugin asks for, approve or deny it,
 * revoke it later, and see the audit trail (docs/PLUGIN_PERMISSIONS.md).
 *
 * Everything below comes from a plugin's own manifest, which is untrusted input: it is
 * only ever inserted with textContent, never innerHTML.
 */
import { api, type PluginAuditEntry, type PluginPublisher, type PluginTrustLevel, type PluginCatalogEntry, type PluginGrantInput, type PluginInfo, type PluginPermissionInfo } from "../core/api_client.ts";
import { showToast } from "../core/ui_utils.ts";
import { getCurrentIconSet, iconUrl } from "../core/icon_sets.ts";

const TIER_LABEL: Record<string, string> = { low: 'Low', medium: 'Medium', high: 'High', critical: 'Critical' };
const ENFORCEMENT_NOTE: Record<string, string> = {
  enforced: 'enforced by OpenSanctuary',
  'consent-only': 'declared, not enforced',
  informational: 'informational',
  reserved: 'reserved (not active yet)',
  unknown: 'unknown',
};
const STATE_LABEL: Record<string, string> = {
  pending: 'Needs review',
  active: 'Approved',
  changed: 'Changed — review again',
  denied: 'Denied',
  revoked: 'Revoked',
  incompatible: 'Incompatible',
  policy_blocked: 'Blocked by policy',
};
const POLICY_LABEL: Record<string, string> = { allow: 'Allow', warn: 'Warn', require: 'Require' };
const POLICY_HINT: Record<string, string> = {
  allow: 'Unsigned and signed plugins are reviewed the same way.',
  warn: 'Approving an unsigned plugin needs an extra confirmation.',
  require: 'Only a built-in or a signed plugin may load at all -- for a console that only installs from one trusted source.',
};

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

function paramsText(p: PluginPermissionInfo): string {
  const parts: string[] = [];
  if (p.categories?.length) parts.push(p.categories.join(', '));
  if (p.commands?.length) parts.push(p.commands.join(', '));
  if (p.keys?.length) parts.push(p.keys.join(', '));
  if (p.hosts?.length) parts.push(p.hosts.join(', '));
  if (p.paths?.length) parts.push(p.paths.join(', '));
  if (p.ports?.length) parts.push(p.ports.join(', '));
  if (p.services?.length) parts.push(`${p.services.join(', ')} (${p.access ?? 'read'})`);
  return parts.join(' · ');
}


type ScopeKey = 'categories' | 'commands' | 'hosts' | 'paths' | 'ports' | 'keys' | 'services';

function scopeValues(p: PluginPermissionInfo | undefined, scope: string): string {
  if (!p || scope === 'none') return '';
  return ((p as any)[scope] ?? []).join(', ');
}

/** The Advanced override editor: every permission toggleable and re-scopable, plus any the plugin never asked for. */
function buildAdvanced(plugin: PluginInfo, onChange: (on: boolean) => void): { root: HTMLElement; collect: () => PluginGrantInput[] | null; isOn: () => boolean } {
  const root = el('div', 'plugin-advanced');
  const toggleRow = el('label', 'plugin-advanced-toggle');
  const toggle = el('input');
  toggle.type = 'checkbox';
  toggleRow.appendChild(toggle);
  toggleRow.appendChild(el('span', undefined, ' Advanced: override what this plugin asks for'));
  root.appendChild(toggleRow);

  const panel = el('div', 'plugin-advanced-panel');
  panel.hidden = true;
  root.appendChild(panel);

  const rows: Array<{ id: string; scope: ScopeKey | 'none'; box: HTMLInputElement; input: HTMLInputElement | null; access: HTMLSelectElement | null }> = [];
  let ack: HTMLInputElement | null = null;
  let built = false;

  const addRow = (container: HTMLElement, id: string, tier: string, enforcement: string, scope: string, current: PluginPermissionInfo | undefined, checked: boolean, note?: string) => {
    const row = el('div', `plugin-permission plugin-permission--${tier} plugin-advanced-row`);
    const label = el('label', 'plugin-advanced-label');
    const box = el('input');
    box.type = 'checkbox';
    box.checked = checked;
    label.appendChild(box);
    label.appendChild(el('span', 'plugin-permission-id', id));
    label.appendChild(el('span', `os-badge plugin-tier plugin-tier--${tier}`, TIER_LABEL[tier] ?? tier));
    label.appendChild(el('span', 'settings-hint', ` — ${ENFORCEMENT_NOTE[enforcement] ?? enforcement}`));
    row.appendChild(label);
    let input: HTMLInputElement | null = null;
    if (scope !== 'none') {
      input = el('input', 'plugin-advanced-scope');
      input.type = 'text';
      input.value = scopeValues(current, scope);
      input.placeholder = scope === 'ports' ? 'ports, comma separated' : `${scope}, comma separated (* = all where allowed)`;
      input.setAttribute('aria-label', `${id} ${scope}`);
      row.appendChild(input);
    }
    let access: HTMLSelectElement | null = null;
    if (id === 'keyring.service') {
      access = el('select', 'plugin-advanced-access');
      for (const a of ['read', 'write']) {
        const o = el('option', undefined, a === 'read' ? 'read only' : 'read and write its own entries');
        o.value = a;
        access.appendChild(o);
      }
      access.value = current?.access ?? 'read';
      access.setAttribute('aria-label', 'keyring.service access');
      row.appendChild(access);
    }
    if (note) row.appendChild(el('div', 'plugin-permission-reason', note));
    container.appendChild(row);
    rows.push({ id, scope: scope as ScopeKey | 'none', box, input, access });
  };

  const build = async () => {
    built = true;
    panel.appendChild(el('div', 'plugin-warning',
      'Advanced override. You are choosing the permissions yourself instead of the ones this plugin asked for. ' +
      'Turning off something it needs, narrowing it, or adding permissions it never asked for can make it misbehave or stop working, ' +
      'and extra permissions give it more access than its author intended. Only do this if you know what you are doing.'));
    let catalog: PluginCatalogEntry[] = [];
    try { catalog = await api.plugins.permissionCatalog(); } catch (e: any) {
      panel.appendChild(el('div', 'plugin-error', `Couldn't load the list of permissions: ${e?.message ?? e}`));
      return;
    }
    const byId = new Map(catalog.map((c) => [c.id, c]));
    const start = plugin.state === 'active' ? plugin.granted : plugin.requested;
    panel.appendChild(el('div', 'settings-hint', 'Permissions it asked for (uncheck to withhold, edit the lists to narrow or widen):'));
    const asked = el('div', 'plugin-permissions');
    for (const p of start) {
      const c = byId.get(p.id);
      addRow(asked, p.id, p.tier, p.enforcement, c?.scope ?? 'none', p, true, p.reason ? p.reason : undefined);
    }
    if (!start.length) asked.appendChild(el('div', 'settings-hint', 'It asked for nothing.'));
    panel.appendChild(asked);

    const extra = catalog.filter((c) => !start.some((p) => p.id === c.id));
    panel.appendChild(el('div', 'settings-hint', 'Other permissions it did not ask for (check to grant):'));
    const extras = el('div', 'plugin-permissions');
    for (const c of extra) {
      const unusable = c.id === 'ui.dom' && plugin.kind !== 'web'; // a sandboxed page has no console access to grant
      if (unusable) continue;
      addRow(extras, c.id, c.tier, c.enforcement, c.scope, undefined, false);
    }
    panel.appendChild(extras);

    const ackRow = el('label', 'plugin-advanced-ack');
    ack = el('input');
    ack.type = 'checkbox';
    ackRow.appendChild(ack);
    ackRow.appendChild(el('span', undefined, ' I understand this may break the plugin or give it more access than intended.'));
    panel.appendChild(ackRow);
  };

  toggle.addEventListener('change', async () => {
    panel.hidden = !toggle.checked;
    if (toggle.checked && !built) await build();
    onChange(toggle.checked);
  });

  const collect = (): PluginGrantInput[] | null => {
    if (!ack?.checked) {
      showToast('Tick "I understand this may break the plugin" to apply an override.');
      return null;
    }
    const out: PluginGrantInput[] = [];
    for (const r of rows) {
      if (!r.box.checked) continue;
      const g: PluginGrantInput = { id: r.id };
      if (r.input && r.scope !== 'none') {
        const items = r.input.value.split(',').map((x) => x.trim()).filter(Boolean);
        if (r.scope === 'ports') {
          const ports = items.map(Number);
          if (ports.some((n) => !Number.isInteger(n) || n < 1 || n > 65535)) {
            showToast(`${r.id}: ports must be numbers from 1 to 65535.`);
            return null;
          }
          g.ports = ports;
        } else {
          (g as any)[r.scope] = items;
        }
      }
      if (r.access) g.access = r.access.value as 'read' | 'write';
      out.push(g);
    }
    return out;
  };
  return { root, collect, isOn: () => toggle.checked };
}

const TRUST_LABEL: Record<PluginTrustLevel, string> = {
  builtin: 'Built-in',
  trusted: 'Trusted publisher',
  known: 'Known publisher',
  new: 'New publisher',
  unsigned: 'Unsigned',
  invalid: 'Invalid signature',
  revoked: 'Revoked publisher',
};
const TRUST_HINT: Record<PluginTrustLevel, string> = {
  builtin: 'Ships with OpenSanctuary.',
  trusted: 'Signed by a publisher you chose to trust.',
  known: 'Signed by a publisher you have approved a plugin from before, but not one you chose to trust.',
  new: 'Signed by a publisher you have not seen before. A signature says who released it, not that it is safe: read what it asks for.',
  unsigned: 'Not signed, so nothing says who released it.',
  invalid: 'Its signature does not verify, so it cannot be installed or run.',
  revoked: 'Signed by a publisher you revoked.',
};

/** What the signature proves, in words: "signed" is only said when the signature covers every file. */
function signatureLabel(plugin: PluginInfo): string {
  const key = plugin.trust?.release_signed ? ' by the OpenSanctuary release key' : plugin.signer_id ? ` (publisher key ${plugin.signer_id})` : '';
  switch (plugin.signature_scope ?? (plugin.signed ? 'manifest' : 'unsigned')) {
    case 'full': return `signed${key}, covers every file`;
    case 'manifest': return `manifest signed${key}; the other files aren't covered by the signature`;
    default: return 'unsigned';
  }
}

/**
 * Independent of `changes` (which only compares against the single most recent approval, and
 * only once a plugin has already been approved at least once): this plugin's version is older
 * than the highest one ever approved from this same signer, remembered even across a
 * revoke/re-approve or an uninstall/reinstall (docs/PLUGIN_SIGNING.md section 7).
 */
function renderVersionFloorWarning(plugin: PluginInfo): HTMLElement | null {
  if (!plugin.is_downgrade_from_floor) return null;
  return el('div', 'plugin-warning', plugin.version_floor
    ? `This is v${plugin.version ?? '?'}, but a newer v${plugin.version_floor} was approved from this publisher before. Rolling back can bring back fixed bugs and old security holes.`
    : 'This is an older version than one approved from this publisher before.');
}

/** "What changed" for an approved plugin whose files differ from what was reviewed. */
function renderChanges(plugin: PluginInfo): HTMLElement | null {
  const c = plugin.changes;
  if (!c) return null;
  const box = el('div', 'plugin-changes');
  const from = c.from_version ? `v${c.from_version}` : 'the version you approved';
  box.appendChild(el('strong', 'plugin-changes-title', c.to_version ? `Update: ${from} → v${c.to_version}` : `Changed since ${from}`));
  if (c.direction === 'downgrade') {
    box.appendChild(el('div', 'plugin-warning', 'This is an OLDER version than the one you approved. Rolling back can bring back fixed bugs and old security holes.'));
  } else if (c.direction === 'same') {
    box.appendChild(el('div', 'plugin-warning', "The version number didn't change, but the files did."));
  }
  const p = c.permissions;
  if (plugin.trust?.level === 'trusted') box.appendChild(el('div', 'settings-hint', 'Signed by a publisher you trust. That never skips this review.'));
  if (p.added.length) {
    const tiers = new Map(p.added_tiers.map((t) => [t.id, t.tier]));
    box.appendChild(el('div', 'plugin-warning plugin-changes-perms', `New permissions requested: ${p.added.map((id) => `${id} (${tiers.get(id) ?? 'unknown risk'})`).join(', ')}.`));
  }
  if (p.changed.length) box.appendChild(el('div', 'plugin-warning plugin-changes-perms', `Wider or different scope: ${p.changed.join(', ')}.`));
  if (!p.added.length && !p.changed.length) box.appendChild(el('div', 'settings-hint', p.removed.length ? `No new permissions (it no longer asks for: ${p.removed.join(', ')}).` : 'No new permissions.'));
  if (c.files) {
    const parts: string[] = [];
    if (c.files.modified.length) parts.push(`edited: ${c.files.modified.join(', ')}`);
    if (c.files.added.length) parts.push(`added: ${c.files.added.join(', ')}`);
    if (c.files.removed.length) parts.push(`removed: ${c.files.removed.join(', ')}`);
    box.appendChild(el('div', 'settings-hint plugin-changes-files', parts.length ? `Files ${parts.join(' · ')}` : 'No file changes.'));
  } else {
    box.appendChild(el('div', 'settings-hint', 'The list of changed files is not available for a plugin approved before updates were tracked.'));
  }
  return box;
}

/** Opens the file chooser for a plugin package and hands the chosen .zip over. */
function pickPackage(onFile: (file: File) => void): void {
  const input = document.createElement('input');
  input.type = 'file';
  input.accept = '.zip,application/zip';
  input.addEventListener('change', () => { const f = input.files?.[0]; if (f) onFile(f); });
  input.click();
}

function renderPlugin(plugin: PluginInfo, refresh: () => void, policy: string): HTMLElement {
  const card = el('div', 'network-card plugin-card');
  card.dataset.pluginId = plugin.id;

  const head = el('div', 'plugin-card-head');
  head.appendChild(el('strong', 'plugin-name', plugin.name));
  if (plugin.version) head.appendChild(el('span', 'plugin-version', ` v${plugin.version}`));
  head.appendChild(el('span', `os-badge plugin-state plugin-state--${plugin.state}`, STATE_LABEL[plugin.state] ?? plugin.state));
  head.appendChild(el('span', 'os-badge os-badge--neutral', plugin.kind === 'sandbox' ? 'Sandboxed page' : 'Console script'));
  if (plugin.trust) {
    const t = el('span', `os-badge plugin-trust plugin-trust--${plugin.trust.level}`, TRUST_LABEL[plugin.trust.level]);
    t.title = TRUST_HINT[plugin.trust.level];
    head.appendChild(t);
  }
  if (plugin.overridden) head.appendChild(el('span', 'os-badge plugin-state plugin-state--changed', 'Permissions overridden'));
  card.appendChild(head);
  card.appendChild(el('div', 'settings-hint plugin-id-line', `id: ${plugin.id}${plugin.trust?.publisher_name ? ` · ${plugin.trust.name_is_claim ? 'says it is by' : 'by'} ${plugin.trust.publisher_name}` : ''} · digest ${plugin.digest.slice(0, 12)}… · ${signatureLabel(plugin)}`));
  if (plugin.trust?.rotated_from_key_id) {
    card.appendChild(el('div', 'settings-hint plugin-rotation', `This plugin is signed by a new key. The publisher's previous key (${plugin.trust.rotated_from_key_id}) signed a statement that it moved to this one, so the trust you gave the old key carries over.`));
  }
  if (plugin.signature_weaker) {
    card.appendChild(el('div', 'plugin-warning', 'This plugin was fully signed when you approved it. Now the signature no longer covers all of its files. Treat this update with more suspicion.'));
  }
  const changes = renderChanges(plugin);
  if (changes) card.appendChild(changes);
  // Don't repeat it: `changes.direction === 'downgrade'` already says this when it applies.
  if (plugin.changes?.direction !== 'downgrade') {
    const floorWarning = renderVersionFloorWarning(plugin);
    if (floorWarning) card.appendChild(floorWarning);
  }
  if (plugin.signer_changed) {
    card.appendChild(el('div', 'plugin-warning', 'The publisher signature is different from the one you approved before (or it used to be signed and now isn\'t). Check that this update is from who you expect.'));
  }

  if (plugin.error) {
    card.appendChild(el('div', 'plugin-error', `Can't be approved: ${plugin.error}`));
  }
  if (plugin.state === 'changed') {
    card.appendChild(el('div', 'plugin-warning', 'This plugin\'s files changed since you approved it, so it is not running. Review the permissions again to re-approve.'));
  }
  if (plugin.state === 'policy_blocked') {
    card.appendChild(el('div', 'plugin-warning',
      "Approved, but this console's plugin signature policy (Require) only loads a built-in or a signed plugin, and this one is unsigned. It stays off until the policy is loosened or a signed build is installed."));
  }
  // A built-in is never touched by policy or the unsigned-confirmation rules below, same as the
  // server's `plugin_policy::approval_gate`/`permits_load` (both check builtin before signed).
  const builtin = plugin.trust?.level === 'builtin';
  // Can the operator even approve this one? `require` refuses an unsigned, non-built-in plugin
  // outright -- no confirmation unlocks it, the policy itself has to change first.
  const unsignedRefused = policy === 'require' && !builtin && !plugin.signed && plugin.state !== 'active' && plugin.state !== 'policy_blocked';
  if (unsignedRefused) {
    card.appendChild(el('div', 'plugin-error',
      "Can't be approved: this console's plugin signature policy (Settings → Plugins → Policy) requires a signed plugin or a built-in."));
  }
  if (plugin.kind === 'sandbox' && plugin.state !== 'active') {
    card.appendChild(el('div', 'settings-hint',
      'Runs in an isolated frame with no access to this console. It can only do what is checked below, and can reach the network only at the hosts listed (if any).'));
  }
  const critical = plugin.requested.some(p => p.id === 'ui.dom');
  if (critical && plugin.state !== 'active') {
    card.appendChild(el('div', 'plugin-warning',
      'This script runs inside the console page itself, with the same access as you. The list below is what it says it will do.'));
  }

  const checks: Array<{ id: string; box: HTMLInputElement }> = [];
  const list = el('div', 'plugin-permissions');
  const shown = plugin.state === 'active' ? plugin.granted : plugin.requested;
  for (const perm of shown) {
    const row = el('label', `plugin-permission plugin-permission--${perm.tier}`);
    const box = el('input');
    box.type = 'checkbox';
    box.checked = true;
    box.disabled = plugin.state === 'active' || !perm.optional || !!plugin.error;
    if (plugin.state !== 'active') checks.push({ id: perm.id, box });
    row.appendChild(box);
    row.appendChild(el('span', 'plugin-permission-id', perm.id));
    row.appendChild(el('span', `os-badge plugin-tier plugin-tier--${perm.tier}`, TIER_LABEL[perm.tier] ?? perm.tier));
    const params = paramsText(perm);
    if (params) row.appendChild(el('span', 'plugin-permission-params', params));
    if (perm.optional) row.appendChild(el('span', 'settings-hint', ' (optional)'));
    row.appendChild(el('span', 'settings-hint', ` — ${ENFORCEMENT_NOTE[perm.enforcement] ?? perm.enforcement}`));
    if (perm.reason) row.appendChild(el('div', 'plugin-permission-reason', perm.reason));
    list.appendChild(row);
  }
  if (!shown.length) list.appendChild(el('div', 'settings-hint', 'No permissions requested.'));
  card.appendChild(list);

  // What it can run and the tools the host installs for it -- the operator must be able to read exactly this.
  if (plugin.tools?.length) {
    card.appendChild(el('div', 'settings-hint', 'Tools OpenSanctuary installs and keeps current for this plugin (downloaded from the release, then checked against the signed checksums with the key shipped in the plugin):'));
    for (const t of plugin.tools) {
      const sig = `key ${t.key_sha256.slice(0, 16)}…`;
      card.appendChild(el('div', 'plugin-tool', `${t.id}: github.com/${t.github} (${t.checksums} signed by ${sig})` + (t.system_command ? `; uses a ${t.system_command} already on this computer if there is one` : '')));
    }
  }
  if (plugin.commands?.length) {
    card.appendChild(el('div', 'settings-hint', 'Commands this plugin can run (exactly these, never through a shell; values in { } are filled in and checked by OpenSanctuary):'));
    for (const c of plugin.commands) {
      card.appendChild(el('pre', 'plugin-command', c.line));
    }
  }
  if (plugin.placements?.length) {
    const where = plugin.placements.map((p) => p.slot === 'library-source' ? `the ${p.tab} library ("${p.label}")` : p.slot === 'video-picker' ? 'the video background picker' : 'a floating panel').join(', ');
    card.appendChild(el('div', 'settings-hint', `Appears in: ${where}.`));
  }

  if (plugin.toolbar?.length) {
    const line = el('div', 'settings-hint plugin-toolbar-preview');
    line.appendChild(document.createTextNode('Adds to the top ribbon: '));
    for (const t of plugin.toolbar) {
      const chip = el('span', 'plugin-toolbar-chip');
      const img = document.createElement('img');
      img.dataset.icon = t.icon;
      img.src = iconUrl(getCurrentIconSet(), t.icon);
      img.alt = '';
      chip.append(img, document.createTextNode(` ${t.label}`));
      if (t.tooltip) chip.title = t.tooltip;
      line.appendChild(chip);
    }
    line.appendChild(document.createTextNode('. Plugin buttons are marked with a puzzle badge and always sit in the Plugins group.'));
    card.appendChild(line);
  }

  // Shared keyring: say plainly what it can reach, who else can, and whether it could send it somewhere.
  const sharedPerms = (plugin.state === 'active' ? plugin.granted : plugin.requested).filter((p) => p.id === 'keyring.service');
  const reach = (plugin.state === 'active' ? plugin.granted : plugin.requested).filter((p) => p.id === 'network.outbound').flatMap((p) => p.hosts ?? []);
  for (const p of sharedPerms) {
    for (const svc of p.services ?? []) {
      const name = svc.replace('OpenSanctuary:Shared:', '');
      const write = p.access === 'write';
      card.appendChild(el('div', 'plugin-warning',
        `Can read secrets stored in the shared keyring area "${name}" (${svc})${write ? ', and store its own secrets there for other plugins to read' : ''}.`));
      const others = plugin.shared_with?.find((s) => s.service === svc)?.plugins ?? [];
      if (others.length) {
        card.appendChild(el('div', 'settings-hint', `Shared with: ${others.map((o) => `${o.name} (${o.access}${o.state === 'active' ? '' : ', not approved yet'})`).join(', ')}.`));
      }
      if (reach.length) {
        const risky = others.some((o) => o.access === 'write');
        card.appendChild(el('div', risky ? 'plugin-error' : 'plugin-warning',
          `It can also reach ${reach.join(', ')}. A shared secret could be sent there${risky ? ', and another plugin can change what is stored' : ''}.`));
      }
    }
  }

  if (plugin.overridden && plugin.override_diff) {
    const d = plugin.override_diff;
    const parts = [
      d.added.length ? `extra: ${d.added.join(', ')}` : '',
      d.removed.length ? `withheld: ${d.removed.join(', ')}` : '',
      d.changed.length ? `re-scoped: ${d.changed.join(', ')}` : '',
    ].filter(Boolean);
    card.appendChild(el('div', 'plugin-warning', `You overrode what this plugin asked for (${parts.join('; ')}). If it misbehaves, revoke it and approve again without the override.`));
  }
  let advancedOn = false;
  let advancedApply: HTMLButtonElement | null = null;
  let normalApprove: HTMLButtonElement | null = null;
  let advanced: ReturnType<typeof buildAdvanced> | null = null;
  if (!plugin.error && plugin.state !== 'incompatible') {
    advanced = buildAdvanced(plugin, (on) => {
      advancedOn = on;
      if (advancedApply) advancedApply.hidden = !on;
      if (normalApprove) normalApprove.hidden = on;
    });
    card.appendChild(advanced.root);
  }

  const actions = el('div', 'plugin-actions');
  const act = async (fn: () => Promise<unknown>, done: string) => {
    try {
      const updated = await fn();
      // Tell the console, so a sandboxed plugin starts/stops without a reload.
      window.dispatchEvent(new CustomEvent('os-plugin-changed', { detail: { plugin: updated } }));
      showToast(done);
      refresh();
    } catch (e: any) {
      showToast(`Plugin action failed: ${e?.message ?? e}`);
    }
  };
  let trustBox: HTMLInputElement | null = null;
  if (plugin.state !== 'active' && plugin.state !== 'policy_blocked' && !plugin.error && plugin.trust?.can_trust) {
    const label = el('label', 'plugin-trust-choice');
    trustBox = document.createElement('input');
    trustBox.type = 'checkbox';
    trustBox.className = 'plugin-trust-box';
    label.append(trustBox, document.createTextNode(` Trust everything ${plugin.trust.publisher_name ? `"${plugin.trust.publisher_name}" ` : ''}signs from now on (key ${plugin.trust.key_id}). It never skips this review.`));
    card.appendChild(label);
  }
  // Confirms (returning the flag to send) or bails (returning null) for approving an unsigned
  // plugin, mirroring the revoked-publisher confirmation above. Independent of policy: granting
  // a Critical permission to an unsigned plugin always asks; `warn` asks for any unsigned plugin.
  const confirmUnsignedIfNeeded = (grantedIds: string[]): boolean | null => {
    if (builtin || plugin.signed) return false;
    const grantingCritical = grantedIds.some((id) => plugin.requested.find((p) => p.id === id)?.tier === 'critical');
    if (policy !== 'warn' && !grantingCritical) return false;
    const why = policy === 'warn'
      ? "This console's plugin signature policy warns on unsigned plugins."
      : `"${plugin.name}" is unsigned and you are granting it a Critical permission.`;
    return window.confirm(`${why}\n\nApprove it anyway?`) ? true : null;
  };
  if (plugin.state !== 'active' && plugin.state !== 'policy_blocked' && !plugin.error && !unsignedRefused) {
    const approve = el('button', 'btn btn-primary', plugin.state === 'changed' ? 'Approve changes' : 'Approve');
    approve.addEventListener('click', () => {
      const granted = checks.filter(c => c.box.checked).map(c => c.id);
      let confirmRevokedPublisher = false;
      if (plugin.trust?.level === 'revoked') {
        if (!window.confirm(`"${plugin.name}" is signed by a publisher you revoked.\n\nApprove it anyway?`)) return;
        confirmRevokedPublisher = true;
      }
      const confirmUnsigned = confirmUnsignedIfNeeded(granted);
      if (confirmUnsigned === null) return;
      void act(() => api.plugins.approve(plugin.id, plugin.digest, granted, { trustPublisher: !!trustBox?.checked, confirmRevokedPublisher, confirmUnsigned }), `${plugin.name} approved`);
    });
    actions.appendChild(approve);
    normalApprove = approve;
  }
  if (plugin.state === 'pending' || plugin.state === 'changed') {
    const deny = el('button', 'btn btn-secondary', 'Deny');
    deny.addEventListener('click', () => void act(() => api.plugins.deny(plugin.id), `${plugin.name} denied`));
    actions.appendChild(deny);
  }
  if (plugin.state === 'active' && plugin.trust?.can_trust) {
    const trust = el('button', 'btn btn-secondary plugin-trust-now', 'Trust publisher');
    trust.title = `Trust everything this publisher signs (key ${plugin.trust.key_id}). It never skips a review.`;
    trust.addEventListener('click', () => void act(async () => { await api.plugins.trustPublisher(plugin.trust!.key_id!); return plugin; }, 'Publisher trusted'));
    actions.appendChild(trust);
  }
  if (plugin.state === 'active') {
    const revoke = el('button', 'btn btn-secondary', 'Revoke');
    revoke.addEventListener('click', () => void act(() => api.plugins.revoke(plugin.id), `${plugin.name} revoked`));
    actions.appendChild(revoke);
  }
  if (plugin.removable) {
    const update = el('button', 'btn btn-secondary plugin-update', 'Update…');
    update.addEventListener('click', () => pickPackage((file) => void act(() => api.plugins.install(file, true), `${plugin.name} updated: review it before it runs again`)));
    actions.appendChild(update);
    const remove = el('button', 'btn btn-danger plugin-remove', 'Remove');
    remove.addEventListener('click', () => {
      if (!window.confirm(`Remove "${plugin.name}"?\n\nIts files, its approval and any secrets it saved are deleted. This can't be undone.`)) return;
      void act(async () => {
        await api.plugins.uninstall(plugin.id);
        return { id: plugin.id, kind: 'sandbox', state: 'revoked' };
      }, `${plugin.name} removed`);
    });
    actions.appendChild(remove);
  }
  if (advanced && plugin.state !== 'incompatible' && !plugin.error && !unsignedRefused) {
    const apply = el('button', 'btn btn-danger', 'Apply override');
    apply.hidden = true;
    apply.addEventListener('click', () => {
      const grants = advanced!.collect();
      if (!grants) return;
      let confirmRevokedPublisher = false;
      if (plugin.trust?.level === 'revoked') {
        if (!window.confirm(`"${plugin.name}" is signed by a publisher you revoked.\n\nApprove it anyway?`)) return;
        confirmRevokedPublisher = true;
      }
      // An override can grant an id the plugin never requested, so its tier isn't known here the
      // way it is for a normal approval -- ask whenever it's unsigned at all, rather than trying
      // (and sometimes failing) to guess which grants are Critical.
      let confirmUnsigned = false;
      if (!builtin && !plugin.signed) {
        if (!window.confirm(`"${plugin.name}" is unsigned.\n\nApprove this override anyway?`)) return;
        confirmUnsigned = true;
      }
      void act(() => api.plugins.approveOverride(plugin.id, plugin.digest, grants, { confirmRevokedPublisher, confirmUnsigned }), `${plugin.name} updated with your override`);
    });
    actions.appendChild(apply);
    advancedApply = apply;
  }
  card.appendChild(actions);
  if (plugin.decided_at) {
    card.appendChild(el('div', 'settings-hint', `Last decision ${new Date(plugin.decided_at).toLocaleString()} by ${plugin.decided_by ?? 'unknown'}`));
  }
  return card;
}

function renderAudit(entries: PluginAuditEntry[]): HTMLElement {
  const wrap = el('details', 'plugin-audit');
  wrap.appendChild(el('summary', undefined, `Audit log (${entries.length})`));
  for (const a of entries) {
    wrap.appendChild(el('div', 'settings-hint', `${new Date(a.ts_ms).toLocaleString()} · ${a.plugin_id} · ${a.action} · ${a.actor}`));
  }
  return wrap;
}

/** The shared keyring entries (names and owners only, never values), with a delete for each: the operator's way to clean up. */
async function renderSharedSecrets(content: HTMLElement, refresh: () => void): Promise<void> {
  try {
    const entries = await api.plugins.sharedSecrets();
    if (!entries.length) return;
    const wrap = el('details', 'plugin-shared-secrets');
    wrap.appendChild(el('summary', undefined, `Shared secrets (${entries.length})`));
    wrap.appendChild(el('div', 'settings-hint', 'Secrets plugins keep in shared keyring areas. Values are never shown here.'));
    for (const e of entries) {
      const row = el('div', 'plugin-shared-secret');
      row.appendChild(el('span', 'plugin-permission-id', `${e.service} / ${e.account}`));
      row.appendChild(el('span', 'settings-hint', ` owner: ${e.owner}`));
      const del = el('button', 'btn btn-secondary btn-sm', 'Delete');
      del.addEventListener('click', async () => {
        try {
          await api.plugins.deleteSharedSecret(e.service, e.account);
          showToast('Deleted the shared secret');
          refresh();
        } catch (err: any) {
          showToast(`Couldn't delete it: ${err?.message ?? err}`);
        }
      });
      row.appendChild(del);
      wrap.appendChild(row);
    }
    content.appendChild(wrap);
  } catch { /* not a console page, or the list isn't available: nothing to show */ }
}

/** Publishers the operator trusted or revoked. */
async function renderPublishers(content: HTMLElement, refresh: () => void): Promise<void> {
  try {
    const list: PluginPublisher[] = await api.plugins.publishers();
    const wrap = el('details', 'plugin-publishers');
    wrap.open = list.length > 0;
    wrap.appendChild(el('summary', undefined, `Publishers (${list.length})`));
    try {
      const rev = await api.plugins.revocations();
      const line = el('div', 'settings-hint plugin-revocation-status', rev.applied_sequence
        ? `Revocation list #${rev.applied_sequence} applied (${rev.listed_keys} keys). It is signed by the release key and can only move forward.`
        : 'No revocation list has been applied.');
      wrap.appendChild(line);
      for (const p of rev.problems) wrap.appendChild(el('div', 'plugin-warning', `A revocation list was ignored: ${p}`));
      if (rev.applied_sequence || rev.problems.length) wrap.open = true; // news the operator should see
    } catch { /* older server, or not a console page */ }
    if (!list.length) {
      wrap.appendChild(el('div', 'settings-hint', 'No publishers trusted or revoked. Tick "Trust everything … signs" when you approve a signed plugin to add one.'));
    }
    for (const p of list) {
      const row = el('div', 'plugin-publisher');
      row.dataset.keyId = p.key_id;
      row.appendChild(el('strong', undefined, p.name || 'Unnamed publisher'));
      row.appendChild(el('span', `os-badge plugin-trust plugin-trust--${p.revoked ? 'revoked' : 'trusted'}`, p.revoked ? 'Revoked' : 'Trusted'));
      row.appendChild(el('span', 'settings-hint', ` key ${p.key_id} · ${p.plugins.length ? p.plugins.join(', ') : 'no plugins'}${p.revoked && p.note ? ` · ${p.note}` : ''}`));
      const doIt = async (fn: () => Promise<unknown>, done: string) => {
        try { await fn(); showToast(done); refresh(); } catch (e: any) { showToast(`Couldn't do that: ${e?.message ?? e}`); }
      };
      if (p.trusted) {
        const stop = el('button', 'btn btn-secondary btn-sm plugin-publisher-untrust', 'Stop trusting');
        stop.addEventListener('click', () => void doIt(() => api.plugins.untrustPublisher(p.key_id), 'No longer trusted'));
        row.appendChild(stop);
      }
      if (!p.revoked) {
        const revoke = el('button', 'btn btn-danger btn-sm plugin-publisher-revoke', 'Revoke');
        revoke.addEventListener('click', () => {
          if (!window.confirm(`Revoke "${p.name || p.key_id}"?\n\nEvery plugin it signed stops running, and approving one again needs an extra confirmation.`)) return;
          void doIt(async () => {
            const r = await api.plugins.revokePublisher(p.key_id);
            for (const id of r.stopped) window.dispatchEvent(new CustomEvent('os-plugin-changed', { detail: { plugin: { id, kind: 'sandbox', state: 'revoked' } } }));
          }, 'Publisher revoked');
        });
        row.appendChild(revoke);
      }
      wrap.appendChild(row);
    }
    content.appendChild(wrap);
  } catch { /* not a console page */ }
}

/** Settings > Plugins > Policy: how strictly this console treats an unsigned plugin (docs/PLUGIN_SIGNING.md section 6). */
function renderPolicyControl(content: HTMLElement, policy: string, refresh: () => void): void {
  const row = el('div', 'settings-row');
  const labelWrap = el('div', 'settings-row-label');
  labelWrap.appendChild(el('label', 'form-label', 'Plugin signature policy'));
  labelWrap.appendChild(el('div', 'settings-row-desc', POLICY_HINT[policy] ?? POLICY_HINT.allow));
  row.appendChild(labelWrap);
  const controlWrap = el('div', 'settings-row-control');
  const select = document.createElement('select');
  select.className = 'plugin-policy-select';
  for (const value of ['allow', 'warn', 'require']) {
    const opt = document.createElement('option');
    opt.value = value;
    opt.textContent = POLICY_LABEL[value];
    opt.selected = value === policy;
    select.appendChild(opt);
  }
  select.addEventListener('change', () => {
    void (async () => {
      try {
        await api.settings.save({ pluginSignaturePolicy: select.value });
        showToast(`Plugin signature policy set to ${POLICY_LABEL[select.value]}`);
        refresh();
      } catch (e: any) {
        showToast(`Couldn't save: ${e?.message ?? e}`);
      }
    })();
  });
  controlWrap.appendChild(select);
  row.appendChild(controlWrap);
  content.appendChild(row);
}

export function renderPluginsSettings(content: HTMLElement) {
  content.innerHTML = '';
  content.classList.add('network-panel');
  const loading = el('div', 'settings-empty', 'Loading plugins…');
  content.appendChild(loading);

  const refresh = () => renderPluginsSettings(content);
  Promise.all([api.plugins.list(), api.plugins.audit(), api.settings.get()])
    .then(([plugins, audit, settings]) => {
      const policy = typeof settings.pluginSignaturePolicy === 'string' ? settings.pluginSignaturePolicy : 'allow';
      content.innerHTML = '';
      content.appendChild(el('div', 'settings-group-header', 'Plugins'));
      content.appendChild(el('div', 'settings-hint',
        'A plugin does nothing until you approve it. Only this console can grant permissions, and a plugin whose files change is stopped until you review it again.'));
      renderPolicyControl(content, policy, refresh);
      const install = el('div', 'plugin-install-row');
      const installBtn = el('button', 'btn btn-secondary plugin-install', 'Install plugin…');
      installBtn.addEventListener('click', () => pickPackage(async (file) => {
        try {
          const p = await api.plugins.install(file);
          showToast(`${p.name} installed. Review its permissions below before it can run.`);
          refresh();
        } catch (e: any) {
          showToast(`Couldn't install: ${e?.message ?? e}`);
        }
      }));
      install.append(installBtn, el('span', 'settings-hint', 'A plugin package is a .zip with a plugin.json. Only install plugins from people you trust: it stays switched off until you approve it.'));
      content.appendChild(install);
      if (!plugins.length) {
        content.appendChild(el('div', 'settings-empty', 'No plugins installed.'));
      }
      for (const p of plugins) content.appendChild(renderPlugin(p, refresh, policy));
      void renderPublishers(content, refresh);
      void renderSharedSecrets(content, refresh);
      content.appendChild(renderAudit(audit));
    })
    .catch((e) => {
      content.innerHTML = '';
      content.appendChild(el('div', 'settings-empty', `Couldn't load plugins: ${e?.message ?? e}`));
    });
}
