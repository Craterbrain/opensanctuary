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

## Translations in the sidebar, searched by reference

A plugin can offer translations without storing them (`library.setRemoteBibles`; metadata only, `src/storage/db/remote_bibles.rs`).
The Scriptures sidebar lists them with the plugin's tag ("YouVersion") next to the Bibles stored here.

* **Search** is the ordinary scripture search bar. A reference ("John 3:16", "Ps 23:1-3") lists one result per translation in scope
  (All Translations, or the one selected): the reference and the tag, **no text**. A text search skips them.
* **Preview** (a single click) says how to load it instead of fetching anything.
* **Add or drag to the schedule** (`AddToSchedule` with an id `yvref:<plugin>:<bible id>:<book>:<chapter>:<from>:<to>`) makes a
  placeholder item at once (`LicensedRef.resolved == false`, one slide per verse asked for, or one for a whole chapter).
  The console's `watchLicensed` then asks the plugin, which fetches the passage, stores it sealed with the publisher's lifetime and
  answers `ResolveLicensed`; the host rebuilds the item's slides from the verses that came back (`SetLicensedPassage`,
  `ScheduleItem::fill_licensed`), with the notice on every slide. Dropped before the text arrives, the placeholder is exactly what a
  saved or exported schedule would hold, so a reopened schedule fills in the same way.
* The default Bible and Bible export only ever offer Bibles stored here, never these translations.
* The YouVersion plugin keeps its own list of added translations (its panel: "Add a translation") and enforces each publisher's
  limit (below) when it looks a passage up; a single chapter (what a search result names) is always within it.

## The key release builds carry

Release builds include a YouVersion App Key, so the plugin works without the church registering anything.

* **Where it lives:** the GitHub Actions secret `YVP_APP_KEY` on `Craterbrain/opensanctuary`. It is never in the repository, the plugin repository or the plugin's zip. `release.yml` passes it to the Linux (container) and Windows builds; pull-request and local builds have none unless `YVP_APP_KEY` is set in the environment, and the plugin then asks for the church's own key.
* **How it is stored:** `build.rs` masks it (XOR with a SHA-256 keystream from a random per-build pad, `src/builtin_key_mask.rs`) into the binary, so `strings`, a text search of an installer or zip, or a git clone does not find it. `packaging/check-key-not-plain.sh` runs after the release builds and fails the release if the key is readable in a `.deb`, tarball or portable zip.
* **How it is used:** the *server* adds `X-YVP-App-Key` to the YouVersion plugin's requests to `api.youversion.com` (`src/api/builtin_key.rs`), and only to that plugin and that host. A key the church saved in the plugin is sent instead. The plugin, its page and its keyring never see the built-in key.
* **What this is not:** secrecy. The app has to send the key to YouVersion, so anyone running it can capture the key from the network. If it is abused, rotate it in the YouVersion portal and ship an update (the old builds then ask for the church's own key). YouVersion measures use per app key itself; there is nothing to report to it.
* **Who the licensee is:** with a built-in key, OpenSanctuary (not each church) is the developer the Biblica and Lockman agreements are with. See the next section.

To make a build with the key locally: `env YVP_APP_KEY="$(cat ~/.config/opensanctuary-dev/youversion_app_key)" cargo build --release`.

## What the licenses say (YouVersion Platform fast-track licenses, read 2026-10-26)

Nothing here is legal advice; it is what the agreements a developer accepts say, and why the code does what it does.

* **Biblica (NIV)**
  * Displayed to one user at a time: no more than **two chapters or twenty-five verses**, whichever is greater. The plugin allows exactly that for the NIV family (publisher id `05a9aa40-...`): up to two chapters of any length, or more chapters while the total stays within 25 verses (it stops fetching as soon as it passes that).
  * The copyright notice, and a one-step link to Biblica ("Biblica" in its text), must be shown with the text. The notice is on every slide; the link is in the plugin panel and, since 2026-10-27, next to the passage's title in the console's Preview and Live panels (`web/src/ui/licensed_link.ts`; it opens the publisher's page through `POST /api/system/open-url`, which only opens https pages on biblica.com, lockman.org, bible.com and youversion.com). A projected slide can't be clicked, so the slide itself carries only the notice.
  * No bulk transfer of the text; electronic rights are text display only. The plugin never downloads a whole Bible. A book named on its own is looked up whole only when it fits the publisher's limit (for the NIV: Obadiah, Philemon, 2 John, 3 John, Jude and Haggai), and a lookup across chapters is one call per chapter.
  * Files containing the content are to be destroyed within 72 hours of the agreement ending.
* **Any other publisher (and Lockman's NASB and AMP, whose terms give no number):** the plugin allows a loose **five chapters** at a time.
* **Lockman (NASB, AMP)**
  * Display "must make downloading of a large portion or the entire work **difficult or impractical**" (the sealed, expiring store above).
  * The copyright notice, or the tag (`NASB 1995`, `AMP`...) after the verses, with a link to lockman.org.
  * Annual report of the **copies distributed**. Exports carry no text, so none are distributed.
* **Cache time:** the licenses leave it to the API: the lifetime is the `Cache-Control` the API sends with each response, which is what the host follows.
* **Projection:** neither license names projecting text in church presentation software. The project owner's position (2026-10-26) is that it is valid under both YouVersion's terms and the publishers' fair-use allowances (displaying a passage to a congregation, a chapter or less at a time); the limits above (at most a chapter, notice on every slide, nothing kept or exported) are what that position relies on. This is a decision, not a confirmation from YouVersion or the publishers.

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
