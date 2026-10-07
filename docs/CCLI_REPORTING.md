# CCLI usage reporting

_Last edited: 2026-10-02 22:40_

CCLI requires churches to periodically report which copyrighted songs were
actually used in services, for royalty distribution. The CSV export,
due-date reminder, and the CCLI.com credential-storage/autofill-assist
follow-up (an embedded native-webview window that autofills CCLI's login and
upload fields without ever auto-submitting either) are all **implemented**
-- see "CCLI.com upload" below for the one real caveat: the autofill
selectors were written generically and haven't been verified against the
real, live CCLI.com, since their site blocks automated fetches from this
environment.

---

## Data source (already existed)

No new usage-tracking was needed. `event_log` (`src/storage/db/events.rs`)
already persists an `ItemWentLive` event with a timestamp every time a song
goes live, including the full item snapshot (title, author, per-slide
`CcliMetadata`/`ccli_number` — see `src/core/models.rs`). The report is a
query over existing data, not new instrumentation. Only counts songs that
actually **went live** — adding a song to a schedule and never presenting it
doesn't count, since that's exactly what distinguishes `ItemWentLive` from
`ItemStaged`.

## Public domain / out-of-copyright songs — **implemented**

CCLI doesn't require reporting on PD works. `is_public_domain` is derived
(`looks_public_domain` in `src/storage/db/ccli.rs`) from the song's
`CcliMetadata.copyright` (empty/absent, or contains "public domain") and no
`ccli_number` — a heuristic, same as planned. No manual override checkbox on
the song itself yet (open question below); for now a song can be corrected
by giving it a `CcliMetadata` entry (even a minimal one) via the slide
editor's notes pane, which flips the heuristic since a `ccli_number` alone
already overrides it.

The report view shows two sections, not a silent filter:
- **Reportable** — counted, included in the CSV.
- **Excluded — Public Domain** — visible, greyed out, never written to the
  CSV, so the operator can catch a wrong PD classification before exporting.

## CSV export — **implemented**

- **`src/storage/db/ccli.rs`**: `Database::get_ccli_usage_report(&self, start_ms: i64, end_ms: i64) -> Result<Vec<CcliUsageRow>>`
  — queries `event_log` for `ItemWentLive` events in range, filters to
  `item_type == "song"`, groups by `ccli_number` (fallback: lowercased
  title, for songs missing a number — flagged, not dropped), produces
  `{title, author, ccli_number: Option<String>, use_count, first_used_ms, last_used_ms, is_public_domain}`.
- **`GET /api/reports/ccli-csv?start=<ms>&end=<ms>`** (`src/api/routes/reports.rs`)
  — hand-built CSV (no new crate), `Content-Disposition: attachment`,
  `text/csv` — mirrors `export_schedule_ewsx`'s download pattern
  (`src/api/routes/schedule.rs`). Host-token-gated, like the sibling JSON
  route below — this surfaces song titles/authors/usage across an arbitrary
  date range, which an unauthenticated LAN caller has no business pulling.
- **`GET /api/reports/ccli-usage?start=<ms>&end=<ms>`** — the same report as
  JSON, for the report window UI's Reportable/Excluded-PD split below.
- **Columns**: `Title, Author, CCLI Song Number, Times Used, First Used, Last Used`
  — the shape CCLI's own manual entry/upload expects. Rows flagged Public
  Domain are excluded from the file entirely.

## Due-date reminder — **implemented**

CCLI reporting cadence (usually annual, sometimes semi-annual) is set by the
church's own license — the app can't know it without being told. A
`ccliReportingDueDate` setting (Settings > Integrations, `YYYY-MM-DD`,
format-validated server-side in `post_settings`) drives a reminder starting
30 days before due. Turned out the `web/live.html` `#alert-banner` this doc
originally pointed at is a *display-facing* nursery/alert mechanism
(`ShowState.alert_message`, shown on the live output screen), not a
console-side notification system — reusing it here would have put a CCLI
reminder on the sanctuary screen, not the operator's own console. Built on
the existing operator-console toast system (`showToast` in
`web/src/core/ui_utils.ts`) instead, with `duration: 0` so it persists until
dismissed rather than disappearing after a few seconds like a normal toast —
checked once per console load in `app_core.ts`, not only when Settings
happens to be open.

## Report window UI — **implemented**

A modal (`web/src/ui/ccli_report_modal.ts`, static markup in `web/index.html`
as `#ccli-report-modal`, following the same modal-controller pattern as
`arrangement_modal.ts`) with:
- Start/end date pickers (defaulting to Jan 1 of the current year through
  today), with a Refresh button.
- The Reportable / Excluded-PD split above, so the operator can review
  before exporting.
- "Export CSV" button.
- A **persistent sidebar**, visible the whole time the window is open,
  stating what CCLI expects: what counts as "used," the PD exclusion rule,
  the configured due date, and the current reporting period.

Opened from Settings > Integrations > "Open CCLI Report…".

## CCLI.com upload — **implemented**: auto-fill, don't auto-submit

CCLI has no public API for submitting usage reports, and driving their whole
login + navigation + submit flow programmatically (classic browser
automation) would have been the riskiest version of this idea — fragile
against their site changing, and it turns one login into an unattended
transaction. Built the narrower version instead, using the app's existing
native webview stack (tao/wry — `src/webview/mod.rs`) rather than an
external headless browser, with password-manager-style credential handling:

### Credential storage (password-manager style) — **implemented, via existing infrastructure**

