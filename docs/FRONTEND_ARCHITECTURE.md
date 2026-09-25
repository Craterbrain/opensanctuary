# Frontend Architecture: Where the HTML Lives, Where the CSS Lives

This is a map, not a tutorial. It exists so you can find the three things that
make up any piece of UI in this app — its **markup** (`web/index.html`), its
**styling** (`web/style.css`), and its **behavior** (a file under `web/src/`)
— without grepping blind.

## How to navigate this doc

Every section below has a short tag like `[ARCH:zone-top]`. That exact string
is also written as a comment in `index.html` and/or `style.css` at the
relevant spot. **Search for the tag, not the line number** — line numbers in
this document are a snapshot (taken 2026-09-18) and will drift as the files
change; the tags don't, because they live in the source as comments and move
with the code.

```
grep -rn "ARCH:zone-top" web/
```

finds every place tagged for a given section — typically one hit in
`index.html` (the markup) and one in `style.css` (the styling). If a section
has no bespoke CSS of its own (most of the 19 modals below), there's no CSS
tag for it — it's using the shared `[ARCH:modals-generic]` component classes
instead, and the table says so.

## The three files that matter, and how they relate

| File | What it is | Size |
|---|---|---|
| `web/index.html` | The entire page. One `<body>`, loaded once. Every panel, toolbar, and modal dialog in the app is a `<div>` somewhere in this one file — nothing is server-templated or fetched as a separate HTML fragment. | 1231 lines |
| `web/style.css` | The **only** stylesheet (`index.html:11`). Every class name used anywhere — in `index.html` or written by TypeScript at runtime — is styled here. There's no CSS-in-JS and no per-component stylesheet. | 3799 lines |
| `web/src/**/*.ts` | Behavior, plus a large amount of DOM construction. Bundled by `bun build` (see `web/build.sh`) into `web/dist/bundle.js`, which is the one `<script type="module">` `index.html` loads (`index.html:1228`). | ~90 files |

`src/main.ts` is the entry point. It synchronizes the network clock, then
`await import()`s the two files that do almost everything:

- **`src/app_core.ts`** (~4400 lines) — engine/WebSocket state, schedule
  rendering, preview/live canvas rendering. Also where nearly every static
  modal's `document.getElementById(...)` reference is declared
  (`app_core.ts:416-430`) and where the generic `showModal`/`closeModal`
  helpers live (`app_core.ts:511`, `:517`).
- **`src/app_ui.ts`** (~2800 lines) — search, keyboard shortcuts, menus, and
  the more involved modal flows that need real logic beyond open/close
  (the Slide Editor, the Import modal, the Schedule Guide).

Everything under `src/editor/` and `src/core/` is imported by those two (or
by each other) rather than being loaded directly from `index.html`.

## The three patterns for "where does this element live"

Every piece of UI in this app is built one of three ways. Knowing which one
you're looking at tells you where to make a change.

**1. Fully static: HTML + CSS, no JS involved in its structure.**
The desktop menu bar (`index.html:16`) is the clearest example — every menu
and menu item is hand-written `<div>`/`<button>` markup, styled by
`style.css:83` (`[ARCH:menu-bar]`). JS only attaches click handlers to
buttons that already exist; it never builds this DOM.

**2. Static shell + JS-rendered content.**
The Schedule column (`index.html:183`) is a static `<div id="schedule-items-list">`
(`index.html:218`, commented `<!-- Rendered by app.js -->`) that
`app_core.ts` fills with one `<div class="schedule-item">` per item on every
state update. The container's chrome (border, header, scrollbar) is styled
by `style.css:640`; the per-item markup `app_core.ts` generates is styled by
classes defined in that same CSS section, even though the actual `<div>`
tags for each item never appear in `index.html`. This is the most common
pattern in the app — search `style.css` for a class name before assuming it
doesn't exist just because you can't find that markup in `index.html`.

**3. Fully JS-built: no static markup at all, sometimes not even a mount div.**
This used to be how the Slide Editor's entire interior worked — as of this
writing it's down to the parts that are genuinely per-item or conditional
(the filmstrip's slide thumbnails, the canvas's rendered elements, and most
of the Properties Panel's body, which depends on what's selected). The
ribbon toolbar was the worst offender (`src/editor/toolbar.ts` used to build
~500 lines of `document.createElement` calls for buttons that never
change) and is now pattern 1 instead — see the deep-dive below. The
remaining clear example of this pattern:
- Toast notifications have **no HTML at all**, not even a mount div.
  `showToast()` (`app_core.ts:178`) creates `#os-toast-container` the first
  time it's called and appends it to `<body>`. It's styled by
  `style.css:3525` (`[ARCH:toast-notifications]`). If you search `index.html`
  for `toast` and find nothing, this is why.

