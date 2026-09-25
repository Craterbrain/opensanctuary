/**
 * Settings manifest driving the searchable, categorized Settings dialog
 * (Plasma System Settings style: a category sidebar + search box, one flat list of
 * settings underneath). Adding a setting is a schema entry here, not a hand-edited
 * modal section — see renderSettingsSidebar/renderSettingsContent in app_core.ts for
 * the generic renderer this feeds.
 *
 * A setting listed here is expected to actually do something. Five settings used to
 * exist in the old 3-tab Options modal (outputMonitor, transitionEffect,
 * alertPosition, alertFontSize, defaultTheme) that were saved to the database and
 * never read by anything. alertPosition/alertFontSize/outputMonitor are now wired
 * for real; transitionEffect had no rendering path to wire to at all
 * (ShowState.transition/SetTransition exists server-side but nothing ever sends or
 * renders it) and was removed rather than left pointing at nothing. defaultTheme was
 * replaced by a real per-category default (Themes tab > right-click a theme > Set as
 * Default — see src/core/models.rs Theme::is_default/category) instead of one flat
 * setting shared across songs/scriptures/presentations.
 */

export type SettingControl = 'text' | 'password' | 'select' | 'readonly' | 'action';

export interface SettingOption {
  value: string;
  label: string;
}

export interface SettingDef {
  key: string;
  label: string;
  description: string;
  category: string;
  control: SettingControl;
  options?: SettingOption[];
  placeholder?: string;
  /** For control: 'action' — the button's label. */
  actionLabel?: string;
}

export interface SettingCategoryDef {
  id: string;
  label: string;
  icon: string;
}

export const SETTINGS_CATEGORIES: SettingCategoryDef[] = [
  { id: 'general', label: 'General', icon: '⚙' },
  { id: 'live-output', label: 'Live Output', icon: '🖥' },
  { id: 'display', label: 'Display', icon: '🖵' },
  { id: 'network', label: 'Network', icon: '🌐' },
  { id: 'paired-devices', label: 'Paired Devices', icon: '📺' },
  { id: 'alerts', label: 'Alerts & Nursery', icon: '🔔' },
  { id: 'integrations', label: 'Integrations', icon: '🔌' },
  { id: 'about', label: 'About', icon: 'ℹ' },
];

// The "display" category has no SETTINGS_SCHEMA entries — it's rendered by the bespoke
// renderDisplaySettings() in settings_dialog.ts (live monitor detection, per-output
// Open/Close, etc.) rather than the generic single-value row renderer below, so it's
// intentionally excluded from search matching too.

