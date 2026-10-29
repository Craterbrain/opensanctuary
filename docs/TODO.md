# To-do

_Last edited: 2026-10-29_

What hasn't been done yet. Each item says where it came from, so the detail lives in one place. Remove an item when it ships.

## Planned features

- [ ] **Planning Center import plugin.** Pull a church's own songs, arrangements and service plans from the [Planning Center Services API](https://developer.planning.center/) (open, OAuth) into the library or a schedule, matched by CCLI number. This brings in content a church already owns; it is not a store. Needs a design pass first: auth flow (OAuth vs. personal access token), what maps to a `Song` arrangement, and rate limits. Build it as a plugin, not core.
- [ ] **Decide the Store dialog.** The Store button is hidden for now (`hidden` on `#btn-store` in `web/index.html`; the dialog is intact). The SongSelect card is being removed (PR #56). Remaining "Coming Soon" card: Motion Background Packs. Options: keep the dialog as a list of ways to bring content in (public-domain Bibles, importing files, plugin hints), or drop it. No store API exists to build on: SongSelect's API is partner-only, and media vendors sell through their own sites.

## Licensed scripture (see `LICENSED_SCRIPTURE.md`)

- [ ] **Rotate the YouVersion key before 1.0.** The `YVP_APP_KEY` secret is currently the development key (it was pasted in a chat). Before 1.0: create a separate production app in the YouVersion Platform portal, accept the licenses on it, put its key in the secret (`gh secret set YVP_APP_KEY -R Craterbrain/opensanctuary`), and retire the development key. Also rotate it any time it is thought to have leaked.

## Unverified (built, never tested for real)

- [ ] **CCLI.com upload autofill.** The selectors were written without ever seeing the live site (it returns 403 to automated fetches). Try it with a real CCLI account. See `CCLI_REPORTING.md`.
- [ ] **OBS on another machine with a self-signed certificate.** The two workarounds (Caddy in front, or `--ignore-certificate-errors` for OBS's browser) are not verified against a current OBS. See `BROADCASTING.md`.
- [ ] **Android TV ADB setup on more TV models.** Only tried on the models available so far; the "home app" switch and its Restore button may behave differently elsewhere. Release 0.3.4 notes ask people to try it on one TV first.
- [ ] **In-app update between two Flatpak releases.** The Flatpak bundle builds and installs, but updating from one `.flatpak` to the next hasn't been exercised. See `installer.md`.

## Bible import

- [ ] **More formats** for *Bible from a Link or File*: SWORD modules (`docs/bible-import.md`).
- [ ] **Copyright note per source:** a few well-known sources (ebible.org, CrossWire) could show their licence text before installing.

## Small improvements

- [ ] **The last 95 explicit `any` in the web UI.** The API-boundary types are in `core/models.ts` and most of the 368 are gone; what is left (plugin boundary, request bodies, the live-output sync engine, debug mirrors) is listed in `TYPECHECKING.md`.
- [ ] **Run a short fuzz pass in CI** as a regression gate (e.g. 60 seconds per target), or leave it manual. Undecided. See `FUZZING.md`.
- [ ] **Importer fuzz targets share one media folder,** so long runs grow the disk. Revisit if it bites. See `FUZZING.md`.
- [ ] **Live and staged items are copied whole on every small edit.** An `Arc` around them would avoid it. Noted, not implemented.
- [ ] **Overlay layout.** The bulk-paste overlay covers part of the Speaker Notes / CCLI pane.
- [ ] **Flaky test.** The rich-text end-to-end test fails occasionally under load.

## Decisions waiting on the owner

- [ ] **Plugin signing:** how an author publishes a first key so people recognize it. See `PLUGIN_SIGNING.md` section 12.
- [ ] **Sign commits and tags** with an SSH key so the history shows GitHub's "Verified" badge. The how-to is written (`SIGNING.md`); it needs a key created and set up on the maintainer's machine and GitHub account (steps 1-3), then optionally the "require signed commits" rule.
- [ ] **Glib dependency alert.** Dismissed as tolerable risk; revisit when `wry` / `tao` next bump.
