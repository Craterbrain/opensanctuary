# Console UI audit (before the design-system pass)

Scope: every operator-facing surface of the console (`web/index.html` + `web/style.css` +
the TS that renders into it). Output surfaces (`.canvas-16-9`, `live.html`, `stage.html`)
and the remote/pairing pages were out of scope. Screenshots: 72 captures at 1366x768
(orange theme) plus spot checks in slate and white, taken with a Playwright driver that
mocks `/api/*` and the engine WebSocket with a realistic library and schedule.

The short version: the console has a good dark, dense base and a working token layer for
the brand colors, but each surface was built on its own. There are three different menu
surfaces, four selection treatments, five tab styles, three input looks, primary buttons in
five color schemes, and 328 inline `style=""` blocks in index.html (plus hundreds of style strings in TS) that hard-code colors the theme can't
reach. The result looks fine in the default orange theme and visibly breaks in the other four.

## 1. Color and theming

| # | Finding | Where |
|---|---------|-------|
| C1 | **Hard-coded cyan `#00e5ff` (~45 uses) ignores the theme**. It's used as "staged", "secondary", "info" and plain decoration. In every theme except slate it clashes with the brand. | `.schedule-item.live-item`, `.slide-card.active-staged`, `.schedule-child-item.selected-child-slide`, `.dropdown-item.active`, `.header-link-btn`, `.os-toast-info`, `.schedule-dropzone`, `.canvas-dual-*`; inline in import modal (sources line, GitHub/yt-dlp card titles, checkbox rows), remote modal URL boxes, `#ctx-slide-duplicate`, `#ctx-lib-add-dual`, library prompt rows, Genius CCLI badges |
| C2 | **Other off-theme accent hues**: gold `#ffd700` (Genius), blue `#64b5f6`/`#2979ff` (online images, "Edit Arrangement"), teal gradient `#00e5ff→#00897b` (yt-dlp download buttons), sky `#38bdf8` (bulk badge, "Blank Slide"), Tailwind blues `#3b82f6`/`#93c5fd` (filmstrip, props focus). | library_panel.ts, index.html import/bg-picker/schedule ctx menu, style.css `.bulk-slide-badge`, `.fs-popover-item.selected`, `.editor-properties-panel :focus` |
| C3 | **Section headers in the schedule are always orange** (`#ffa726` border/text inline in TS), even in slate/white/crimson. | `renderSchedule()` header rows in app_core.ts |
| C4 | **Undefined tokens**: `--os-bg-surface`, `--os-bg-base`, `--os-accent`, `--text-muted`, `--os-bg-card` are referenced 39 times but never defined, so hover states silently vanish (Settings nav hover, schedule-view preset buttons) or fall back to literal colors. | style.css, index.html (arrangement, CCLI, articles, web modal), settings_dialog.ts |
| C5 | **Raw neutrals instead of tokens**: 107 distinct hex colors in style.css alone (`#1e1e24`, `#18181b`, `#27272a`, `#3f3f46`, `#343642`, `#383a46`, `#121214`, `#202024`, `#151515`...). The studio editor uses a separate zinc palette from the rest of the app. | menus, `.ribbon-popover`, `.fs-popover-menu`, `.editor-*` panels, bulk overlay |
| C6 | **Live vs. staged state colors are muddled**: live uses `#FF3D00` (nearly the orange brand primary), staged uses cyan, the live schedule row uses cyan, the selected child slide uses cyan. Nothing says "on air" consistently. | `.slide-card.active`, `.slide-card.active-staged`, `.schedule-item.live-item`, `.schedule-child-item.*` |
| C7 | **Unreadable text on selection**: the selected schedule row turns orange but keeps its secondary line in a dim/orange color ("Bethel Music • CCLI" is orange-on-orange). Library rows force `#fff` titles inline, so hover/selection can't restyle them. | schedule item `.notes`, catalog `<td style="color:#fff">` |
| C8 | **Icons vanish on custom-colored primary buttons**: remote modal's active tab is an inline orange fill without the "white icon" rule, so its icon disappears. | `#tab-remote-control` etc. |

## 2. Surfaces, elevation, borders