---

## Top-level page structure `[ARCH:app-shell]`

`index.html:14` — `<div id="app-container">` wraps the entire visible app
(everything except modals, which are separate top-level children so they can
overlay on top of it — see below). Global CSS variables (brand colors,
surface colors, text colors) are defined in `:root` at `style.css:1`, and the
universal reset (`box-sizing`, margin/padding zero) is at `style.css:59`.

### Menu bar `[ARCH:menu-bar]`
- **HTML:** `index.html:16-99` — File/Edit/Live/Profiles/View/Help, each a
  `<div class="menu-item">` with a `<div class="menu-dropdown">` child.
- **CSS:** `style.css:83` (banner: "DESKTOP MENU BAR").
- **JS:** `app_ui.ts` attaches click handlers per menu action (search for
  `menu-file-`, `menu-edit-`, etc. — the IDs match the `<button>` IDs).

### Top ribbon `[ARCH:zone-top]`
- **HTML:** `index.html:100-178` — Left Utility Cluster (`:102`, e.g. New,
  Open, Save buttons) and Right Live Override Cluster (`:150`, e.g. Blackout,
  Clear Text).
- **CSS:** `style.css:174` (banner: "TOP RIBBON (ZONE_TOP)").
- **JS:** `app_ui.ts`/`app_core.ts` (button IDs match 1:1, no indirection).

### The 3-column workspace `[ARCH:workspace-tiers]`
- **HTML:** `index.html:179-338` — a horizontal split of three columns, with
  two draggable splitter bars between them (`:222`, `:261`).
- **CSS:** `style.css:302` (banner: "WORKSPACE & TIERS") for the split
  layout itself and the splitter bars.

| Column | Tag | HTML | CSS banner | Notes |
|---|---|---|---|---|
| 1: Schedule | `[ARCH:zone-sch]` | `:183-221` | `style.css:640` | List is JS-rendered (pattern 2 above); view-mode variants (icons/list/large) at `style.css:1176`. |
| 2: Preview | `[ARCH:zone-prev]` | `:225-260` | `style.css:1381` | Mirrors the Live column's rendering logic one step behind. |
| 3: Live | `[ARCH:zone-live]` | `:264-337` | `style.css:1309` (deck) + `style.css:1381` (canvas mirror, shared with Preview) | Also has its own playback controls bar at `:293`. |

### Resource library (bottom tier) `[ARCH:zone-lib]`
- **HTML:** `index.html:342-431` — three panes: search/filter tree (`:362`),
  catalog grid/table (`:384`), asset preview monitor (`:415`).
- **CSS:** `style.css:1581` (banner: "RESOURCE AREA (ZONE_LIB)"); grid vs.
  table view toggle at `style.css:603`.
- **JS:** catalog rendering, filtering, and drag-and-drop are in
  `app_core.ts`; the Genius lyrics search and other library-specific fetches
  live under `src/ui/` and `src/core/`.

---

## Modals `[ARCH:modals-generic]`

