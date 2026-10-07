# Plan: first-time setup

_Last edited: 2026-10-02 20:05_

## Why this matters more than it looks like

Per the public-repo decision (`bibles/` and copyrighted `songs/` excluded,
see the repo's `.gitignore`), a fresh install today has **zero Bible
content and almost no song content** — only `songs/public_domain.db`'s
handful of bundled hymns. Without a first-run flow that actively helps the
user get *some* content in, "add a scripture item" is a dead end the
first time anyone opens the app. This is the most important gap this plan
needs to close, not just an onboarding nicety.

## Trigger condition

No separate "first run" flag file. The signal already exists implicitly:
`Database::new()`/`new_with_dirs()` creates `library.db` if it isn't there.
Thread that through explicitly — `Database::was_freshly_created(&self) ->
bool` — and expose it from a new `GET /api/first-run-status` route (or fold
into the existing status/settings endpoint). The frontend checks this once
at boot (`app_core.ts` init sequence) and shows the setup overlay if true.

Render as an **overlay on top of the normal console**, not a blocking
full-screen takeover with nothing behind it — a user who dismisses it
without finishing shouldn't be stuck. Add a "Re-run First-Time Setup"
action under Settings → About so a church can redo it later (new machine,
starting over, etc.) — trivial once the modal exists, meaningful for
support.

## Steps

1. **Welcome.** One screen, plain-language intro, "Get Started."

2. **Data directory — implemented.** Shows the auto-detected default
   (`paths.md`). Also the point where legacy-layout migration is offered:
   `paths::detect_legacy_portable_library` checks CWD and the exe's own
   directory for an older portable install's `library.db`; if found, a
   banner offers "Use This Library Instead" rather than silently starting
   fresh. Accepting points `install.json` at the legacy directory (no file
   copy) and asks for a restart. Picking a different folder for the *new*
   database before anything is written there (the originally-sketched
   alternative to this step) is not built — only the legacy-detection path
   is.

3. **Church / Sanctuary name.** Feeds the existing `churchName` setting
   (already wired to the live-output footer and mDNS advertisement — no
   new plumbing needed, just prompting for it here instead of leaving it
   to be discovered in Settings).

4. **Bible content.** The load-bearing step (see above). Three options
   presented together, not sequential forced choices:
   - Fetch a public-domain translation now via the existing
     `BibleProviderRegistry` (KJV/ASV/WEB) — one click, works offline
     afterward.
   - Point at existing Bible `.db` files (someone with a prior install, or
     files obtained separately) — sets `biblesDirectory`
     (`paths.md`'s Settings entry).
   - Skip — but land back on this exact step from Settings, not just a
     buried settings toggle, since skipping it is easy to forget about.

5. **Song content.** Lower-stakes than Bible content since
   `public_domain.db` ships bundled — offer pointing at an existing songs
   directory, or skip.

6. **Network quick-check.** Show the detected hostname/LAN IP(s) the
   console will be reachable at, and one line each explaining Live Output,
   Stage Foldback, and Remote Control URLs. This is genuinely non-obvious
   to a first-time operator and currently only discoverable by reading the
   console's startup log output — worth surfacing once, here.

7. **Done.** Drop into the normal console; a brief toast ("Setup complete
   — revisit anytime in Settings") rather than a modal that needs
   dismissing.

## Where implemented

- `web/src/ui/first_time_setup.ts` (new), following the existing modal
  patterns in `web/src/ui/` (e.g. `settings_dialog.ts`,
  `theme_picker.ts`).
- Rust: `was_freshly_created()` on `Database`, the status route, and
  whatever `paths.md`'s data-dir-move / `install.json` write needs on the
  Rust side for step 2.
- No new settings-schema entries required beyond what `paths.md` already
  adds (`storage` category) and what already exists (`churchName`) — this
  is a guided *flow* through existing/planned settings, not new state.
