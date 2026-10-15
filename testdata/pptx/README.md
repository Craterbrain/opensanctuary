# PPTX test fixtures

- `fixture.pptx`: a real presentation produced by actual LibreOffice Impress (via its
  `--convert-to pptx` OOXML export filter, not hand-authored XML), so `pptx_import.rs`'s
  `Shape`/`Table`/`Group` tests run against the same shapes a real producer emits, not just what
  ECMA-376/ISO 29500 says is legal. It started life as an ODP file built the same way
  `testdata/odp/make_fixture.py` builds `testdata/odp/fixture.odp` (two title/subtitle placeholder
  boxes from the slide layout, a 2x2 table with "R0C0".."R1C1" in its cells, and a group of two
  colored rectangles), then was converted to `.pptx` to get real OOXML out the other end. Not
  committed as a `.py` script this time -- the generating script lived in a throwaway scratch
  directory, not this repo -- but the same two-step "build real content with the actual
  application, then export to the target format" method, not hand-authored XML asserted to match
  the spec.
- Confirmed by this fixture, not assumed: a `<p:graphicFrame>` wrapping an `<a:tbl>` has its own
  top-level `<p:xfrm>` (not nested in a `<p:spPr>` the way every other shape's transform is), and a
  `<p:grpSp>`'s `<a:off>`/`<a:ext>` exactly equal its own `<a:chOff>`/`<a:chExt>` in the common case
  (an untouched, never-resized-after-grouping group), making its children's own coordinates already
  absolute -- the general `GroupRemap` formula (`pptx_import.rs`) subsumes this as the identity case.
