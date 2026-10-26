# Licensed Bible text

_Last edited: 2026-10-26_

Some Bibles (the NIV, NASB, AMP and others, offered through the YouVersion Platform) are licensed to be **shown, not
kept**. OpenSanctuary treats their text differently from a Bible you own (`docs/file-formats.md`, `bibles/<VERSION>.db`):
it is held only as long as the publisher allows, it is hard to read off the disk, and it never leaves in a saved or exported file.
The reference does, so the text can be looked up again.

This applies to anything a plugin stores with a `licensed` block (today, the YouVersion plugin:
<https://github.com/Craterbrain/opensanctuary-youversion-plugin>). Bibles from the public-domain repository and the other Online
Bibles sources are ordinary Bibles and are not affected.

## What the host does

| Rule | How |
|---|---|
| **Kept only as long as the publisher says** | The plugin passes the lifetime the API gave (`Cache-Control: max-age` minus `Age`, returned by `net.fetch` as `cacheSecs`) as `licensed.ttl_secs`. The host stores `expires_at_ms`, clamps it to 7 days, and refuses text whose lifetime is 0. The NIV answers one hour; public-domain versions answer a day. |
| **Not readable from the files** | The verses are sealed with AES-256-GCM before they reach the database (`src/storage/db/licensed.rs`). The key is in the OS keyring, or in an owner-only file beside the database where there is none. A SQLite browser, a copy of `library.db`, its WAL file or a backup shows nothing readable. Text search does not see it. |
| **Gone when expired** | An expired passage is not served (even before it is deleted), is deleted at every start-up, and hourly while the app runs. `PRAGMA secure_delete` zeroes the freed pages. |
| **Never in a saved or exported schedule** | `Schedule::without_licensed_text` is applied to the event log (`ShowEvent::without_licensed_text`), `.ewsx` saves and downloads, `.osservice` bundles, and the console's browser auto-save (`web/src/core/licensed.ts`). Each licensed item keeps its `licensed` reference (plugin, Bible id, `JHN.3.16-18`, notice, `resolved: false`) and a one-line placeholder. |
| **Looked up again on open** | A schedule item with `licensed.resolved == false` makes the console ask the plugin named in it (`os.event 'licensed'`, see `plugin_bridge.ts`). The plugin fetches the passage, stores it (sealed, with a fresh lifetime) and answers with the `ResolveLicensed` command, which the host turns into `SetLicensedText`. The live and staged copies are updated too. Opening a file without that plugin shows a notice instead. |
| **Renewed while the app is open** | The plugin asks the publisher again shortly before a passage's time is up; the answer's body is thrown away and `library.refreshLicensed` renews the lifetime of the text already stored. If the publisher now says "do not keep", it is left to expire. Only the plugin that stored a passage can renew it. |
| **Notice on every slide** | The host makes the short notice (`NIV · © Biblica, Inc`) the last line of every licensed slide and part of its reference label. The full copyright text is stored with the passage (`licensed.copyright`). |

A text that is **on screen** has to be readable by the app, so none of this is DRM: it makes copying the text out of the
files or an export difficult and short-lived, as the licenses ask. An open schedule keeps showing its text until the app
closes, even if the stored copy expired, so a service never goes blank.

## What the licenses say (YouVersion Platform fast-track licenses, read 2026-10-26)

Nothing here is legal advice; it is what the agreements a developer accepts say, and why the code does what it does.

* **Biblica (NIV)**
  * Displayed to one user at a time: no more than **two chapters or twenty-five verses**, whichever is greater. The plugin looks up one chapter or less.
  * The copyright notice, and a one-step link to Biblica ("Biblica" in its text), must be shown with the text. The notice is on every slide; the link is in the plugin panel.
  * No bulk transfer of the text; electronic rights are text display only. The plugin never downloads a whole Bible or book.
  * Files containing the content are to be destroyed within 72 hours of the agreement ending.
* **Lockman (NASB, AMP)**
  * Display "must make downloading of a large portion or the entire work **difficult or impractical**" (the sealed, expiring store above).
  * The copyright notice, or the tag (`NASB 1995`, `AMP`...) after the verses, with a link to lockman.org.
  * Annual report of the **copies distributed**. Exports carry no text, so none are distributed.
* **Neither document** states a cache time; the lifetime is the `Cache-Control` the API sends with each response.

## For plugin authors

* `library.addScriptures` items may carry `licensed: { bible_id, usfm, tag, copyright, link, ttl_secs }`. The host sets the id (`lic_...`), the owner and the expiry; the reply has `expiresAtMs`.
* `library.refreshLicensed { ids, ttlSecs }` renews passages the plugin stored.
* `net.fetch` replies with `cacheSecs` (how much longer the response may be kept, `null` if it says nothing, `0` for no-store/private).
* A plugin that wants its passages looked up again on open needs `commands.send` for `ResolveLicensed` and handles `OS.on('licensed', ...)`.
* A passage is shown with `tag` as its last line; do not strip it.

## Where it lives

`src/storage/db/licensed.rs` (store, sealing, expiry) · `src/core/models.rs` (`LicensedRef`, `without_licensed_text`) ·
`src/core/engine/licensed.rs` (`SetLicensedText`) · `src/api/command_pipeline.rs` (`ResolveLicensed`) ·
`src/api/routes/plugins.rs` (the `licensed` block, `/api/plugin-host/scriptures/refresh`) · `web/src/core/plugin_bridge.ts` ·
tests: `licensed.rs` unit tests, `tests/core_tests/licensed.rs`, `web/tests/e2e_licensed_passages.test.ts`, `web/tests/licensed.test.ts`.
