# Import/Export Architecture — Discussion Notes (2026-09-28)

_Last edited: 2026-10-15_

Notes from a design conversation about `src/storage`'s importers, whether they should be
split out as plugins, adopting OpenLP or ODF/ODP as a format, and what the actual fidelity
gaps are today — this is a record of the discussion and its conclusions. The PPTX
importer-fidelity gap (one flattened TextBlock per slide instead of positioned per-shape
elements) and the autofit-checkbox follow-on decision have both since been implemented —
see "Import fidelity findings" below for what's actually built and what's still out of scope.

## Current state

`src/storage` holds two different things under one module: persistence (`db.rs`, `keyring.rs`,
`paths.rs`) and format import/interop adapters:

| File | Format | Lines |
|---|---|---|
| `openlp_import.rs` | OpenLP | 946 |
| `freeshow_import.rs` / `freeshow_show_import.rs` | FreeShow | 872 / 746 |
| `pptx_import.rs` | PowerPoint (.pptx) | 444 |
| `ewsx.rs` | EasyWorship | 1554 |
| `odp_import.rs` / `odp_export.rs` | OpenDocument Presentation (.odp) | see below |

All the migration-only ones are one-directional (pull data *in* from a format being migrated
away from); ODP is bidirectional -- the idea below, now implemented.

## Should importers be split into a plugin?

Two different "plugin" concepts got conflated initially — worth keeping separate:

- **`OsPlugin`** (`src/core/plugins.rs`) is a runtime dylib-loading system (`libloading`) for
  genuine third-party/optional extensions loaded from disk at runtime. Wrong shape for the
  importers — they're first-party code shipped in every build, not user-installed extensions.
- **What actually fits:** a lightweight internal `trait Importer { fn detect(...); fn import(...); }`
  + a static registry, same spirit as the existing `MediaSearchProvider`
  trait-and-registry patterns already documented in `docs/PLUGINS.md`. Gets the structural win
  (clean per-format boundary, easy to add a format) without runtime-loading complexity.
- **"Build without it" (smaller binary):** that's a Cargo feature-flag problem, not a
  plugin-loading problem — gate each importer's registration behind
  `#[cfg(feature = "pptx-import")]` etc. Feature flags are compile-time only; if *runtime*
  toggling (one shipped binary, enable/disable without rebuilding) turns out to be a real
  requirement, that's what would justify reaching for `OsPlugin` instead. Not needed for the
  stated goal.

## OpenLP as the default/native format?

Considered and **not recommended** as the app's native storage format. OS-Next's own
`src/core/models.rs` schema exists because it needs things OpenLP's format wasn't designed for
(live show state, remote sync, themes). Coupling native storage to
a third-party format means every future feature is constrained by what that schema can represent,
plus inheriting its migrations forever. Recommendation: keep `openlp_import.rs` as the
always-compiled-in importer (most common migration source), everything else optional via feature
flag — but OS-Next's own model stays the source of truth.

## ODF/ODP as an interchange format -- **implemented** (2026-10-14)

Was "more promising, and partially already true without any work": `ODP` (ISO/IEC 26300,
OpenDocument Presentation) is a real open standard, and OS-Next's own rich slide model
(`ElementTransform` with normalized x/y/w/h/rotation/z-index, `SlideElement` union of
TextBlock/Image/Video/Shape/Line/Table/Group, `TextRun`/`TextParagraphStyle`) was already
structurally close to ODP's `draw:frame` / `text:p` / `text:span` shape model, confirmed once
`odp_import.rs` was actually written: no model changes were needed, same conclusion the PPTX
fidelity work below already reached.

A library search came first (an MIT Rust crate to adopt, or at least reference): no crate cleared
the bar. The closest fit, `libreoffice-rs`/`lo_impress`, is MIT and genuinely reads/writes ODP
in pure Rust, but is self-described as "not recommended for production" (19% documented, 36
stars). Built against the OASIS spec instead, the same `next_tag`/`extract_attr` hand-rolled XML
scanning `pptx_import.rs` already used (pulled out into `storage::xml_scan` once both needed it),
validated against a real presentation built and saved by actual LibreOffice via its `pyuno`
scripting bridge (`testdata/odp/`), not just hand-authored XML believed to match the spec.

