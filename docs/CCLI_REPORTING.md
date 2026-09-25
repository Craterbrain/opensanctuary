# CCLI usage reporting (design doc, not yet built)

CCLI requires churches to periodically report which copyrighted songs were
actually used in services, for royalty distribution. This covers exporting
that usage as a CSV, reminding the operator when it's due, and — as a
separate, more speculative follow-up — an embedded-browser window that
stores the operator's CCLI.com credentials the way a password manager does
(OS keychain, not app storage) and auto-fills both the login form and the
CSV upload field the moment they appear, without ever auto-submitting the
login or the report on the user's behalf.

---

## Data source (already exists)

No new usage-tracking is needed. `event_log` (`src/storage/db.rs`) already
persists an `ItemWentLive` event with a timestamp every time a song goes
live, including the full item snapshot (title, author, `CcliMetadata`/
`ccli_number` — see `src/core/models.rs`). The report is a new query over
existing data, not new instrumentation. Only counts songs that actually
**went live** — adding a song to a schedule and never presenting it doesn't
count.

## Public domain / out-of-copyright songs

CCLI doesn't require reporting on PD works. Derive `is_public_domain` from
`Song.copyright` (empty, or contains "public domain") and no `ccli_number` —
a heuristic, so pair it with a manual override checkbox on the song, since
heuristics will misfire both ways.

The report view shows two sections, not a silent filter:
- **Reportable** — counted, included in the CSV.
- **Excluded — Public Domain** — visible, greyed out, never written to the
  CSV, so the operator can catch a wrong PD classification before exporting.

## CSV export

- **`src/storage/db.rs`**: `get_ccli_usage_report(&self, start_ms: i64, end_ms: i64) -> Result<Vec<CcliUsageRow>>`
  — query `event_log` for `ItemWentLive` events in range, filter to
  `item_type == "song"`, group by `ccli_number` (fallback: lowercased title,
  for songs missing a number — flagged, not dropped), produce
  `{title, author, ccli_number: Option<String>, use_count, first_used, last_used, is_public_domain}`.
- **New route** `GET /api/reports/ccli-csv?start=<ms>&end=<ms>` — hand-built
  CSV (no new crate needed for something this simple), `Content-Disposition:
  attachment`, `text/csv` — mirrors the existing `export_schedule_ewsx`
  handler's download pattern (`src/api/routes.rs`).
- **Columns**: `Title, Author, CCLI Song Number, Times Used, First Used, Last Used`
  — the shape CCLI's own manual entry/upload expects.

## Due-date reminder

CCLI reporting cadence (usually annual, sometimes semi-annual) is set by the
church's own license — the app can't know it without being told. Add a
`ccliReportingDueDate` setting (or a recurring interval), and surface a
banner/toast starting N days before due, reusing the existing alert-banner
pattern (`web/live.html`'s `#alert-banner` / the Settings > Alerts category).

## Report window UI

A modal (or dedicated window) with:
- Start/end date pickers, defaulting to the current reporting period.
- The Reportable / Excluded-PD split above, so the operator can review
  before exporting.
- "Export CSV" button.
- A **persistent sidebar**, visible the whole time the window is open,
  stating what CCLI expects: what counts as "used," the PD exclusion rule,
  where the due date comes from, and the current reporting period.

## CCLI.com upload — narrowed design: auto-fill, don't auto-submit

CCLI has no public API for submitting usage reports, and driving their whole
login + navigation + submit flow programmatically (classic browser
automation) is the riskiest version of this idea — fragile against their
site changing, and it turns one login into an unattended transaction.

Narrower version, using the app's existing native webview stack (tao/wry —
see `src/webview/mod.rs`) instead of an external headless browser, with
password-manager-style credential handling rather than no storage at all:

### Credential storage (password-manager style)

- Store the CCLI username/password in the OS's own credential store — macOS
  Keychain, Windows Credential Manager, Linux Secret Service — via a crate
  like `keyring`, not a row in the app's plaintext `settings` table. This is
  the same trust model a real password manager uses: the OS, not the app,
  guards the secret at rest.
- A small Settings section to save/update it: username field, masked
  password field, "Save" — decrypted only in-memory, only for as long as
  it takes to inject it into the login form.
- If CCLI login fails (password changed, 2FA, etc.), surface that plainly
  and let the user re-enter and re-save — don't retry silently.

### Autofill, in the embedded webview

- Open CCLI's login page in one of our own `wry::WebView` windows.
- Inject a script (`wry` supports runtime script evaluation) that finds the
  username/password fields on the page and sets their values — the same
  DOM-level technique real password-manager browser extensions use (set the
  value, then dispatch `input`/`change` so CCLI's own page JS sees it as a
  real entry, not just a `.value =` no framework will notice).
- The user still clicks CCLI's own **Log In** button themselves. We fill the
  fields; we don't submit the form.
- Once logged in, the user navigates to the upload step themselves, same as
  today. We're not scripting navigation between pages — only reacting once
  they get to a page we recognize (login fields, then later the upload
  field).
- Detect the upload moment by injecting a small script into the page (`wry`
  supports runtime script evaluation) that watches for a file-input element
  appearing on the page (a `MutationObserver`, or a check on navigation/load
  since CCLI's upload page is presumably a known, stable URL). The moment
  it's present, hand it our already-generated CSV directly — construct a
  `File` from the CSV bytes and assign it to the input via `DataTransfer`,
  then dispatch a `change` event so CCLI's own page code picks it up exactly
  as if the user had browsed to and selected the file manually.
- Stop there. We fill the field; **the user still clicks CCLI's own Submit
  button**. We never complete the transaction on their behalf. This keeps a
  human confirmation step in front of anything actually being submitted to
  CCLI, and meaningfully lowers both the reliability risk (only one DOM
  interaction to keep working, not a whole scripted flow) and the ToS
  concern (assisting a manual upload the user completes, not an unattended
  submission).

This is the version worth building: credentials stored the way a password
manager stores them (OS keychain, not our own plaintext), fields
autofilled the way a password manager autofills them (DOM value + events,
no scripted navigation), and no auto-submit anywhere in the flow — the user
still clicks both Log In and Upload themselves. The app removes the two
annoying manual steps (typing the password, finding where the CSV was
saved) without ever completing a transaction on the user's behalf.

## Open questions

- Exact heuristic thresholds for "public domain" detection — copyright
  string matching alone may need a curated list of known-PD phrases.
- Where the reminder banner should surface (operator console only, or also
  a startup check) and how far in advance.
- Whether the embedded-browser upload-assist window ships alongside CSV
  export or as a later phase.
- Whether CCLI's upload page URL/DOM is stable enough to target reliably, or
  needs a "if we can't find the field, just show the file path" fallback.
