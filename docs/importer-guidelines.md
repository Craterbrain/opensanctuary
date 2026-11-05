# Writing an importer: engineering guidelines

_Last edited: 2026-11-05_

This document distills the architectural conventions, critical lessons learned, and failure modes encountered during the formatted import implementations for EasyWorship (`.ewsx`), PowerPoint (`.pptx`), OpenDocument Impress (`.odp`), OpenLP (`.sqlite`, `.osj`, `.osz`), and FreeShow (`.show`, `.project`).

For developers. OpenSanctuary's own file types (`.ossong`, `.ospres`, ...) are described for users in
[`file-formats.md`](file-formats.md).

---

## 1. Core Architectural Rules

### Rule 1: Configuration & Directory Threading Over Hardcoded Paths
- **Never** hardcode CWD-relative paths (e.g. `PathBuf::from("web/media/images")`). The server can be launched from arbitrary working directories with `--web-dir <path>`.
- Any importer function that extracts or copies media files must accept `media_dir: &Path` as a parameter.
- Sibling routes and parent loaders (`open_schedule`, `import_openlp`, `import_pptx`, `import_freeshow_show`) must derive `media_dir` from the app's configured directory:
  ```rust
  let media_dir = app.web_dir.join("media").join("images");
  ```
- If a function has both a default convenience signature and a parameterized variant, thread `media_dir` through explicit helper variants (e.g. `load_schedule_from_bytes_with_media_dir`) and keep the un-parameterized version as a thin wrapper for unit tests.

### Rule 2: Verify Schemas Against Real Files (Never Guess)
- **Do not guess schemas.** When an implementation plan flags a schema as unverified, that is an explicit mandate to inspect actual sample bytes before writing parser code.
- Check local directories first: real third-party applications often store configuration and themes locally (e.g. `~/.local/share/openlp/themes/` contains modern OpenLP themes as flat JSON, not XML).
- If real local files are unavailable, pull real samples from the official open-source repository or documentation.
- Writing a parser against an unverified hypothesis produces code that compiles, passes tests written against its own fabricated fixtures, and silently fails or corrupts real user data.

### Rule 3: The Reachability Invariant (Exhaustive Call-Site & Entry-Point Audit)
- **Unreachable code is dead code in disguise.** Implementing an enhanced function, a richer signature, or a `_v2`/`_with_*` parameterized variant in a storage or utility module is only the first half of a task. If the application's actual entry points continue calling the legacy, un-upgraded function, the feature does not exist in production—no matter how many tests pass against the new helper directly.
- **The Call-Site Sweep Mandate**:
  Whenever introducing an upgraded, parameterized, or replacement interface:
  1. *Exhaustive Caller Inventory*: Grep the entire codebase for every call site of the superseded interface.
  2. *Migrate Live Production Paths*: Audit every caller and migrate all production entry points—HTTP route handlers, schedule dispatchers, CLI commands, UI event handlers, and auto-detection routines—to the enriched interface. Only retain the older signature as a thin compatibility wrapper if needed for backwards compatibility or minimal synthetic test cases.
  3. *Trace from the Wire/Edge*: Verify that the execution path starting from the user-facing boundary (an incoming HTTP request, an uploaded file stream, or a UI event) traverses all the way down into your new implementation.
  4. *Beware False Test Confidence*: Unit tests that invoke the new internal function directly only prove that the function works in isolation; they provide zero evidence that the application itself runs it. Always test through the public entry point to guarantee reachability.

### Rule 4: Graceful `None` Degradation Over Fabricated Paths
- If an optional background image, media file, or theme cannot be resolved or served, return `None`.
- **Never manufacture fallback strings** (e.g. returning a bare filename or raw local filesystem path as `Some(SlideBackground::Image { file_path, .. })`).
- In a web client, an unservable local path or bare name results in broken 404 image elements on screen. `None` is an expected, supported state that allows the system to fall back cleanly to defaults or theme styling.

### Rule 5: Explicit Verification Citations in Code
- When an importer's schema is verified against an external open-source repository, document the exact citation in the module doc comment:
  - Upstream repository (e.g. `github.com/ChurchApps/FreeShow`)
  - Source file (e.g. `src/types/Show.ts`)
  - Commit SHA (e.g. `commit 95d57096245e`)
- Citing the source provides durability, allows future maintainers to audit changes when third-party schemas evolve, and distinguishes verified schemas from guesswork.

