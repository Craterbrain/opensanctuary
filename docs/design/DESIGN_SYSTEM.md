# OpenSanctuary console design system

The console is a dense, dark control-room tool (1366x768 up to 4K). This system keeps it
that way and makes every surface use the same few recipes. Everything lives in
`web/style.css`: tokens in `:root`, components in the "DESIGN SYSTEM: FOUNDATIONS & SHARED
COMPONENTS" section right after the app shell. Before adding a class or an inline
`style=""`, look here first.

## Principles

1. **Quiet chrome, loud state.** Surfaces are neutral greys. Brand color is spent only on
   what the operator is acting on: the primary action, the selection, focus. Everything
   else is grey.
2. **One recipe per idea.** A selected row looks the same in the schedule, the library,
   Settings and the guide. A menu looks the same whether it drops from the menubar, the
   ribbon, a right-click or the search box.
3. **Theme through tokens, never literals.** All color comes from `--os-*` tokens. The five
   color themes only override `--os-brand-*`; anything written with a literal hex won't
   follow them. Text on a `--os-brand-primary` fill is always white (a unit test checks
   every theme).
4. **State colors don't follow the theme.** On air is tally red (`--os-live`), staged is
   tally green (`--os-staged`), success/warning/danger are fixed. They mean the same thing
   in every theme.
5. **Dense on purpose.** Controls are 22-30px, text 10.5-12.5px, sizes scale with
   `clamp()`. Don't pad things out.

## Tokens

| Group | Tokens | Notes |
|---|---|---|
| Surfaces | `--os-bg-void` < `--os-bg-studio` < `--os-bg-panel` < `--os-bg-elevated`; `--os-bg-sunken` (fields, wells), `--os-bg-overlay` (menus, popovers, toasts) | Raised = lighter. Fields are *sunken* so they never look like buttons. |
| Borders | `--os-border-subtle` (row dividers), `--os-border` (default), `--os-border-strong` (overlays, hovered fields, buttons) | |
| Text | `--os-text-primary`, `--os-text-secondary`, `--os-text-tertiary` (hints, placeholders), `--os-text-disabled`, `--os-text-on-accent` (#fff), `--os-accent-text` (= brand highlight, for brand-colored text/icons on dark) | Use `--os-accent-text`, not `--os-brand-primary`, for colored text: primary is a fill color and is too dark as text in some themes. |
| Brand (theme) | `--os-brand-primary` (fills), `--os-brand-amber` (highlight), `--os-brand-flame`, `--os-brand-bright`, `--os-brand-*-rgb` | Set at runtime by `applyColorTheme()` (`src/core/icon_sets.ts`). |
| Interaction | `--os-hover`, `--os-pressed` (translucent white), `--os-selected` (= primary), `--os-selected-soft`, `--os-selected-soft-border`, `--os-focus`, `--os-focus-ring` | Hover/pressed are overlays, so they work on any surface or fill. |
| State | `--os-live`, `--os-staged` (+ `-rgb`); `--os-success`, `--os-warning`, `--os-danger` (fills, + `-rgb`) and `--os-*-text` (text on dark); `--os-info-text` | |
| Spacing | `--os-space-1..8` = 2, 4, 6, 8, 12, 16, 20, 24px | |
| Radius | `--os-radius-xs` 2, `-sm` 4 (controls, rows, chips), `-md` 6 (cards, menus, tiles), `-lg` 8 (dialogs), `-pill` | |
| Type | `--os-fs-2xs` (badges, kbd) · `-xs` (eyebrows, metadata) · `-sm` (controls, rows: the default UI size) · `-md` (body copy, setting names) · `-lg` (dialog titles) · `-xl`, `-2xl` (headings); `--os-fw-*`, `--os-lh-tight`, `--os-lh`; `--os-font`, `--os-font-mono` | All `clamp()`ed. |
| Controls | `--os-control-h-sm` (22-24), `--os-control-h` (26-30), `--os-control-h-lg` (30-34) | |
| Elevation | `--os-shadow-sm` (hovered tile), `-md` (menus, popovers, toasts), `-lg` (dialogs) | |
| Motion | `--os-dur-fast` 90ms, `--os-dur` 140ms, `--os-dur-slow` 220ms; `--os-ease`, `--os-ease-out` | Color/opacity transitions only; no bouncing or lifting. |
| Layers | `--os-z-splitter` 25, `-menubar` 100, `-modal` 2000, `-dropdown` 3000, `-popover` 3500, `-modal-nested` 4000, `-confirm` 10000, `-toast` 99999 | Same values the dialog manager already stacks with. |

## Components

### Buttons
`.btn` is the secondary button (raised, bordered) and the default. Add one modifier:

| Class | Use for |
|---|---|
| `.btn-primary` | The one main action of a dialog/area (Save, Apply, Broadcast). Flat primary fill, white text. At most one per group. |
| `.btn` / `.btn-secondary` | Everything else that's a real action (Cancel in a group, Refresh, Browse...). |
| `.btn-ghost`, `.btn-back` | Low-emphasis actions: "Back" on the left of a dialog footer, "Cancel" in dense bars. |
| `.btn-danger` | Confirming something destructive (inside the confirm dialog). |
| `.btn-danger-ghost` | A destructive action sitting among normal ones (Stop Stream, Revoke, Remove). |
| `.btn-tag` | Chips: insert-a-tag, quick filters. |
| `.btn-sm` / `.btn-lg` | Sizes (default is md). `.btn-icon` makes it square; `.btn-block` full-width left-aligned. |
| `.header-btn` | 24px borderless icon button in panel headers. |
| `.header-link-btn` | Small accent action in a header bar ("Open Screen", "Add to Schedule"). |
| `.modal-close-btn` | Dialog close. |

Do: put icons in labels with `{icon:name}` or emoji that `emoji_icons.ts` maps. Don't: set
`background`/`color`/`padding` inline on a button, or invent a colored button (gold Genius,
teal yt-dlp...). Pick a variant.

### Form controls
Inputs, selects and textareas are styled globally (sunken, 1px border, primary focus ring).
Checkboxes, radios and ranges use `accent-color: --os-brand-primary`; file inputs get a
`.btn`-like chooser button.

| Class | Use for |
|---|---|
| `.form-field` | Label above a full-width control (dialogs). |
| `.form-row` | Inline label/control/button row; text inputs flex to fill. |
| `.form-check` (`.form-check--boxed`) | Checkbox/radio + label (boxed = an option row with a description). |
| `.form-label`, `.form-help` | Label text; help/hint text under a control. |
| `.settings-row` (+ `-label`, `-desc`, `-control`) | Settings-style "name + description on the left, control on the right". Also used by the theme editor. |

Don't: re-skin an input inline, give it a fixed pixel width inside a dialog (let
`.form-field` size it), or put labels in the placeholder only.