| # | Finding | Where |
|---|---------|-------|
| S1 | **Three menu surfaces**: menubar/ribbon dropdowns `#1e1e24` + `#484856` border + 5px radius; context menus `--os-bg-panel` + `--os-border` + 6px; editor popovers `#18181b` + `#3f3f46` + 6-8px. Shadows differ on each (`0 10px 28px .85`, `0 12px 32px .85`, `0 10px 25px -5px .7`, `0 20px 25px -5px .6`). | `.menu-dropdown-content`, `.ribbon-dropdown-menu`, `.context-menu`, `.search-mode-menu`, `.schedule-view-menu-dropdown`, `.fs-popover-menu`, `.ribbon-popover` |
| S2 | **Cards have no consistent recipe**: 1px vs 2px borders, radius 4/6/8/12, backgrounds panel/elevated/`rgba(255,255,255,.025)`/`#181f25`/`#141414`. | `.store-card`, `.network-card`, `.display-monitor-card`, `.catalog-grid-card` (1px/4px) vs picker cards (2px/6px), arrangement columns (inline), remote URL boxes (inline) |
| S3 | **38 distinct box-shadows**, 16 distinct radii in style.css; modals use clamp radius, menus 5-6px, cards 4-8px, pills 9999/14/12px. | style.css |
| S4 | **Splitters glow in a hard-coded orange** `rgba(255,145,0,.6)` in every theme; drop indicators and drag handles too. | `.splitter-*`, `.drop-above/below`, `.dragover-active`, `.schedule-bottom-drop-zone` |

## 3. Selection, hover, focus