### Rule 6: Format Capability Restraint
- Do not invent rich-text or styling concepts that the source format cannot express.
  - OpenLP lyrics are structurally plain text with line breaks (`<lines>`, `<verse>`) — do not fabricate character runs or text styling.
  - Only populate `Slide.elements` when real formatting (bold, italic, colors, custom fonts, non-default alignment) is present.
  - If a slide has no formatting, leave `elements` empty so `Slide::synthesize_v2_if_needed()` can provide standard defaults.

---

## 2. Multi-Tier Verification Standard

Every format importer must satisfy four levels of verification before landing:

1. **Unit Tests (In-Memory)**:
   - Hand-crafted minimal fixtures testing individual parser components (e.g. RTF control words, DrawingML run properties, FreeShow CSS styles, OpenLP JSON/XML themes).
2. **Round-Trip & Integration Tests**:
   - Test against real fixture files or dynamically generated archives.
   - Bypassing serialization shortcuts: do not rely on internal JSON dumps (`manifest.json`) when verifying format decoders.
3. **Live HTTP Route Tests**:
   - Spin up a test server or `TcpListener` on port 0, post base64 file payloads to the live route (`/api/import/*`), and verify database insertion and returned JSON metadata.
4. **End-to-End Visual Verification (Playwright)**:
   - Load the imported presentation, take it live, and verify visual rendering in a real browser (text contrast, alignment, and background image/video loading).

---

## 3. Supported Presentation Formats Reference

| Format | Extension | Importer Module | Primary Schema / Packaging | Background Support | Text Formatting Support |
|---|---|---|---|---|---|
| **EasyWorship** | `.ewsx`, `.ewpx` | `src/storage/ewsx.rs`, `src/storage/rtf.rs` | Zip archive containing SQLite `main.db` + RTF text in `Song.words` / `Schedule.xml` | Solid colors, image/video resources from `resource_image` / `resource_video` tables | Rich RTF: bold, italic, underline, strike, color, font, size, alignment |
| **PowerPoint** | `.pptx` | `src/storage/pptx_import.rs` (+ `pptx_inherit.rs`, `pptx_geom.rs`, `pptx_chart.rs`, `pptx_table_styles.xml`, `emf.rs`, `wmf.rs`, `svg_raster.rs`) | Open Packaging Conventions (OPC) Zip with OOXML DrawingML; styles inherited master → layout → placeholder → shape → paragraph → run | Solid, gradient, picture and tiled fills from slide, layout or master; theme colours and fill styles | DrawingML runs (`a:rPr`) with inherited styles: bold, italic, underline, strike, size, colour, theme fonts, alignment, bullets/numbering, spacing |
| **OpenDocument Impress** | `.odp` | `src/storage/odp_import.rs` (+ `odp_geom.rs`, `odp_chart.rs`) | ODF Zip: `content.xml` + `styles.xml` with style inheritance and master pages | Solid, gradient, hatch, picture and tiled fills from page or master | ODF text styles: bold, italic, underline, strike, size, colour, font, alignment, list numbering |
| **OpenLP** | `.sqlite`, `.osj`, `.osz` | `src/storage/openlp_import.rs` | SQLite DB (`songs.sqlite`), Service JSON (`.osj`), Zip archive (`.osz`) | Modern flat JSON (`~/.local/share/openlp/themes/*/*.json`), legacy XML / `.otz` packages | Plain text with verse labels; formatting not supported by source |
| **FreeShow** | `.show`, `.project` | `src/storage/freeshow_show_import.rs` | JSON array `[id, show]` or object `{...}` with `layouts`, `slides`, `media` | Solid colors, layout backgrounds, embedded base64 media, local/remote image & video (`SlideBackground::Video`) | Inline CSS runs: bold, italic, underline, strike, color, font-family, font-size, letter-spacing, alignment |

---

## 4. Pre-Commit Checklist for New Importers

- [ ] Schema verified against a real local file or upstream commit SHA (with citation in module header).
- [ ] No hardcoded relative paths; `media_dir: &Path` threaded from `app.web_dir`.
- [ ] Reachability invariant verified: exhaustive call-site sweep completed, migrating all production entry points (HTTP upload routes, dispatchers, auto-detection).
- [ ] Missing or unresolvable assets degrade gracefully to `None` without fabricating paths.
- [ ] `Slide.background` (legacy CSS string) and `Slide.background_v2` (`SlideBackground`) both set.
- [ ] `Slide.elements` left empty when no formatting is present.
- [ ] Automated tests passing (`cargo test` count strictly increasing, zero regressions).
- [ ] Verified via live HTTP route and/or Playwright rendering.
