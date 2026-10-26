# OpenSanctuary file formats

_Last edited: 2026-10-24_

OpenSanctuary has its own files for passing songs, presentations, services and Bibles between
installs, or for a backup. Unlike `.ewsx` (EasyWorship's schema) and `.odp` (OpenDocument), they keep
**everything** the app holds for an item, because they are the app's own state written out.

| Extension | Holds | Save from | Open from |
|---|---|---|---|
| `.ossong` | one or more songs | right-click a song → *Save as OpenSanctuary File…*; Import dialog → *All Songs* | Import dialog → *OpenSanctuary Files* |
| `.ospres` | one or more presentations | right-click a presentation; Import dialog → *All Presentations* | Import dialog |
| `.osservice` | a whole schedule | Save dialog → *OpenSanctuary Service, with media* | Open Schedule dialog, or drop it on the schedule (top half replaces, bottom half appends) |
| `.osbible` | one Bible translation | right-click a Bible chapter; Import dialog → *Save Bible* | Import dialog |

**Why these names.** `.osp` is OpenShot's project file, `.osb` is an osu! storyboard, and `.osz` is both
an OpenLP service and an osu! beatmap, so three-letter `os?` names would fight other programs for
double-click. These four collide with nothing we know of.

## What's inside

Every extension is the same zip container, so any unzip tool can open one:

```
manifest.json          format name + version, kind, title, counts, and the media table
content.json           the app's own serde state (Songs / Presentations / a Schedule / chapters)
themes.json            the themes that content names (only if it names any)
media/<sha256>.<ext>   pictures, videos and music, each stored once however often it is used
```

* **`manifest.json`** — `format` (`opensanctuary-bundle`), `format_version` (currently `1`), `kind`
  (`song`, `presentation`, `service` or `bible`), `created_by`, `created_at`, `title`, `counts`, `themes`
  (names), and `media`: one entry per file with its SHA-256 `id`, `kind` (`image`/`video`/`audio`), `ext`,
  original `name` and `size`. Unknown fields are ignored; a `format_version` newer than the app's is refused
  with a message saying so, rather than guessed at.
* **`content.json`** — `{ "songs": [...] }`, `{ "presentations": [...] }`, `{ "schedule": {...} }` or
  `{ "bible": {...}, "chapters": [...] }`. These are the same structures the app stores and sends over its
  API (`Song`, `Presentation`, `Schedule`, …), so nothing is translated on the way in or out.
* **Media references.** A path into the media folder (`/media/images/bg.jpg`, alone or inside CSS such as
  `url('…')`, anywhere in the content) is replaced by `bundle-media:<sha256>` when saving, and by a path into
  the opening library's own media folder when opening. The file is found by scanning every string in the
  content, so a new field that holds a media path is bundled without anyone remembering to list it.
* **Themes are references.** A song names its theme; the bundle carries that theme's definition, but it is
  only used when the library opening the file has no theme of that name. Yours is never overwritten.
* **Bibles** are stored chapter by chapter in the order they were installed (the source's own book order,
  deuterocanon included), with the translation's abbreviation, name, language, license and attribution.
  The chapter text is the same shape the Bibles repository publishes.

## Opening a file

Opening is two steps. The file is first **inspected** (the server describes it and changes nothing), then
**imported**.

* **Copyright advisory.** If the file holds songs whose copyright or CCLI number says they are not public
  domain, and opening it would actually add some of them, a pop-up lists them and says churches usually need a
  license such as CCLI. It never blocks: *Continue* always imports, and *Cancel* leaves everything untouched.
  The same test decides this as decides which database a song is filed in (`is_public_domain_song`). A file
  whose songs are all in the library already opens without the pop-up.
* **Nothing is overwritten.** An item whose id is already in the library is skipped if it is identical (compared
  by content, with media compared by hash, so a picture that moved folders still counts as the same), and
  otherwise added as a **separate copy** with a new id. A theme of the same name is kept as it is.
* **Bibles** that are already installed ask before they are replaced.
* **Media** is written under a readable, stable name (`Easter_Sunrise-3fa9c2b1.jpg`: the original name plus
  eight characters of its hash), so opening the same file twice reuses the first copy instead of piling up
  duplicates.
* A **service** is not filed into the song library; it opens into the live show exactly as an `.ewsx` does
  (replace, or append), including the "one console at a time" rule.

## Licensed Bible text is not in the file

A passage from a Bible that is licensed to be shown but not kept (the NIV and NASB through YouVersion) is saved as its
**reference** only: which plugin, which Bible, `JHN.3.16-18`, and the notice. The text is not stored in an `.osservice` or `.ewsx`.
Opening the file asks the plugin to look the passages up again (it needs to be installed, approved, and have its key). See
`docs/LICENSED_SCRIPTURE.md`.

## Safety

A bundle is untrusted input (it may be emailed, downloaded or handed over on a USB stick). The reader:

* accepts only the entries listed above: any other name, including a path that tries to climb out of a
  folder, an absolute path, or a name that isn't text, refuses the whole file. Archive names are never used as
  file paths; media is written under names the app derives itself.
* requires the media table and the archive to agree exactly: nothing listed but missing, nothing present but
  unlisted.
* caps every read: 8 MB of manifest, 256 MB of content, 16 MB of themes, 2 GB per media file, 8 GB of media
  and 20,000 entries per file.
* checks each media file's **hash** and its **real content type** (the same magic-byte sniffing the other
  importers use; SVG is not accepted because it can carry script) as it streams to disk, and keeps nothing from
  a file that fails either check. A script renamed `cute.png`, or a video presented as music, is refused.
* validates a Bible's abbreviation and chapter sizes before anything is stored.
* needs the console's host token for every route (`/api/export/bundle`, `/api/bundle/inspect`,
  `/api/bundle/import`).

Uploads are streamed to a temporary file in the data directory (`tmp/bundle-*.tmp`) and exports are streamed
back from one, so a bundle with video never sits in memory. The temporary file is removed when the request
ends; leftovers from a crash are swept up after a day.

## Where it lives

* `src/storage/bundle.rs` — the format, reader and writer; tests in `src/storage/bundle/tests.rs`.
* `src/api/routes/bundle.rs` — the three routes; HTTP tests in `tests/core_tests/bundles.rs`.
* `web/src/ui/bundle_ui.ts` — opening (inspect → advisory → import) and saving; `web/tests/bundle_ui.test.ts`
  and `web/tests/e2e_bundle_files.test.ts`.
* `fuzz/fuzz_targets/bundle_open.rs` — the reader, fuzzed (see `docs/FUZZING.md`).