The CCLI username/password are stored in the OS's own credential store —
macOS Keychain, Windows Credential Manager, Linux Secret Service — through
`KeyringService` (`src/storage/keyring.rs`), which turned out to already
exist as a fully generic `(service, account, secret)` store (built for
plugin/provider API keys, `keyring = "4.2"` was already a dependency) with
a matching frontend client (`web/src/core/keyring.ts`) that already had
`KEYRING_SERVICES.CCLI` and a typed `UserCredentials`/`setUserCredentials`
helper defined, unused until now. No new storage mechanism, no new route —
the CCLI Report modal's "CCLI.com Login" section just calls the existing
generic `POST /api/keyring/set` et al. under `OpenSanctuary:CCLI` /
`credentials`. Saving with a blank password keeps the previously-saved one
(so updating just the username doesn't require retyping the password).
If CCLI login fails (password changed, 2FA, etc.), nothing here detects or
retries that — the user just sees CCLI's own login page reject it and can
re-save a corrected password from the same modal.

### Autofill, in the embedded webview — **implemented**

- "Open Upload Assistant" (in the CCLI Report modal, next to "Export CSV")
  calls `POST /api/reports/ccli-open-assist`, which builds the CSV for the
  modal's currently-selected date range (the same `build_ccli_csv_text`
  the download route uses), reads the saved credentials (if any) from the
  keyring, and opens CCLI's reporting portal (`https://reporting.ccli.com/`)
  in its own native `wry::WebView` window
  (`DisplayEvent::OpenExternal` in `src/webview/mod.rs` — a new kind of
  window, since every other window in this app loads one of OS-Next's own
  served pages) with an autofill script baked into it as a
  `with_initialization_script` (`src/api/routes/reports.rs`'s
  `CCLI_ASSIST_SCRIPT_TEMPLATE`).
- That script re-runs on **every page load within the window**, not just the
  first — `with_initialization_script` is per-navigation, not scoped to the
  window's initial URL, which is exactly the mechanism needed to catch both
  the login page and, later, the upload page without any extra wiring.
- Login: finds the page's `input[type="password"]` (the one reliable
  signal regardless of CCLI's actual field naming, which couldn't be
  verified from here — CCLI's site blocks automated fetches) plus a
  best-guess username field (`autocomplete`/`type="email"`/name-or-id
  containing "user"/"email"), sets both via the real native value setter
  plus `input`/`change` events (the same DOM-level technique real
  password-manager browser extensions use), and stops. **The user still
  clicks CCLI's own Log In button themselves.**
- Upload: finds the page's `input[type="file"]`, constructs a `File` from
  the already-generated CSV via `DataTransfer`, assigns it, and dispatches
  `change` — exactly as if the user had browsed to and selected the file
  themselves. **The user still clicks CCLI's own Upload/Submit button
  themselves.** The script never calls `.submit()` or `.click()` on
  anything; a unit test (`assist_script_never_calls_submit_or_click`)
  guards against that regressing.
- If a field can't be found (CCLI's real markup doesn't match the generic
  selectors above), the script simply does nothing on that page rather than
  guessing wrong — there's no visible in-page fallback message, but nothing
  breaks either; the operator just falls back to typing/browsing manually,
  same as without this feature at all.

This is the version that got built: credentials stored the way a password
manager stores them (OS keychain, not plaintext), fields autofilled the way
a password manager autofills them (DOM value + events, no scripted
navigation), and no auto-submit anywhere in the flow — the user still
clicks both Log In and Upload themselves. **Caveat, stated plainly:** the
exact field-detection selectors were written generically because CCLI's
real login/upload page DOM couldn't be inspected from here (their site
returns 403 to automated fetches) — this hasn't been verified against the
real, live CCLI.com. If CCLI's actual markup doesn't match the generic
`type="password"`/`type="file"` detection this relies on, autofill simply
won't trigger (degrades to "nothing happens," not "something breaks"), and
the selectors in `CCLI_ASSIST_SCRIPT_TEMPLATE` would need adjusting by
whoever first tries this against a real account.

## Open questions

- Exact heuristic thresholds for "public domain" detection — copyright
  string matching alone may need a curated list of known-PD phrases.
- A manual PD-override checkbox directly on the song (rather than only via
  giving it a `CcliMetadata` entry) — would make correcting a misfire more
  discoverable.
- **Whether CCLI's login/upload page DOM is stable enough to target
  reliably — genuinely still unresolved**, and unlike the others on this
  list, not resolvable from here at all: CCLI's site returns 403 to
  automated fetches, so the autofill script's selectors (generic
  `type="password"`/`type="file"`/`autocomplete` matching, documented above)
  were written without ever seeing the real page. They may just work (both
  are extremely common, nearly-universal markup patterns), or may need
  adjusting once someone tries this against a real CCLI account. Either
  way, a selector miss degrades to "autofill doesn't trigger," not a
  crash or a wrong-field injection — see the "no visible in-page fallback
  message" note above for the one piece of the originally-sketched
  "if we can't find the field, show the file path" fallback that didn't
  get built (there's no in-page error surfaced; the operator just falls
  back to doing it manually, silently).

~~Where the reminder banner should surface and how far in advance~~ —
resolved: a persistent operator-console toast, 30 days out (see "Due-date
reminder" above).

~~Whether the embedded-browser upload-assist window ships as a later
phase~~ — resolved: built in the same pass as this update, now that the CSV
export (shipped separately, earlier) was confirmed working first.