| # | Finding | Where |
|---|---------|-------|
| H1 | **Four selection treatments** for the same idea ("this row is selected"): solid primary (schedule row, settings nav), primary→flame gradient (library row, category tree, import tabs, type pills, bg-picker tabs), tinted primary + amber left bar (bible tree), tinted cyan + cyan bar (child slides). | schedule, library, settings, scripture tree |
| H2 | **Menu hover**: solid primary (menubar), primary→flame gradient (context menu, search-mode menu), `#27272a` (editor popovers). "Active" menu item is cyan. | dropdowns |
| H3 | **Hover feedback varies**: brightness filter (section headers), translateY(-2px) lift (grid cards, picker cards), border-amber (buttons, inputs), background change (rows), scale(1.2) (drag handles), translateX (back button). Inputs change background *and* border on hover. | everywhere |
| H4 | **Focus rings**: inputs get a glow ring, buttons/tabs/menu items/cards get none (keyboard users can't see focus on most controls), drag handles have their own amber outline. | global |

## 4. Buttons

| # | Finding | Where |
|---|---------|-------|
| B1 | **Primary buttons come in five color schemes**: primary→flame gradient (`.btn-primary`), flat amber with black text (Apply Arrangement, Export CSV), teal gradient (yt-dlp download), gold gradient (Genius import), blue gradient (online image import). Some re-declare the same gradient inline. | index.html, library_panel.ts |
| B2 | **Heights/padding differ per call site**: 22, 24, 26, 28, 30, 32px; padding overridden inline on ~30 buttons (`padding: 4px 10px; font-size: 11px`, `padding: 7px 18px; font-weight: 700`...). | bulk overlay, import modal, remote modal, video picker, studio footer |
| B3 | **Secondary/ghost/danger variants are ad hoc**: `.btn-secondary` is referenced but never defined; "Stop Stream" fakes danger with inline `color:#ff8888`; `.btn-danger` uses `!important` on everything; "Paste" buttons use an inline blue-grey `#263238` palette. | web modal, remote modal, import/video modals |
| B4 | **Icon buttons**: `.header-btn` (22x24), `.studio-btn-toggle` (24x24), `.ribbon-shape-btn-sm` (22x22), `.view-toggle-btn` (no fixed size), `.modal-close-btn` (no hit area). The schedule header repeats a 300-character inline style on each of its four icon buttons. | panel headers, studio ribbon, modals |
| B5 | **"Link" buttons** (`.header-link-btn`) are cyan text on an orange tint. | Live header "Open Screen", library preview "Add to Schedule", "Compare" |

## 5. Form controls

| # | Finding | Where |
|---|---------|-------|
| F1 | **Inputs look like buttons**: both use `--os-bg-elevated` + `--os-border`, so a text field and a secondary button are indistinguishable at a glance. | global input rules vs `.btn` |
| F2 | **Three input skins**: global (elevated, 6px 10px, 11.5px), properties panel (`#27272a`/`#3f3f46`, 3px 6px, blue focus), bulk select (`#27272a`, 26px). The API-bible language filter is inline `#1a2228`/`#37474f`. | style.css, index.html import modal |
| F3 | **Inputs in simple dialogs don't fill the dialog** (Save, Web, Alert): a 200px field floating in a 600px dialog. | save-modal, web-modal, alert-modal |
| F4 | **Labels**: `.form-label` is 11px/600 secondary, but used inconsistently: some with trailing colons, some inline-overridden to 12px, Settings rows use it as the setting's *name* (so the most important text in a row is the dimmest). Studio notes pane labels render at browser default size and overflow the window. | settings rows, remote ADB form, notes pane |
| F5 | **Native controls unstyled**: checkboxes/radios/range sliders/color inputs/file inputs use browser defaults (blue checkboxes, white "Choose File" buttons, a white bar for color inputs). Some checkboxes are `transform: scale(1.15)`. | import local tab, theme editor, yt-dlp, settings |
| F6 | **Studio properties panel buttons are completely unstyled** (white native buttons: "solid gradient animated image video", preset names) because they were written for Tailwind classes that aren't loaded. | properties_panel.ts in the slide editor |

## 6. Tabs and segmented controls

| # | Finding | Where |
|---|---------|-------|
| T1 | **Five tab styles**: folder tabs (library), gradient pill buttons (import modes, image categories, video sub-tabs), header-centered pill buttons (Background picker), inline-styled 6px-top-radius buttons (remote modal), left-border nav (articles, settings). | bottom-tier, import-modal, media-image-picker-modal, remote-modal |
| T2 | **Same class, two meanings**: `.import-tab-btn` is used both for page tabs (import modes) and for filter chips (image categories), so they can't be styled differently. | import-modal, bg picker |
| T3 | **Segmented toggles**: `.view-mode-toggle` (`#181818` track), `.sched-view-btn` (amber fill with black text), studio type pills (rounded 14px gradient) - three takes on "pick one of N". | panels, schedule view menu, studio sidebar |

## 7. Lists, tables, cards

| # | Finding | Where |
|---|---------|-------|
| L1 | Row heights and paddings differ per list (schedule 5px 8px, catalog clamp, category 4px 12px, bible tree 4px 10px, settings nav 8px 10px, articles nav 10px 14px inline, freeshow list 6px 10px). | lists |
| L2 | Row dividers: `rgba(255,255,255,.02/.03/.04/.08)` and `--os-border` all used. | lists |
| L3 | Grid cards: catalog cards (1px, 4px radius, centered title) vs picker cards (2px, 6px radius, left title, check badge) vs theme picker cards. Picker cards stretch to the full grid height when there are few items (no `grid-auto-rows`). | `.catalog-grid-card`, `.media-image-card`, `.media-video-card`, `.theme-picker-card` |
| L4 | Badges/pills: `.bible-lang-badge`, `.network-badge--*`, `.grid-card-badge` (gold text), `.bulk-slide-badge` (sky, 9999px), Genius/CCLI badges (inline), device platform tags (inline), version pill (inline) - six recipes. | various |

## 8. Typography

| # | Finding | Where |
|---|---------|-------|
| Y1 | 32 distinct font sizes in style.css (9, 9.5, 10, 10.5, 11, 11.5, 12, 12.5, 13, 13.5, 14, 15, 16, 18...) plus 22 more in TS. | global |
| Y2 | **Section headers ("eyebrows")** are uppercase-700 in six slightly different sizes/colors/letter-spacings, and amber in the schedule-view menu. | settings groups, arrangement/CCLI columns, articles nav, filmstrip, props header, schedule view menu |
| Y3 | Modal titles are prefixed with "OpenSanctuary -" in some dialogs and not others; title weight/size is consistent (good). | options, shortcuts |

## 9. Modals

| # | Finding | Where |
|---|---------|-------|
| M1 | Header/footer chrome is shared (good), but bodies override padding inline in 11 of 18 dialogs (12/16/18/20/24/28px). | index.html |
| M2 | Footer layout: most dialogs put "Back" left and actions right; the theme editor puts its actions on the left; CCLI report and Articles override the footer inline. | theme-editor-modal, ccli-report-modal, schedule-articles-modal |
| M3 | Danger header is a hard-coded red gradient. | confirm-modal |
| M4 | The remote modal's tab strip and panels are ~40 inline declarations (fixed 480px width, `#141414` URL boxes, orange-tinted info boxes). | remote-modal |
| M5 | Arrangement, CCLI and Articles dialogs build their two-/three-column layouts entirely inline with undefined tokens (C4). | arrangement-modal, ccli-report-modal, schedule-articles-modal |
| M6 | Close button has no hover background/hit area, unlike every other icon button. | `.modal-close-btn` |

## 10. Feedback: empty states, toasts, banners

| # | Finding | Where |
|---|---------|-------|
| E1 | Four empty-state recipes: `.ui-empty-state` (exists, barely used), `.deck-empty-state`, the schedule's inline empty block, the library preview's `.asset-preview-empty`; the library table shows nothing at all when empty. | schedule, decks, library |
| E2 | Toast colors: success `#00e676`, info cyan, error `#ff5252` - none from the status tokens; the success message repeats the check mark (icon + "✓" in the text). | `.os-toast-*` |
| E3 | The "console isn't authenticated" banner is entirely inline: mustard `#3a2a00` background, a blue `#4a90d9` Connect button - unrelated to any theme. | `showHostAuthBanner()` in app_core.ts |
| E4 | Info/warning callouts inside dialogs are inline (orange-tinted in remote, cyan-tinted in yt-dlp, red-tinted for ADB errors) with different radii and paddings. | remote-modal, import-modal, settings paired devices |

## 11. Studio (slide editor) chrome

| # | Finding | Where |
|---|---------|-------|
| D1 | The studio uses its own zinc palette (`#121214`, `#18181b`, `#27272a`, `#3f3f46`) via `!important` rules, so it doesn't follow `--os-bg-*` or the theme. | `.editor-*` rules, bulk overlay |
| D2 | Ribbon controls: `.studio-btn` (24px, panel bg, border) next to `.studio-step-btn` (borderless) next to `.ribbon-insert-btn` (borderless tall) next to selects with a different height; disabled state is opacity-only on some. | `#slide-editor-ribbon` |
| D3 | Properties panel: unstyled native buttons (F6), its own input skin (F2), blue focus. Notes pane: unstyled labels that overflow the right edge at 1366px. | properties_panel.ts, notes_pane.ts |
| D4 | Filmstrip tag chips use a separate Tailwind color set (blue/amber/purple/rose/teal/indigo) unrelated to the verse badge colors used everywhere else. Left as-is (semantic, per-section colors), see "Deliberately unchanged". | `.fs-tag-*` |

## 12. Accessibility

- No visible focus indicator on buttons, tabs, menu items, list rows or cards (H4).
- Several text colors under 4.5:1 on their surface: `--os-text-disabled` used for placeholders (#5A5D6B on #2D2F39 ≈ 2.2:1), `rgba(255,255,255,.4)` hints.
- Settings > Paired Devices "Pair TV App" / "Install via ADB" add `.active` to `#remote-modal`, which has no `.active` style (the dialog manager shows modals with `style.display`), so those two buttons do nothing visible. This is a **behavior bug**, not styling; reported, not changed.

## What changed

The system is documented in [DESIGN_SYSTEM.md](DESIGN_SYSTEM.md). Against the findings above:

- **Color (C1-C8)**: cyan, gold, sky, teal and Tailwind blues are gone from the console chrome;
  colored text uses `--os-accent-text`, fills use `--os-brand-primary`, status uses fixed
  success/warning/danger tokens. Live = tally red, staged = tally green in every theme.
  Schedule section headers follow the theme. The undefined tokens (C4) are now defined as
  aliases. Selected-row secondary text is 80% white (C7).
- **Surfaces (S1-S4)**: one overlay recipe for every menu/popover/toast, one card recipe,
  one tile recipe; 3 shadows, 5 radii. Splitters and drop indicators use the theme.
- **Selection/hover/focus (H1-H4)**: strong selection = solid primary everywhere (schedule,
  library rows, category/version trees, Settings nav, guide nav); soft selection = tint +
  bar. Hover is a translucent overlay. A visible `:focus-visible` ring on every control.
- **Buttons (B1-B5)**: primary / secondary / ghost / danger / danger-ghost / chip, three
  sizes, icon-only; no inline button styling remains in `index.html`.
- **Forms (F1-F6)**: sunken fields distinct from buttons; `.form-field` / `.form-row` /
  `.form-check`; checkboxes, radios, ranges, file and color inputs themed; the studio
  properties panel and background popover (written for an unloaded Tailwind, so they
  rendered as raw browser buttons with all five background panes visible at once) now use
  real classes.
- **Tabs (T1-T3)**: underline tabs for page switching (library, Import, Remote); segmented
  controls for views/filters/modes.
- **Dialogs (M1-M6)**: shared shell with body-layout modifiers; footers put the primary
  action last on the right; danger header is a red rule, not a different surface.
- **Feedback (E1-E4)**: one empty-state recipe (schedule, decks, library incl. an empty
  library message, Genius/online search, paired devices); toasts and the host-auth banner
  use the overlay surface + status edge; callouts replace hand-tinted boxes.
- Inline `style=""` in `index.html`: 328 -> 53 (the rest are `display:none` toggles that JS
  owns, and the canvas internals, which were out of scope).

**Deliberately unchanged**: slide canvases, `live.html`, `stage.html`, `remote.html`,
`pairing.html`; per-section tag colors (verse/chorus/bridge badges, filmstrip tag chips);
the editor's on-canvas guides/marquee; the remote-modal `.active` bug noted above;
modal focus trapping (Tab can leave an open dialog - behavior, not styling).