export const SETTINGS_SCHEMA: SettingDef[] = [
  {
    key: 'churchName',
    label: 'Church / Sanctuary Name',
    description: 'Shown on the live output footer and advertised to Roku/Android TV clients discovering this console on the network.',
    category: 'general',
    control: 'text',
    placeholder: 'Your Church Name',
  },
  {
    key: 'defaultBibleVersion',
    label: 'Default Bible Translation',
    description: 'Used when adding new scripture items, and as the fallback translation wherever one isn\'t otherwise specified.',
    category: 'general',
    control: 'select',
    options: [], // populated at render time from installed bibles
  },
  {
    key: 'aspectRatio',
    label: 'Editor Preview Aspect Ratio',
    description: 'Aspect ratio used for the operator console\'s own preview canvas.',
    category: 'live-output',
    control: 'select',
    options: [
      { value: '16:9', label: '16:9 Widescreen (Standard HD/4K)' },
      { value: '4:3', label: '4:3 Standard Definition' },
    ],
  },
  {
    key: 'openLiveOutputWindow',
    label: 'Live Output Window',
    description: 'Open Live Output in a separate browser window you can drag to a second display and fullscreen — the browser-based equivalent of a dedicated projector output.',
    category: 'live-output',
    control: 'action',
    actionLabel: 'Open Live Output Window',
  },
  {
    key: 'alertPosition',
    label: 'Alert Banner Position',
    description: 'Where the nursery/message alert banner appears on the live output screen.',
    category: 'alerts',
    control: 'select',
    options: [
      { value: 'bottom', label: 'Bottom of Screen' },
      { value: 'top', label: 'Top of Screen' },
    ],
  },
  {
    key: 'alertFontSize',
    label: 'Alert Banner Text Size',
    description: 'Relative text size for the alert banner (scales with screen size).',
    category: 'alerts',
    control: 'select',
    options: [
      { value: '28', label: 'Medium' },
      { value: '36', label: 'Large' },
      { value: '48', label: 'Extra Large' },
    ],
  },
  {
    key: 'pexelsApiKey',
    label: 'Pexels API Key',
    description: 'Enables searching Pexels for royalty-free background images and video from the Media Library.',
    category: 'integrations',
    control: 'password',
    placeholder: 'Paste your Pexels API key...',
  },
  {
    key: 'pixabayApiKey',
    label: 'Pixabay API Key',
    description: 'Enables searching Pixabay for royalty-free background images and video from the Media Library.',
    category: 'integrations',
    control: 'password',
    placeholder: 'Paste your Pixabay API key...',
  },
  {
    key: 'networkHostname',
    label: 'Host Name (DHCP Option 12 & mDNS)',
    description: 'The network identity advertised to DHCP servers via Option 12 and mDNS (.local).',
    category: 'network',
    control: 'text',
    placeholder: 'opensanctuary',
  },
  {
    key: 'networkPort',
    label: 'HTTP & Remote Control Port',
    description: 'Port used for the operator console, live stream, and mobile remote control.',
    category: 'network',
    control: 'text',
    placeholder: '8080',
  },
  {
    key: 'networkDedicatedMac',
    label: 'Dedicated MAC & IP Address',
    description: 'Provisions an isolated virtual network adapter (macvlan/ipvlan/VMAdapter) to obtain a dedicated DHCP lease.',
    category: 'network',
    control: 'readonly',
  },
  {
    key: 'networkBroadcastAll',
    label: 'Multi-Interface Broadcast Mode',
    description: 'Broadcasts DHCP Option 12 and mDNS across all active network interfaces simultaneously.',
    category: 'network',
    control: 'readonly',
  },
  {
    key: 'securityCspMode',
    label: 'Content Security Policy (CSP)',
    description: 'Restricts script execution, cross-site framing, and object embedding. Balanced is recommended for local sanctuary production.',
    category: 'network',
    control: 'select',
    options: [
      { value: 'balanced', label: 'Balanced Protection (Recommended)' },
      { value: 'strict', label: 'Strict Hardening (HTTPS & Self Only)' },
      { value: 'disabled', label: 'Disabled (No CSP)' },
    ],
  },
  {
    key: 'securityCorsMode',
    label: 'CORS Network Boundary Policy',
    description: 'Controls which network origins can issue API requests to this console. Permissive allows all LAN display devices (Android TV, Roku).',
    category: 'network',
    control: 'select',
    options: [
      { value: 'permissive', label: 'Permissive (LAN Displays & Companions)' },
      { value: 'restricted', label: 'Restricted (Local Subnet & Sanctuary Only)' },
    ],
  },
  {
    key: 'securityFrameOptions',
    label: 'Frame Embedding (X-Frame-Options)',
    description: 'Controls whether the console or live/stage views can be embedded in iframes.',
    category: 'network',
    control: 'select',
    options: [
      { value: 'sameorigin', label: 'Same-Origin Only (Recommended)' },
      { value: 'deny', label: 'Deny All Framing' },
      { value: 'disabled', label: 'Allow All (OBS Studio Browser Sources)' },
    ],
  },
  {
    key: 'serverInstanceId',
    label: 'Console Instance ID',
    description: 'This console\'s stable identity, advertised via mDNS and used by permanent Roku/Android TV displays to detect if they\'ve been pointed at a different console. See docs/CLIENT_PAIRING.md.',
    category: 'about',
    control: 'readonly',
  },
  {
    key: 'appVersion',
    label: 'Version',
    description: '',
    category: 'about',
    control: 'readonly',
  },
];
