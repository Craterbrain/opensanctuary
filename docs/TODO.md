# To-do

_Last edited: 2026-10-25_

What hasn't been done yet. Each item says where it came from, so the detail lives in one place. Remove an item when it ships.

## Planned features

- [ ] **Planning Center import plugin.** Pull a church's own songs, arrangements and service plans from the [Planning Center Services API](https://developer.planning.center/) (open, OAuth) into the library or a schedule, matched by CCLI number. This brings in content a church already owns; it is not a store. Needs a design pass first: auth flow (OAuth vs. personal access token), what maps to a `Song` arrangement, and rate limits. Build it as a plugin, not core.
- [ ] **Decide the Store dialog.** The SongSelect card is being removed (PR #56). Remaining "Coming Soon" card: Motion Background Packs. Options: keep the dialog as a list of ways to bring content in (public-domain Bibles, importing files, plugin hints), or drop it. No store API exists to build on: SongSelect's API is partner-only, and media vendors sell through their own sites.

## Licensed scripture (see `LICENSED_SCRIPTURE.md`)

- [ ] **Biblica's link:** the NIV license wants a one-step link to Biblica whenever the text is shown. The plugin panel shows the YouVersion link; the projected slide carries only the notice.
- [ ] **Rotate the YouVersion key before 1.0.** The `YVP_APP_KEY` secret is currently the development key (it was pasted in a chat). Before 1.0: create a separate production app in the YouVersion Platform portal, accept the licenses on it, put its key in the secret (`gh secret set YVP_APP_KEY -R Craterbrain/opensanctuary`), and retire the development key. Also rotate it any time it is thought to have leaked.
- [ ] **Delete a licensed passage by hand** (a "clear kept passages now" button in the plugin).

## Unverified (built, never tested for real)

- [ ] **CCLI.com upload autofill.** The selectors were written without ever seeing the live site (it returns 403 to automated fetches). Try it with a real CCLI account. See `CCLI_REPORTING.md`.
- [ ] **System-wide Linux install (`--system`).** Needs real root; only the per-user path has been tested end to end. See `installer.md`.
- [ ] **OBS on another machine with a self-signed certificate.** The two workarounds (Caddy in front, or `--ignore-certificate-errors` for OBS's browser) are not verified against a current OBS. See `BROADCASTING.md`.
- [ ] **Android TV ADB setup on more TV models.** Only tried on the models available so far; the "home app" switch and its Restore button may behave differently elsewhere. Release 0.3.4 notes ask people to try it on one TV first.
- [ ] **In-app update between two Flatpak releases.** The Flatpak bundle builds and installs, but updating from one `.flatpak` to the next hasn't been exercised. See `installer.md`.
- [ ] **Bundle files on real installers.** `.ossong`/`.ospres`/`.osbible`/`.osservice` are covered by unit, HTTP, e2e and fuzz tests, but haven't been opened on a Windows install or the Flatpak. See `file-formats.md`.

## Bible import

- [ ] **More formats** for *Bible from a Link or File*: SWORD modules (`docs/bible-import.md`).
- [ ] **Copyright note per source:** a few well-known sources (ebible.org, CrossWire) could show their licence text before installing.

## Small improvements

- [ ] **Manual "public domain" override on a song.** Today a song is corrected by adding a `CcliMetadata` entry in the notes pane. A checkbox on the song would be easier to find. See `CCLI_REPORTING.md`.
- [ ] **Better public-domain detection.** The copyright-string heuristic may need a curated list of known public-domain phrases. See `CCLI_REPORTING.md`. The bundle copyright pop-up uses the same test.
- [ ] **CCLI autofill fallback message.** When the autofill can't find a field it silently does nothing; show the file path instead. See `CCLI_REPORTING.md`.
- [ ] **Blocking plugin hooks stall a server worker.** Run them with `spawn_blocking` and a timeout. See `PLUGIN_PERMISSIONS.md`.
- [ ] **Console that only uses the HTTP fallback never releases the console lock** when it goes away (the WebSocket drop is what releases it). Accepted for now. See `CLIENT_PAIRING.md`.
- [ ] **Turn on stricter TypeScript flags,** `noImplicitAny` first. See `TYPECHECKING.md`.
- [ ] **Run a short fuzz pass in CI** as a regression gate (e.g. 60 seconds per target), or leave it manual. Undecided. See `FUZZING.md`.
- [ ] **Importer fuzz targets share one media folder,** so long runs grow the disk. Revisit if it bites. See `FUZZING.md`.
- [ ] **Live and staged items are copied whole on every small edit.** An `Arc` around them would avoid it. Noted, not implemented.
- [ ] **Overlay layout.** The bulk-paste overlay covers part of the Speaker Notes / CCLI pane.
- [ ] **Flaky test.** The rich-text end-to-end test fails occasionally under load.

## Decisions waiting on the owner

- [ ] **Plugin signing:** how an author publishes a first key so people recognize it. See `PLUGIN_SIGNING.md` section 12.
- [ ] **Sign commits and tags** with an SSH key so the history shows GitHub's "Verified" badge. Release files are already signed with minisign; this would cover the source history.
- [ ] **Glib dependency alert.** Dismissed as tolerable risk; revisit when `wry` / `tao` next bump.
