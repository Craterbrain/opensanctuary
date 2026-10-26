# Bible from a link or file

_Last edited: 2026-10-26_

**Import → Bible from a Link or File** installs a Bible from a direct link or a file on disk. The server does the download and the reading, so it works
for any website, not only the ones a plugin was granted. It replaced the Online Bibles plugin's GitHub "browse a repository" tab.

| Format | What it is | Typical source |
|---|---|---|
| **USFM** | `\id` / `\c` / `\v` markers, one file per book | ebible.org (`..._usfm.zip`), Paratext |
| **OSIS XML** | `<verse osisID="Gen.1.1">` (container) or `<verse sID=…/> … <verse eID=…/>` (milestone) | CrossWire, many sites |
| **Zefania XML** | `<XMLBIBLE><BIBLEBOOK><CHAPTER><VERS>` | Zefania modules, SourceForge |

Any of them loose, or in a `.zip` (all `.usfm`/`.sfm` files are read as one Bible; an XML Bible inside a zip is read as it is). The format is decided from the
content, never the file name. UTF-8 and UTF-16 (with a byte-order mark) are read.

## What it does

* **A link:** the operator pastes it (and may give an abbreviation); the server downloads it, reads it, and installs it. A file: the same, from the file picker.
* **Details** (name, language, rights, abbreviation) come from the file (OSIS header, Zefania `<INFORMATION>`); an abbreviation typed in wins, then the file's, then the
  file name's. They are stored with the Bible the way `.osbible` stores them.
* **Footnotes, cross references, headings, word attributes (Strong's) and other markup are dropped**: only verse text is kept. Verse ranges (`\v 3-4`) are filed under their first
  verse. Books come out in the canonical order, deuterocanon after the 66.
* **Replace is explicit.** If that abbreviation is installed, the server answers 409 and the console asks; *Replace* deletes the installed Bible first, so a smaller file leaves
  nothing of the old one behind. Other Bibles are never touched. Nothing is written until the whole file has been read.
* The chapters have the same ids the plugin host gives (`scrip_<uuid>` from book, chapter, verses and version), so a Bible installed this way, by the Online Bibles plugin or from an
  `.osbible` is the same rows.
* The dialog reminds the operator that many Bibles are copyrighted and that using them is their responsibility, as for any imported content.

## Safety

The link is typed by a person but the file is untrusted.

* **Link rules** (`plugin_net::check_public_url`, applied again on every redirect, at most four): `https` only, the standard port, no user name or password, a host *name* (not an
  address, `localhost` or a bare local name), and the connection goes to exactly the public address the name resolved to (no loopback, private, link-local, CGNAT, unique-local,
  multicast or reserved ranges: the same check as plugin fetches). 128 MB at most.
* **Console only:** `POST /api/bibles/import/link` and `/api/bibles/import/file` need the host token.
* **XML** is read by a small reader (`bible_import.rs`) that never expands entities (only `&amp; &lt; &gt; &quot; &apos;` and numeric references are decoded; a `<!DOCTYPE>` and its
  internal subset are skipped whole), loads nothing external, and uses no recursion, so a "billion laughs", an XXE or a deeply nested file cannot hurt it.
* **Zips:** at most 400 entries and 400 MB unpacked (counted as read, so a size that lies is stopped); only `.usfm`, `.sfm`, `.xml`, `.txt` and `.osis` entries are opened; entry names are never
  used as paths (nothing is extracted to disk).
* **Limits:** at most 4,000 chapters, 400 verses per chapter, 16,000 bytes per verse; a file past any of them is refused whole.
* Fuzzed: `fuzz/fuzz_targets/bible_import.rs` (`docs/FUZZING.md`).

## Where it lives

`src/storage/bible_import.rs` (formats, limits, install; tests in `src/storage/bible_import/tests.rs`) · `src/api/routes/import.rs` (the two routes) ·
`src/api/plugin_net.rs` (`check_public_url`, `fetch_public_bytes`) · `web/src/ui/bible_import_ui.ts` and the card in `web/index.html` ·
tests: `tests/core_tests/bible_import.rs`, `web/tests/e2e_bible_import.test.ts`, `web/tests/bible_import_ui.test.ts`.

Not supported yet: SWORD modules, USX, OpenSong XML, Unbound/`.txt` tables. (They would be new readers next to the three above.)
