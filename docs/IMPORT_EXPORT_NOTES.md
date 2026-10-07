# Import/Export Architecture — Discussion Notes (2026-09-28)

_Last edited: 2026-10-02 21:15_

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
| `genius_import.rs` | Genius | 345 |
| `pptx_import.rs` | PowerPoint (.pptx) | 444 |
| `ytdlp_import.rs` | yt-dlp media | 444 |
| `ewsx.rs` | EasyWorship | 1554 |

All are one-directional (pull data *in* from a format being migrated away from), except the
proposed ODP idea below, which would need both directions.

## Should importers be split into a plugin?

Two different "plugin" concepts got conflated initially — worth keeping separate:

- **`OsPlugin`** (`src/core/plugins.rs`) is a runtime dylib-loading system (`libloading`) for
  genuine third-party/optional extensions loaded from disk at runtime. Wrong shape for the
  importers — they're first-party code shipped in every build, not user-installed extensions.
- **What actually fits:** a lightweight internal `trait Importer { fn detect(...); fn import(...); }`
  + a static registry, same spirit as the existing `BibleProvider` / `MediaSearchProvider`
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
(live show state, remote sync, themes, plugin resource contribution). Coupling native storage to
a third-party format means every future feature is constrained by what that schema can represent,
plus inheriting its migrations forever. Recommendation: keep `openlp_import.rs` as the
always-compiled-in importer (most common migration source), everything else optional via feature
flag — but OS-Next's own model stays the source of truth.

## ODF/ODP as an interchange format

More promising, and partially already true without any work: `ODP` (ISO/IEC 26300,
OpenDocument Presentation) is a real open standard, and OS-Next's own rich slide model
(`ElementTransform` with normalized x/y/w/h/rotation/z-index, `SlideElement` union of
TextBlock/Image/Video/Shape/Line/Table/Group, `TextRun`/`TextParagraphStyle`) is **already
structurally close** to ODP's `draw:frame` / `text:p` / `text:span` shape model. Modeling the
internal structs *more* like ODF would be low-value — the gap isn't the data model, it's that
the importers don't fully use the model they already have (see below).

If pursued: treat as a bidirectional adapter (import .odp → native model, export native model →
.odp), same `Importer` trait slot as the others plus an `export()` method. This would be the
first adapter needing real export correctness (round-tripping matters, unlike a one-time
migration import).

## What determines how a slide actually displays

Traced through `renderSlideVisual` (`web/src/app_core.ts:2769`, mirrored in `live_output.ts`):
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

Still explicitly out of scope, same as before: `Shape`/`Line`/`Table` elements (a slide with
an authored shape, connector, or table just silently drops that one shape, as it always
has — a scan it doesn't recognize, not a crash), a grouped shape's position (`<p:grpSp>`'s
child coordinate space isn't composed with the group's own transform, so a shape inside a
group gets a position computed as if it weren't grouped — wrong, but no worse than the old
behavior of dropping it into the shared flattened blob), layout/master-inherited
backgrounds, and speaker notes.

**EasyWorship** (`ewsx.rs`): already emits one positioned `TextBlock` per source shape
(real `x`/`y`/`width`/`height`/`order_index` columns in EasyWorship's own `element` table) —
this was the working example the PPTX rewrite above was patterned against. EasyWorship
itself is fundamentally a "one text box + one background" tool per slide, so even the
pre-rewrite flattened shape was already close to lossless for that source; the rewrite
didn't touch this importer.

**Conclusion (historical, now acted on):** the model didn't need to change to fix PPTX
fidelity — the fix was having the importer emit one positioned element per source shape
instead of flattening to one TextBlock, the same way `ewsx.rs` already did. This is also
the shape a future ODP importer would need, if that's ever built.

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
