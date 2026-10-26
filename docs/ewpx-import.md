# EasyWorship presentations (.ewpx)

_Last edited: 2026-10-26_

**Import → EasyWorship Presentations** adds an `.ewpx` (for example a weekly announcement deck) to the **Presentations** library, so it can be
added to a schedule every week. Before this, an `.ewpx` could only be dropped onto the live schedule.

* Text, bullets, positions and sizes come across as the other presentation imports do.
* **An `.ewpx` has no pictures in it.** Its slides name a background picture (`ready for church.jpg`) that lives in EasyWorship's own library. When that
  file is not in this library, the slide's background is cleared so the deck's **theme background** shows instead; every slide has a background. A picture that
  *is* in the media library under that name is kept. The import message says how many slides were changed.
* **Theme.** Pick one in the import card, or leave it on *Default presentation theme*. The default presentation theme is a white page, so a deck written
  in light text (most announcement decks) would vanish on it; for such a deck, when no theme is chosen and a background had to be replaced, the first dark presentation
  theme (*Presentation Dark*) is used. A theme chosen by hand is always used.
* Importing the same file again adds a copy (`LCspring 2`); it never overwrites a deck you have edited.
* Adding a presentation to the schedule applies the default theme, but **no longer removes a slide's own background** (a logo slide, a picture slide). Songs and
  scriptures still follow the theme.

A logo or other picture slide: add a slide, then give it a picture as its background in the slide editor (or import a deck that has one). Bundle files
(`.ospres`, see `file-formats.md`) carry their pictures, so a finished deck can be moved to another install.

Where it lives: `POST /api/import/ewpx` in `src/api/routes/import.rs`; tests in `tests/core_tests/ewpx_import.rs` (they need the real LCspring deck in `$OS_TEST_DOCS`
and skip without it) and the `ewpx_tests` unit tests; the card is in `web/index.html` and `web/src/app_ui.ts`.
