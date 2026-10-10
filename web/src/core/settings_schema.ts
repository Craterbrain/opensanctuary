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

/**
 * 'panel' rows belong to a category rendered by its own bespoke panel (display, theme).
 * They are never shown in that category's own view -- only as search results, where
 * they open the panel -- so the Settings search can find what those panels hold.
 */
export type SettingControl = 'text' | 'password' | 'select' | 'readonly' | 'action' | 'panel';

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
  { id: 'theme', label: 'Theme', icon: '🎨' },
  { id: 'network', label: 'Network', icon: '🌐' },
  { id: 'paired-devices', label: 'Paired Devices', icon: '📺' },
  { id: 'alerts', label: 'Alerts & Nursery', icon: '🔔' },
  { id: 'integrations', label: 'Integrations', icon: '🔌' },
  { id: 'storage', label: 'Storage', icon: '📁' },
  { id: 'about', label: 'About', icon: 'ℹ' },
];

// The "display" and "theme" categories are rendered by the bespoke renderDisplaySettings()
// / renderThemeSettings() in settings_dialog.ts (live monitor detection, per-output
// Open/Close; icon-set and color-theme pickers) rather than the generic single-value row
// renderer below. Their SETTINGS_SCHEMA entries are 'panel' rows: search-only pointers
// that open the panel (see SettingControl).

export const SETTINGS_SCHEMA: SettingDef[] = [
  {
    key: 'themeLinked',
    label: 'Match Icons and Colors',
    description: 'Use one cohesive theme for both the icon set and the interface colors, or turn this off to choose them independently.',
    category: 'theme',
    control: 'panel',
    actionLabel: 'Open Theme',
  },
  {
    key: 'iconSet',
    label: 'Icon Set',
    description: 'Theme icons: the color of the toolbar and menu icons (orange, slate, gold, white or crimson).',
    category: 'theme',
    control: 'panel',
    actionLabel: 'Open Theme',
  },
  {
    key: 'colorTheme',
    label: 'Color Theme',
    description: 'Accent colors used across the interface (selection, buttons, highlights). Follows the icon set unless set independently.',
    category: 'theme',
    control: 'panel',
    actionLabel: 'Open Theme',
  },
  {
    key: 'displayMonitors',
    label: 'Detected Monitors',
    description: 'The screens connected to this computer, with their resolution and which one is primary. Refresh to detect newly connected displays.',
    category: 'display',
    control: 'panel',
    actionLabel: 'Open Display',
  },
  {
    key: 'displayOutputsPanel',
    label: 'Display Outputs',
    description: 'Open or close the live output and stage display windows on each monitor, and add or remove outputs.',
    category: 'display',
    control: 'panel',
    actionLabel: 'Open Display',
  },
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
    key: 'ccliReport',
    label: 'CCLI Usage Report',
    description: 'Export a CSV of songs used in services for CCLI reporting. See docs/CCLI_REPORTING.md.',
    category: 'integrations',
    control: 'action',
    actionLabel: 'Open CCLI Report…',
  },
  {
    key: 'ccliReportingDueDate',
    label: 'CCLI Reporting Due Date',
    description: 'When your CCLI license requires its next usage report. A reminder appears starting 30 days before this date. Leave blank to disable the reminder.',
    category: 'integrations',
    control: 'text',
    placeholder: 'YYYY-MM-DD',
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
    key: 'publicHttpsUrl',
    label: 'Public HTTPS URL',
    description: 'A trusted public domain (e.g. a Caddy reverse proxy with a real Let\'s Encrypt cert — see docs/TUNNELS.md) to prefer over the self-signed certificate everywhere a URL or QR code is generated for a device off this machine.',
    category: 'network',
    control: 'text',
    placeholder: 'https://connect.yourchurch.org',
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
    key: 'dataDirectory',
    label: 'Data Directory',
    description: 'Where your library database, Bibles, songs, and media cache live. See docs/paths.md for details.',
    category: 'storage',
    control: 'readonly',
  },
  {
    key: 'moveDataDirectory',
    label: 'Move Data Directory',
    description: 'Relocate your data directory to another folder or drive with SHA-256 integrity verification.',
    category: 'storage',
    control: 'action',
    actionLabel: 'Move Data Directory…',
  },
  {
    key: 'revealDataDirectory',
    label: 'Reveal Data Directory',
    description: 'Open the data directory in your file manager (Explorer/Finder) on this machine.',
    category: 'storage',
    control: 'action',
    actionLabel: 'Reveal in File Manager',
  },
  {
    key: 'biblesDirectory',
    label: 'Bibles Directory',
    description: 'Where installed Bible translation files (.db) are read from. Leave blank to use the default alongside the data directory. Requires a restart to take effect.',
    category: 'storage',
    control: 'text',
    placeholder: 'Default: alongside the data directory',
  },
  {
    key: 'songsDirectory',
    label: 'Songs Directory',
    description: 'Where song databases (public-domain and copyrighted) are read from. Leave blank to use the default alongside the data directory. Requires a restart to take effect.',
    category: 'storage',
    control: 'text',
    placeholder: 'Default: alongside the data directory',
  },
  {
    key: 'mediaCacheDirectory',
    label: 'Media Cache Directory',
    description: 'Where downloaded/searched background images and video are stored. Leave blank to use the default alongside the data directory. Requires a restart to take effect.',
    category: 'storage',
    control: 'text',
    placeholder: 'Default: alongside the data directory',
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
  {
    key: 'rerunFirstTimeSetup',
    label: 'First-Time Setup',
    description: 'Re-run the welcome flow (data directory, Bible/song content, network info) -- useful after moving to a new machine.',
    category: 'about',
    control: 'action',
    actionLabel: 'Re-run First-Time Setup',
  },
  {
    key: 'checkForUpdates',
    label: 'Check for Updates',
    description: 'OpenSanctuary checks for a new release periodically in the background. Click to check right now -- if one\'s available, you\'ll be offered to download and install it immediately.',
    category: 'about',
    control: 'action',
    actionLabel: 'Check for Updates…',
  },
  {
    key: 'installUpdateFromFile',
    label: 'Install Update from File',
    description: 'For platforms the automated download doesn\'t cover yet, or to install without waiting for a check: download a release yourself from GitHub and pick it here -- verified against a checksum file if you downloaded one alongside it, then handed to your system\'s own installer.',
    category: 'about',
    control: 'action',
    actionLabel: 'Install Update from File…',
  },
];