`odp_export.rs` is the first adapter that needed real export correctness, and is checked two
ways: round-tripped through `odp_import.rs`, and -- the stronger check -- handed to real
LibreOffice headless (`soffice --convert-to pdf`), confirmed by eye once via a throwaway debug
render (`docs/` doesn't keep the image; see the commit that added `odp_export.rs`).

`POST /api/import/odp` and `GET /api/export/odp/:id` (console-only, mirroring `/api/schedule/export`'s
EWSX download) are wired into the Import modal and the library's item context menu
("Export as ODP…", presentations only) respectively. Export always writes a fixed 28cm x 15.75cm
(16:9) page, since `ElementTransform` only stores relative coordinates, never an authored absolute
canvas size. Still out of scope: a rotated frame (`draw:transform`), `draw:connector` (an anchored
connector between two other shapes by ID, which this app's `Line` has no equivalent for),
gradient/bitmap slide backgrounds (solid only), and master-page-inherited content other than page
size.

### Shapes, lines, tables, and groups -- **added** (2026-10-15), after a real gap was reported

The first version of this importer/exporter (above) deliberately dropped `Shape`/`Line`/`Table`/
`Group` elements, on the reasoning that PPTX's own importer has the same gap. That reasoning didn't
hold up: this app's own model and editor *do* support all four (`web/src/editor/shape_library.ts`,
`line_tool.ts`, `table.ts`), so dropping them on an ODP round trip was a real fidelity bug, not
parity with an existing limitation -- caught when a live import → app → export round trip (the
same kind of check that found the white-on-white bug above) was redone against a real fixture with
an actual rectangle on it and the rectangle came back missing.

`odp_import.rs` now reconstructs: `draw:rect`/`draw:ellipse`/`draw:custom-shape` → `Shape`,
`draw:line`/`draw:polyline`/`draw:path` → `Line`, a `<table:table>` inside a `draw:frame` →
`Table`, and `draw:g` → `Group` (recursing to any depth via a new `xml_scan::extract_balanced_tag`,
since a same-named tag can nest inside itself -- the existing `next_tag` helper can't handle that,
only ever having been asked to handle non-self-nesting shapes before). A `draw:custom-shape`'s
`draw:type` (its ODF preset name) is translated to this app's own `shape_kind` vocabulary via
`map_custom_shape_type`; `odp_export.rs` writes it back out the same way, so this app's own
`shape_kind` strings round-trip exactly, and a real producer's preset shapes come through as the
closest equivalent this app can render.

**The preset name list was verified against real LibreOffice, not guessed or copied from its
source.** LibreOffice's own shape-gallery toolbar resource files (`/usr/lib/libreoffice/share/
config/soffice.cfg/modules/simpress/toolbar/*.xml`) list every `.uno:XShapes.y` command the
Basic/Symbol/Arrow/Star/Callout/Flowchart galleries expose; for each one, a probe `.odp` naming a
`draw:custom-shape` with only `draw:type="<candidate>"` and no `draw:enhanced-path` at all was
handed to real `soffice --convert-to pdf`, and only a name Impress itself filled in a distinct
default glyph for was treated as confirmed (a blank render just means Impress has no built-in
default geometry for that name without a path -- not proof the name is wrong, since a real producer
file always carries an explicit path anyway -- so this is a strong *positive* signal and a weak
negative one; unconfirmed names were left out rather than guessed). All 28 `flowchart-*` names
confirmed this way. The shape names themselves, and which gallery they live in, are plain facts
about a public, standardized vocabulary (the same hyphenated-preset convention is shared across
ODF and OOXML interchange) -- not copyrightable expression -- and every outline this app actually
draws for them (`shape_library.ts`'s new `case` arms, mirrored in `odp_export.rs`'s
`shape_enhanced_path`) is a clean-room fixed-point polygon designed from scratch, not traced from
any other application's asset or source.

Two real mistakes this verification process itself caught, worth recording:
- **`draw:enhanced-path`'s two-subpath "hole" trick doesn't work.** The "ring" shape was first
  implemented as two opposite-wound octagon subpaths in one `draw:enhanced-path`/SVG `<path>`,
  expecting a nonzero/even-odd fill rule to render the gap as a hole, the same way SVG would. Real
  LibreOffice rendered it as a *solid* filled blob instead -- it does not apply a hole-by-winding
  rule to `draw:enhanced-path`'s "N"-terminated subpaths the way SVG does for its own path data.
  Found by actually rendering one and looking, not by the round-trip-identity test (which only
  checks `shape_kind`/`fill_color` string equality after re-import, and would have stayed green
  even with a solid blob). Fixed by drawing "ring" as a single unfilled, thick-stroked octagon
  outline instead (`fill_color` becomes the stroke's color, a documented special case in both
  `render_element` and `extract_shape_element`), and a dedicated regression test now asserts the
  exported XML is actually unfilled (`test_ring_shape_exports_as_unfilled_not_a_solid_blob`),
  rather than trusting the round-trip test alone.
- **`Shape`/`Line`'s `stroke_width` is not a physical unit.** Both `shape_library.ts` and
  `line_tool.ts` draw into a fixed `0 0 100 100` SVG viewBox and pass `stroke_width` straight
  through as that viewBox's own `stroke-width` -- so it's really "percent of the shape's own box"
  (its shorter side), not points or any other absolute unit. The first version of this importer
  converted a real `svg:stroke-width` (in cm) to points, which is the wrong unit entirely; fixed by
  converting to/from that same relative-to-box percentage instead
  (`cm_to_relative_stroke`/`relative_stroke_to_cm`).

Exotic preset shapes this app's own editor still can't draw a real outline for (confirmed in the
real-world `shape-test.odp` fixture below: a heart, a Pac-Man-style pie shape, a trapezoid-cup, a
brace pair, and an MS-Office-imported blob) fall back to a plain rectangle glyph -- position, size,
and fill/stroke-none are still preserved exactly, only the precise outline is approximated, the
same honest degrade this importer already used for anything it doesn't recognize at all. A 3D
shape (`dr3d:scene`, e.g. an extruded cube) is genuinely out of scope -- this app's model has no 3D
concept at all -- and confirmed (via that same real fixture) to not confuse the page-shape scanner
into misparsing anything around it.

`testdata/odp/README.md`'s fixture doesn't exercise any of this (it predates it); a second,
real-world fixture does: `shape-test.odp`, hand-authored in actual LibreOffice Impress's shape
gallery by a human (not `pyuno`), referenced via the same `$OS_TEST_DOCS` convention
`docs/workflow.md` already documents for EasyWorship fixtures (not committed to the repo -- a
human's own test file). It's what caught `resolve_graphic_style` not walking
`style:parent-style-name`: every one of its 9 shapes leaves fill/stroke completely unset on its own
style and inherits from a shared parent (`"standard"`, almost universal real-producer behavior for
a shape gallery) -- the first version of `resolve_graphic_style` only looked at one named style in
isolation, so every shape in that file came back unfilled (`fill_color: "none"`) until the
inheritance walk was added.

**A real round trip was actually driven through the running app** (not just tested at the data
layer) to answer "does an imported .odp look the same, and does it still look the same after
exporting again": a Playwright session imported `testdata/odp/fixture.odp` through the real
Import modal, opened the result in the real Slide Editor and screenshotted it, exported it back
out through the real "Export as ODP…" menu item, and handed the result to real LibreOffice
headless. This caught a real bug `odp_export.rs`'s own tests hadn't: a run with no explicit color
(this app's own placeholder default, white, since slides are normally shown over a dark
background/video) was written into the file as a literal `fo:color="#ffffff"`. Re-opened in a
real ODF tool, which defaults an unstyled page to white, that text was invisible --
white-on-white. Fixed by omitting `fo:color`/`style:font-name` entirely when a run sits at *both*
of this app's own placeholder defaults at once (the one case the exporter can tell apart from "the
source explicitly asked for white" with any real confidence), so the opening tool's own default
(black, a real font) applies instead. Confirmed fixed by redoing the same live round trip:
the text is visible again in LibreOffice's own render.

One thing observed along the way that is not a bug, worth recording so it isn't rediscovered as if
it were: the same text can wrap differently in the app's own canvas versus LibreOffice's renderer
at the same box width, since browser and LibreOffice layout engines use different font metrics for
the same font name/fallback -- an inherent cross-renderer difference, not a position or size that
failed to round-trip. (This fixture's rectangle, mentioned as a dropped shape in an earlier version
of this note, is no longer dropped -- see "Shapes, lines, tables, and groups" above.)

### Checked against the actual OASIS standard, not just LibreOffice's own UI -- (2026-10-15)

Everything above was built and verified against real producer output, but "real producer" so far
meant specifically LibreOffice Impress's own shape gallery. A follow-up pass asked the more
rigorous question: what does the actual ODF 1.3 standard (OASIS, not LibreOffice's toolbar
conventions) define as legal `<draw:page>` content, and does this importer silently drop any of
it? It does not (drop anything previously silent) -- but four more spec elements, distinct from
anything `draw:custom-shape`'s preset vocabulary covers, were found and added:

- **`draw:polygon`** (a closed point list) was a genuine silent-drop gap, the same class of bug as
  the original `Shape`/`Line`/`Table`/`Group` one above: it simply wasn't in the page-shape
  dispatcher's candidate list at all, so a file using one (instead of the functionally-equivalent
  `draw:custom-shape`) lost that shape entirely. Fixed; falls back to a plain rect glyph on import
  (no point-list field on `Shape` to preserve the exact outline), same honest degrade as an
  unrecognized custom-shape preset, but position/size/color are kept.
- **`draw:regular-polygon`** (a true N-sided polygon, `draw:corners`/`draw:concave`) is mapped by
  corner count to the closest shape this app can draw (`regular_polygon_kind`).
- **`draw:caption`** (a pointer-line callout, a distinct spec element from the `*-callout`
  custom-shape presets) reuses the existing `"callout"` glyph.
- **A shape wrapped in a hyperlink** (`<draw:a href="...">...</draw:a>`) is unwrapped in place,
  continuing the page's own z-order sequence, rather than missed for sitting one tag deeper than
  every other page-level shape (the link itself has no equivalent in this app's model, same gap
  PPTX's own importer has).

One real finding from verifying these against actual LibreOffice (not assumed): a bare
`draw:regular-polygon` with only `draw:corners`/`draw:concave` and no supplementary path data
renders as *nothing at all* in real LibreOffice's own importer -- it does not compute the N-gon's
geometry from just the corner count the way the spec's attribute names might suggest. This
importer still extracts position/color/shape_kind for one regardless, which is more lenient than
the reference implementation, not a bug -- there was simply nothing to cross-check the *visual*
result against for this one case, unlike `draw:polygon`, which real LibreOffice does render.

Also checked and confirmed correctly out of scope (no sensible equivalent in this app's model, or
rare enough in a real slide deck not to be worth it): `draw:connector`, `draw:measure`,
`draw:control`, `draw:object`/`-object-ole`/`draw:applet`/`draw:plugin`/`draw:floating-frame`
(embedded sub-documents/OLE/Java-applet/plugin content), `draw:layer-set`, image-maps/contours, and
the entire animation/transition/sound/click-action vocabulary (`anim:*`,
`presentation:animations`/`-sound`/`-event-listener`) -- the last of these is a real, separate
product question (this app's `Slide`/`SlideElement` model has no animation concept at all), not a
parser gap, and was deliberately left alone this pass.

## What determines how a slide actually displays

Traced through `renderSlideVisual` (`web/src/core/slide_render.ts`, called from `app_core.ts`'s
Preview/Live canvases and from the Slide Editor's filmstrip thumbnails/`ui/slide_deck_view.ts`;
`live_output.ts` has its own independent copy of the same branch, not a call to this function):
branches on `slide.elements.length > 0`.

- **Elements present** → `renderSlideElements` (`web/src/core/slide_render.ts`): the rich
  positioned-element model, shared code between the editor canvas, Preview, Live, and
  projector/stage outputs.
- **No elements** → flattened single-textbox `text`/`background` string + `autoFitLyrics`
  autofit — the "legacy lyrics slide" path (songs/scripture).

A comment in `app_core.ts` documents that this dual-path split already caused a real bug
(Preview and Live independently autofit and visibly disagreed on text size) before being
unified into one shared function.

## Import fidelity findings

**PPTX** (`pptx_import.rs`) — **rewritten to emit one positioned element per shape,
confirming the "importer gap, not model gap" conclusion below.** Each `<p:sp>` text shape
becomes its own `SlideElement::TextBlock`, positioned/sized via its real `<p:spPr><a:xfrm>`
(EMU coordinates, normalized against the slide's actual `<p:sldSz>`, falling back to the
standard 16:9 size if absent) rather than the old shared full-slide default — a
caption-over-a-photo or side-by-side-columns slide now keeps each piece where it was
authored instead of collapsing into one text blob. Each `<p:pic>` inline picture (a shape
in the slide body, as opposed to a slide-level `<p:bg>` fill) becomes its own
`SlideElement::Image`, extracted into `media_dir` via the same relationship-resolution
chain `extract_slide_background` already used for background pictures — inline images
(a positioned logo, a photo with a caption box over it) were never imported at all before
this, not just flattened. `z_index` is assigned in real document/paint order. The legacy
flat `text` field is kept correct via `Slide::project_text_from_elements()` instead of a
separately-tracked flat run list, now that a slide can carry more than one `TextBlock`.

### Shapes, lines, connectors, tables, and groups — **added** (2026-10-15), ported from ODP

The gap directly above (`Shape`/`Line`/`Table` elements silently dropped, a grouped shape's
position read as if it weren't grouped) was fixed the same way the identical ODP gap was: a
non-text `<p:sp>` with its own `<a:prstGeom>` becomes a `Shape`; a plain `<p:sp>` with
`prst="line"` (how LibreOffice's own OOXML export represents a simple line, confirmed
against real output) or an actual `<p:cxnSp>` connector becomes a `Line` (`line_kind` from
the connector's own preset family — `straightConnector1`/`bentConnectorN`/
`curvedConnectorN`, Microsoft's own published vocabulary); a `<p:graphicFrame>` wrapping an
`<a:tbl>` becomes a `Table`; a `<p:grpSp>` becomes a `Group`, recursing to any depth via
`xml_scan::extract_balanced_tag` (a group can nest inside itself) and remapping each child's
local coordinates through the group's own `<a:off>`/`<a:ext>`/`<a:chOff>`/`<a:chExt>`
(`GroupRemap`) — a step ODF's `draw:g` never needed, since an ODF group's children are
already in absolute page coordinates, but OOXML's aren't. A nested group's own off/ext are
themselves in the *outer* group's local space and must be remapped through it before
building its own children's remap — caught and fixed via a hand-authored two-level-nested-
group test, not assumed correct.

An OOXML preset shape name (`<a:prstGeom prst="...">`) is translated to this app's
`shape_kind` vocabulary via `map_prst_geom_type`, every entry taken verbatim from
Microsoft's own published `ShapeTypeValues` enumeration (the exact serialized string each
member documents) — not guessed from memory and not copied from ODF's equivalent mapping,
since the two vocabularies genuinely diverge for the same shape (`donut` not `ring`, `plus`
not `cross`, `lightningBolt` not `lightning`, `flowChartInputOutput` not the ODF-side
`flowchart-data`) — getting any of these wrong from memory would have silently mis-bucketed
real shapes, so each one was checked against that primary source before being written down.

Two deliberate, documented simplifications surfaced by this work, not oversights:
- **Only explicit, literal color is ever read**, never a theme color (`<a:schemeClr>`) and
  never a shape's `<p:style>`/`<a:fillRef>` reference into the theme's format scheme (same
  "explicit values only" policy text-run colors already used). This is a real, *common* gap,
  confirmed by research into real-world OOXML parsing pain points, not a rare corner case: a
  shape inserted via the authoring tool's UI without manually overriding its fill very often
  has *no* explicit `<a:solidFill>` at all, only a `fillRef` — such a shape imports with this
  app's own default (no color resolved), not a crash and not a guessed color. Covered by a
  dedicated test (`test_pptx_import_theme_colored_shape_has_no_crash_and_no_guessed_color`).
- **A rotated group's children are not re-composed around the rotation** — `GroupRemap` only
  applies the group's own linear offset/scale; a `<p:grpSpPr>`'s own `rot` isn't read at all.
  This is a genuinely hard problem (even mature implementations have long-standing open bugs
  about rotated+grouped shape positions), and an unrotated group — the overwhelmingly common
  case — is unaffected.

Validated against a real fixture (`testdata/pptx/fixture.pptx`, produced by actual
LibreOffice's OOXML export, not hand-authored XML — see that directory's `README.md`), which
confirmed two structural facts before any code was written against them: a table's
`<p:xfrm>` sits directly under `<p:graphicFrame>`, not nested in a `<p:spPr>` like every
other shape's transform; and an untouched group's `<a:off>`/`<a:ext>` exactly equal its own
`<a:chOff>`/`<a:chExt>`, making `GroupRemap`'s general formula reduce to the identity in
that common case.

Still out of scope: layout/master-inherited backgrounds, and speaker notes.

Two real bugs an independent review of this work caught before merge, worth recording:
- **A table's merged-cell handling silently misplaced data for any merge not at a row's
  trailing edge.** The first version tracked a running column position by advancing past
  `gridSpan` for the origin cell and past 1 for each `hMerge`/`vMerge` continuation -- but a
  `<a:tr>` always carries exactly one `<a:tc>` per grid column regardless of `gridSpan`
  (confirmed against documented real OOXML examples, not assumed): `gridSpan` is purely
  descriptive of how many columns a cell visually covers, and every covered column still gets
  its own (`hMerge`-marked) `<a:tc>`. Advancing past `gridSpan` double-counted those columns,
  shifting every real cell after a merge one column too far right. Fixed by placing each
  `<a:tc>` purely by its ordinal position in the row (position *is* the column index,
  `gridSpan` plays no part in placement), and caught with a test designed so a naive
  "append real cells, pad the row's end" bug would produce a visibly different (wrong) result
  from correct positional placement -- the original test's own merge happened to sit at the
  row's trailing edge, where that bug produces the same output as the fix, so it couldn't have
  caught this itself.
- **A `<p:sp>` with `prst="line"` that also carried real text silently dropped the text.** The
  first version checked `prst=="line"` before ever looking at `<p:txBody>`; a line-preset
  autoshape can legally carry one in real OOXML. Fixed by trying real text first, unconditionally
  (the same priority this importer already used for text vs. non-text shapes before `Line`
  existed at all), and falling back to the line/shape distinction only when there's none.

Also worth recording: the first version's test for the hardest part of this work (a nested
group's coordinates, remapped through its own parent group's transform before building its own
children's remap) asserted `shape_kind`/`fill_color`/child counts but never a single x/y/w/h --
it would have stayed green even if the whole nested-remap step were deleted. Fixed with explicit
numeric position assertions (confirmed to actually fail when the fix is disabled, not just
asserted to work).

**EasyWorship** (`ewsx.rs`): already emits one positioned `TextBlock` per source shape
(real `x`/`y`/`width`/`height`/`order_index` columns in EasyWorship's own `element` table) —
this was the working example the PPTX rewrite above was patterned against. EasyWorship
itself is fundamentally a "one text box + one background" tool per slide, so even the
pre-rewrite flattened shape was already close to lossless for that source; the rewrite
didn't touch this importer.

**Conclusion (historical, now acted on):** the model didn't need to change to fix PPTX
fidelity — the fix was having the importer emit one positioned element per source shape
instead of flattening to one TextBlock, the same way `ewsx.rs` already did. `odp_import.rs`
(see above) was built the same way, and went further: `Shape`/`Line`/`Table`/`Group` too, not
just `TextBlock`/`Image` -- PPTX's own shape/table/group gap (just above) is still real and
unfixed, tracked here as a known follow-on, not parity to preserve.

## Follow-on decision: autofit checkbox (presentations only) — **implemented**

Came out of this discussion, tracked separately as a product requirement: autofit for
**presentation slides only** (the `TextBlock.block.autofit` flag, rich `elements[]` path) is a
user-facing checkbox in the Slide Editor's properties panel (`web/src/editor/properties_panel.ts`,
`#prop-autofit`) plus a matching toolbar toggle (`web/src/editor/toolbar.ts`), both wired through
`SlideEditor.applyTextStyle()`. `pptx_import.rs` constructs imported text blocks with
`autofit: false` so PPTX text keeps its authored position instead of silently
reflowing/shrinking against a background; the operator opts in via the checkbox. Three render
paths (`text_block.ts`, `canvas.ts`, `slide_render.ts`) all gate the `autoFitLyrics()` call on this
flag. Explicitly **out of scope**, and unaffected: song/scripture slides (flat `text`/`background`
+ `autoFitLyrics` path, `ewsx.rs`) keep autofitting unconditionally, same as before — visual
anchoring needs there are expected to be solved via themes, not this checkbox.