`index.html:432` opens a block of 19 `<div class="modal-backdrop">` elements
— every one is a **direct sibling** of `#app-container`, not nested inside
it, so they can sit on top of the whole app via `z-index` regardless of
scroll position. Almost all of them share one component system:
`.modal-backdrop` → `.modal-dialog` → `.modal-header` / `.modal-body` /
`.modal-footer`, defined once at **`style.css:1986`** (banner: "MODALS &
DIALOGS"), with named size variants (`.modal-dialog--sm/md/lg/xl`) at
`style.css:2013` so individual modals don't need inline sizing.

`app_core.ts:416-430` declares a `const xModal = document.getElementById(...)`
for most of them up front; `showModal()`/`closeModal()` (`app_core.ts:511`,
`:517`) are the generic open/close functions almost everything uses.

| # | Modal | id | HTML lines | CSS | Opened by |
|---|---|---|---|---|---|
| 1 | Slide/Presentation Studio | `create-modal` | `:437-502` | **Bespoke** — see deep-dive below | `app_ui.ts` `openSlideEditor()` (`:107`) |
| 2 | Bible/Scripture/Video Importer | `import-modal` | `:504-680` | Generic + tabs at `style.css:2434` | `app_ui.ts` `openImportModal()` (`:1532`) |
| 3 | Settings (Edit > Options) | `options-modal` | `:685-707` | **Bespoke** — `[ARCH:modal-settings]`, `style.css:2106` | `showModal(optionsModal)` |
| 4 | Keyboard Shortcuts (F1) | `shortcuts-modal` | `:709-756` | Generic | `showModal(shortcutsModal)` |
| 5 | Nursery & Alert (F8) | `alert-modal` | `:758-777` | Generic | `showModal(alertModal)` |
| 6 | Open Schedule (Ctrl+O) | `open-modal` | `:779-816` | Generic | `showModal(openModal)` |
| 7 | Save Schedule (Ctrl+S) | `save-modal` | `:818-841` | Generic | `showModal(saveModal)` |
| 8 | Media & Song Store | `store-modal` | `:843-878` | Generic | `showModal(storeModal)` |
| 9 | Web Integration | `web-modal` | `:880-907` | Generic | `showModal(webModal)` |
| 10 | Mobile Remote Control | `remote-modal` | `:909-931` | Generic | `showModal(remoteModal)` |
| 11 | About OpenSanctuary | `about-modal` | `:933-957` | Generic | `showModal(aboutModal)` |
| 12 | Theme & Background Picker | `theme-picker-modal` | `:959-984` | Generic + `style.css:3427` | dynamically shown near the trigger |
| 12b | Song Arrangement Editor | `arrangement-modal` | `:986-1042` | Generic | `showModal(arrangementModal)` |
| 13 | Schedule Guide (Articles) | `schedule-articles-modal` | `:1044-1087` | Generic | `app_ui.ts` `openScheduleGuideModal()` (`:1428`) |
| 14-17 | Context menus (Bible version, Schedule item, Schedule empty area, Slide/Verse, Library item, Live monitor) | `*-context-menu` | `:1089-1164` | `style.css:3260` ("Context Menu for Bible Versions & Items") | positioned at cursor on right-click |
| 18 | Media Background Image Picker | `media-image-picker-modal` | `:1166-1200` | Generic + `style.css:2814` | dynamically shown from property panels |
| 19 | Confirmation Dialog | `confirm-modal` | `:1202-1224` | Generic | `app_core.ts` (used pervasively for "are you sure?" prompts) |

---

## Deep dive: the Slide/Presentation Studio (`create-modal`)

This is the most complex piece of UI in the app — the one modal that doesn't
use the generic dialog chrome. It used to also be the one whose *interior*
was almost entirely JS-built; that's being walked back pane by pane (the
ribbon toolbar first) so the structure lives in `index.html` like everything
else, and the `.ts` files only wire up behavior. If you're debugging a
layout issue here, read the "known traps" section below first.

### Bespoke modal chrome `[ARCH:slide-editor-chrome]`
- **HTML:** `index.html:437-502`. Unlike every other modal, this one has no
  `.modal-header`/`.modal-body`/`.modal-footer` — it's `.studio-modal-dialog`
  containing `.studio-workspace` (`:446`, a column: the static ribbon
  toolbar on top, `.studio-workspace-row` below it holding
  `#canvas-editor-mount`, the bulk-paste overlay, and the relocated
  `#studio-sidebar-header` block) and `.studio-footer` (Author/CCLI fields +
  Save/Cancel buttons).
- **CSS:** `style.css:2547` (comment: "Slide Editor Modal Styles"), through
  roughly `style.css:3259` (right before the Context Menu section). Notably
  `.studio-modal-dialog` here is sized to fill the whole viewport rather than
  using the `.modal-dialog` card sizing every other modal uses.
- **JS:** `app_ui.ts` owns opening/closing and the title/type/author/CCLI
  fields (`getCanvasSlideEditor()`, `:69`); it constructs exactly one
  `SlideEditor` and reuses it across every open.

### The editor shell `[ARCH:slide-editor-shell]`
- **HTML:** the ribbon toolbar (`#slide-editor-ribbon`, inside
  `.studio-workspace`) is now fully static; `#canvas-editor-mount`
  (`index.html`, inside `.studio-workspace-row`) is still a single empty
  `<div>` — everything inside *it* is still JS-built.