### Tabs and segmented controls
- **`.os-tabs` > `.os-tab`** (underline): switch what a whole surface shows. Library tabs
  (`.tab-btn`), Import dialog modes, Remote dialog panels (`.modal-tabs`).
- **`.os-segmented` > `button.active`** (filled pill in a sunken track): pick one of a few
  views, filters or modes. View-mode toggles, Background picker Images/CSS/Videos, image
  and style categories, video sub-tabs, Song/Scripture/Presentation, schedule view sizes,
  properties-panel modes. `.import-mode-tabs` without `.os-tabs` is a segmented control.

### Menus and popovers
Every dropdown, context menu and the search-mode menu share `.os-menu`: overlay surface,
strong border, `--os-shadow-md`, 4px inset padding, rows `--os-control-h-sm` tall with a
`--os-radius-sm` hover pill in solid primary. Dividers: `.dropdown-divider`. Destructive
items: `.context-menu-item-danger` (red text, red hover). Keyboard hints: `<kbd>` inside
`.dropdown-item`. Panels of controls that drop from a button (schedule view, editor
background) use `.os-popover`.

Don't color individual menu items to make them stand out; order and dividers do that.

### Lists and selection
- **Strong selection** (the item you're about to act on): solid `--os-selected` fill, white
  text, secondary text at 80% white. Schedule rows, library rows, category/version trees,
  Settings and guide navigation (`.os-nav-item`).
- **Soft selection** (a secondary "current" marker): `--os-selected-soft` with an inset
  2-3px bar. Child slides, highlighted search results.
- **Hover**: `--os-hover` overlay. Never change text color on hover in a list.
- **On air / staged**: inset bar or ring in `--os-live` / `--os-staged`.

### Cards and tiles
- `.os-card` (`.store-card`, `.network-card`, `.display-*` cards): elevated surface, 1px
  border, `--os-radius-md`, 12px padding. Title `.os-card-title`.
- `.os-tile` (library grid cards, background/video/theme picker cards): thumbnail +
  `.os-tile-caption`. Hover = stronger border + small shadow (no lifting). Selected =
  primary border ring; pickers add a check badge, library cards fill the caption.
- Thumbnail overlay labels: `.os-thumb-badge` (dark translucent, white text).

### Badges
`.os-badge` + `--accent` (brand highlight), `--neutral`, `--success`, `--warning`,
`--danger`, `--info`, `--live`. 18px tall, 2xs bold text. Language badges, network status,
device platform, CCLI numbers and "Genius" source tags all use these.

### Section headers
`.os-eyebrow`: xs, bold, uppercase, 0.06em tracking, secondary color. Use it to title a
group of controls or a column (Settings groups, dialog columns, sidebars, the preview pane).
Not for dialog titles (those are `.modal-header`) or for emphasis in running text.

### Dialogs
Shell: `.modal-backdrop` > `.modal-dialog[.modal-dialog--xs|sm|md|lg|xl]` >
`.modal-header` (title `<span>` + `.modal-close-btn`), optional `.modal-tabs`,
`.modal-body`, `.modal-footer`.
- Body layouts: `.modal-body--stack` (column with gaps, fixed height),
  `.modal-body--scroll-stack`, `.modal-body--columns`, `.modal-body--flush` (sidebar layouts),
  `.modal-body--center`.
- Footer: a ghost "Back"/secondary action on the left; `.modal-footer-actions` on the right
  (it's pushed right even when alone), primary action last. `.modal-footer-note` for a hint.
- Destructive dialogs: `.modal-header--danger` (red rule + red title) and `.btn-danger`.

### Callouts, empty states, toasts, banners
- `.os-callout` (`--info`, `--warning`, `--danger`): inline note inside a dialog/panel,
  3px left rule. Use instead of hand-tinted boxes.
- `.os-code-box`: read-only monospace value (URLs, IDs).
- `.ui-empty-state` (`.empty-state-icon`, `-title`, `-desc`, `.btn-empty-action`,
  `--compact`): every "nothing here yet" (schedule, decks, library, pickers).
- Toasts (`showToast`): overlay surface with a status-colored left edge.
- `.os-banner`: a persistent bar across the top (the host-auth prompt).

### Toolbars
Ribbon buttons (`.ribbon-btn`, `.studio-btn`, `.studio-btn-toggle`, `.ribbon-insert-btn`,
`.ribbon-shape-btn-sm`) are borderless until hovered; toggles fill with primary when on.
Groups are separated by `.os-toolbar-divider` / `.studio-divider`.

### Focus and scrollbars
Every interactive element shows a 2px `--os-focus` outline on `:focus-visible` (not on
mouse click); fields show `--os-focus-ring`. The slide canvases and `contenteditable` text
are excluded. Scrollbars are thin, transparent-track, translucent thumbs.

## Icons on colored fills
An icon on a primary/danger fill must be white. The block at the end of `style.css`
whitens `.ui-icon` / `img[data-icon]` inside every recipe that fills with primary or
danger. When you add a new filled state, add its selector there.

## Known gaps
Two behavior bugs turned up while building this system and are still open (behavior, not
styling, so fixing them means touching the modal/dialog code, not `style.css`):
- Settings → Paired Devices' "Pair TV App" and "Install via ADB" buttons
  (`settings_dialog.ts`) open the remote modal by calling
  `remoteModal.classList.add('active')`, but the dialog manager actually shows/hides modals
  via `style.display` and no `#remote-modal.active` rule exists — the click does nothing
  visible. Fix by routing through the dialog manager's real open function instead.
- No focus trap in open dialogs: `Tab` can move focus out of a modal into the page behind
  it. Every other modal behavior (backdrop click, `Esc`, stacking) works; this one doesn't.

## Out of scope
The slide canvases (`.canvas-16-9`, the editor canvas, thumbnails), `live.html`,
`stage.html`, `remote.html` and `pairing.html` are output or separate surfaces and are not
styled by this system. Per-section tag colors (verse/chorus/bridge badges and the editor
filmstrip tag chips) are content semantics and keep their own palette.
