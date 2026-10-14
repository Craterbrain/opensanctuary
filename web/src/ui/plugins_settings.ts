/**
 * Settings > Plugins: review what each installed plugin asks for, approve or deny it,
 * revoke it later, and see the audit trail (docs/PLUGIN_PERMISSIONS.md).
 *
 * Everything below comes from a plugin's own manifest, which is untrusted input: it is
 * only ever inserted with textContent, never innerHTML.
 */
import { api, type PluginAuditEntry, type PluginInfo, type PluginPermissionInfo } from "../core/api_client.ts";
import { showToast } from "../core/ui_utils.ts";

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
  return parts.join(' · ');
}

function isNative(p: PluginInfo): boolean {
  return p.kind === 'native' || p.kind === 'legacy-native';
}

function renderPlugin(plugin: PluginInfo, refresh: () => void): HTMLElement {
  const card = el('div', 'network-card plugin-card');
  card.dataset.pluginId = plugin.id;

  const head = el('div', 'plugin-card-head');
  head.appendChild(el('strong', 'plugin-name', plugin.name));
  if (plugin.version) head.appendChild(el('span', 'plugin-version', ` v${plugin.version}`));
  head.appendChild(el('span', `os-badge plugin-state plugin-state--${plugin.state}`, STATE_LABEL[plugin.state] ?? plugin.state));
  head.appendChild(el('span', 'os-badge os-badge--neutral', isNative(plugin) ? 'Backend library' : plugin.kind === 'sandbox' ? 'Sandboxed page' : 'Console script'));
  card.appendChild(head);
  card.appendChild(el('div', 'settings-hint', `id: ${plugin.id} · digest ${plugin.digest.slice(0, 12)}… · ${plugin.signed ? `signed (publisher key ${plugin.signer_id})` : 'unsigned'}`));
  if (plugin.signer_changed) {
    card.appendChild(el('div', 'plugin-warning', 'The publisher signature is different from the one you approved before (or it used to be signed and now isn\'t). Check that this update is from who you expect.'));
  }

  if (plugin.error) {
    card.appendChild(el('div', 'plugin-error', `Can't be approved: ${plugin.error}`));
  }
  if (plugin.state === 'changed') {
    card.appendChild(el('div', 'plugin-warning', 'This plugin\'s files changed since you approved it, so it is not running. Review the permissions again to re-approve.'));
  }
  if (plugin.kind === 'sandbox' && plugin.state !== 'active') {
    card.appendChild(el('div', 'settings-hint',
      'Runs in an isolated frame with no access to this console. It can only do what is checked below, and can reach the network only at the hosts listed (if any).'));
  }
  const critical = plugin.requested.some(p => p.id === 'native.code' || p.id === 'ui.dom');
  if (critical && plugin.state !== 'active') {
    card.appendChild(el('div', 'plugin-warning',
      isNative(plugin)
        ? 'This is native code that runs with full access to this computer. The list below is what the plugin says it will do — OpenSanctuary can only enforce part of it.'
        : 'This script runs inside the console page itself, with the same access as you. The list below is what it says it will do.'));
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
  if (plugin.state !== 'active' && !plugin.error) {
    const approve = el('button', 'btn btn-primary', plugin.state === 'changed' ? 'Approve changes' : 'Approve');
    approve.addEventListener('click', () => {
      const granted = checks.filter(c => c.box.checked).map(c => c.id);
      void act(() => api.plugins.approve(plugin.id, plugin.digest, granted), `${plugin.name} approved`);
    });
    actions.appendChild(approve);
    if (plugin.state === 'pending' || plugin.state === 'changed') {
      const deny = el('button', 'btn btn-secondary', 'Deny');
      deny.addEventListener('click', () => void act(() => api.plugins.deny(plugin.id), `${plugin.name} denied`));
      actions.appendChild(deny);
    }
  }
  if (plugin.state === 'active') {
    const revoke = el('button', 'btn btn-secondary', 'Revoke');
    revoke.addEventListener('click', () => void act(() => api.plugins.revoke(plugin.id), `${plugin.name} revoked`));
    actions.appendChild(revoke);
    if (plugin.restart_required) {
      actions.appendChild(el('span', 'settings-hint', 'Takes effect after OpenSanctuary restarts.'));
    }
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

export function renderPluginsSettings(content: HTMLElement) {
  content.innerHTML = '';
  content.classList.add('network-panel');
  const loading = el('div', 'settings-empty', 'Loading plugins…');
  content.appendChild(loading);

  const refresh = () => renderPluginsSettings(content);
  Promise.all([api.plugins.list(), api.plugins.audit()])
    .then(([plugins, audit]) => {
      content.innerHTML = '';
      content.appendChild(el('div', 'settings-group-header', 'Plugins'));
      content.appendChild(el('div', 'settings-hint',
        'A plugin does nothing until you approve it. Only this console can grant permissions, and a plugin whose files change is stopped until you review it again.'));
      if (!plugins.length) {
        content.appendChild(el('div', 'settings-empty', 'No plugins installed.'));
      }
      for (const p of plugins) content.appendChild(renderPlugin(p, refresh));
      content.appendChild(renderAudit(audit));
    })
    .catch((e) => {
      content.innerHTML = '';
      content.appendChild(el('div', 'settings-empty', `Couldn't load plugins: ${e?.message ?? e}`));
    });
}