- **CSS:** `style.css:2668` (ribbon groups) and `style.css:3615` (banner:
  "Slide Editor Modern Workspace Layout & Component Styling", covers the
  still-JS-built panes) through end of file.
- **JS:** `src/editor/slide_editor.ts`'s `initLayout()` (`:171`) wires the
  ribbon and builds the rest of this tree:

| Pane | Structure | Built/bound by | Styled at |
|---|---|---|---|
| Ribbon toolbar (slide mgmt, undo/redo, font, insert, layout, arrange, zoom) | **Static** (`index.html`, `#slide-editor-ribbon`) | `src/editor/toolbar.ts` (`EditorToolbar`) binds to it by id — builds nothing except the data-driven font-family `<option>`s and the Background popover | `style.css:2668` |
| Filmstrip (slide thumbnails, left) | JS-built (one card per slide — inherently a loop) | `src/editor/thumbnail.ts` (`FilmstripSidebar`) | `.editor-filmstrip-mount`, `style.css:3620`-ish |
| Canvas viewport (center) | JS-built (renders the active slide's elements) | `src/editor/canvas.ts` (`EditorCanvas`) | `.editor-canvas-viewport`, `style.css:2996` ("Stage Pane") |
| Sidebar column (right): relocated title/type header + Properties Panel | Header is static (`#studio-sidebar-header`, relocated); Properties Panel body is JS-built (depends on what's selected) | `slide_editor.ts:275` builds the column wrapper; `src/editor/properties_panel.ts` (`PropertiesPanel`) | `.editor-sidebar-column` / `.editor-props-mount`, `style.css:2573` (header) / `style.css:3671` (props panel — shared rule, see trap below) |
| Speaker Notes & CCLI bar (bottom) | JS-built, but fully static content (no loops/conditionals) — a good next candidate to move to `index.html` | `src/editor/notes_pane.ts` (`NotesPane`) | `.editor-notes-pane`, `style.css:3671`-ish |

The Shapes button (`#tb-insert-shapes` in the ribbon) opens a popover whose
10 shape buttons are static markup too (`.editor-shapes-popover` in
`index.html`) — `toolbar.ts` just toggles its `hidden` attribute and reads
each button's `data-shape`. The Background popover stays JS-built on
purpose: its contents come from `SOLID_PALETTE_PRESETS`/`GRADIENT_PRESETS`
(`types.ts`), so generating it from that data is more honest than
hand-writing a second copy of the same list in HTML.

### Two known traps in this codebase (found and fixed this session)

**1. A component's constructor can silently overwrite the `className` you
gave its container.** Both `NotesPane.render()` and `PropertiesPanel`'s
constructor do `this.containerEl.className = '...'` (a full overwrite, not
`classList.add`) on the exact `<div>` that `slide_editor.ts` just gave a
different class to. If you add a CSS rule targeting the class
`slide_editor.ts` assigned and it silently doesn't apply, this is almost
certainly why — check what the component's own render/constructor code does
to `.className` before assuming the CSS selector is wrong. See
`notes_pane.ts:54`'s comment and `properties_panel.ts:9`'s comment for the
specifics of how this was resolved.

**2. A leftover CSS rule can fight a live-JS-sized element and silently clip
it.** The Speaker Notes panel used to also carry a legacy `studio-notes-drawer`
class whose `max-height: 120px` rule capped its real (larger) content — the
overflow spilled past the box with `overflow: visible`, then got clipped by
an ancestor's `overflow: hidden` right at the same boundary as the footer
below it, which visually looked exactly like "content hidden behind the
Save bar." If a JS-sized flex/grid child appears clipped or shows less than
its content needs, grep every class actually present on that element (not
just the one you expect) for a stray `max-height`/fixed `height` rule — the
element's own inline styles won't win against a `!important` rule elsewhere,
and non-`!important` rules still lose to more specific or later-declared
ones.

---

## Adding a new tagged section

If you add a genuinely new, sizeable piece of UI and want it in this map:
1. Pick a short slug (`kebab-case`, e.g. `zone-foo`).
2. Add `<!-- [ARCH:zone-foo] -->` next to its HTML in `index.html` (or fold
   the tag into an existing banner comment).
3. Add `/* [ARCH:zone-foo] */` next to its CSS section banner in `style.css`.
4. Add a row/section here.
Don't bother tagging one-off tweaks to an existing section — that's what the
existing tag already covers.
