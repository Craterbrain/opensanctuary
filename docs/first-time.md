# First-time setup

_Last edited: 2026-10-22_

Status: **implemented** — all 7 steps below are built (`web/src/ui/first_time_setup.ts`).
What follows is the original plan, kept because it's still the accurate rationale and
step list; each step notes where the shipped version matched or diverged from it.

## Why this matters more than it looks like

Per the public-repo decision (`bibles/` and copyrighted `songs/` excluded,
see the repo's `.gitignore`), a fresh install today has **zero Bible
content** — songs are covered: the public-domain hymn library is compiled into
the program and a brand-new library gets it on first run (`docs/public-domain-songs.md`). Without a first-run flow that actively helps the
user get *some* content in, "add a scripture item" is a dead end the
first time anyone opens the app. This is the most important gap this plan
needed to close, not just an onboarding nicety.

## Trigger condition — implemented, differently than planned

Not `Database::was_freshly_created()` as originally sketched (that would stay
true for a process's whole lifetime, re-showing the wizard on every reload
mid-setup *and* on every later restart until the operator finished it in one
sitting). Instead, `GET /api/server-info` (`src/api/routes/show.rs`) returns
`"first_run": !skip_first_time_setup && !settings.contains_key("firstTimeSetupCompleted")`
— a persisted settings flag the wizard sets on completion or skip
(`firstTimeSetupCompleted`), so a mid-setup reload keeps showing it (expected)
but finishing or dismissing it suppresses it for good, across restarts too. A
`--skip-first-time-setup` CLI flag forces it off (used by the e2e test suite).
`maybeShowFirstTimeSetup()` (`app_core.ts` init sequence) checks this once at
boot.

Renders as an **overlay on top of the normal console**, not a blocking
full-screen takeover — dismissing it early doesn't strand the user. "Re-run
First-Time Setup" lives under Settings → About, per the original plan.

## Steps

1. **Welcome.** One screen, plain-language intro, "Let's Go ➜."

2. **Data directory.** Shows the auto-detected default (`paths.md`). Also the
   point where legacy-layout migration is offered:
   `paths::detect_legacy_portable_library` checks CWD and the exe's own
   directory for an older portable install's `library.db`; if found, a
   banner offers "Use This Library Instead" rather than silently starting
   fresh. Accepting points `install.json` at the legacy directory (no file
   copy) and asks for a restart. Picking a different folder for the *new*
   database before anything is written there (the originally-sketched
   alternative to this step) was never built — only the legacy-detection path
   is.

3. **Church / Sanctuary name.** Feeds the existing `churchName` setting
   (already wired to the live-output footer and mDNS advertisement — no
   new plumbing needed, just prompting for it here instead of leaving it
   to be discovered in Settings).

4. **Bible content.** The load-bearing step (see above). Three options
   presented together, not sequential forced choices:
   - Turn on the **Online Bibles** plugin (one click) and download a
     translation from its panel — works offline afterward. Its list starts
     with 64 public-domain translations the project hosts itself
     (`Craterbrain/opensanctuary-bibles`: KJV, WEB, ASV, BSB, Reina-Valera
     1909, Louis Segond, Luther 1912 and more, in 30 languages), so there is
     always something to install. An empty Scriptures tab has a **Get
     Bibles** button that opens the plugin (or its review card, if it isn't
     turned on yet).
   - Point at existing Bible `.db` files (someone with a prior install, or
     files obtained separately) — sets `biblesDirectory`
     (`paths.md`'s Settings entry).
   - Skip — but land back on this exact step from Settings, not just a
     buried settings toggle, since skipping it is easy to forget about.

5. **Song content.** Lower-stakes than Bible content since
   the public-domain hymn library is built in and added on first run — offer pointing at an existing songs
   directory, or skip.

6. **Network quick-check.** Shows the detected hostname/LAN IP(s) the
   console is reachable at.

7. **Done.** Drop into the normal console with a toast, rather than a modal
   that needs dismissing.

## Where implemented

- `web/src/ui/first_time_setup.ts`, following the existing modal patterns in
  `web/src/ui/` (e.g. `settings_dialog.ts`).
- Rust: the `first_run`/`legacy_library_detected` fields on `GET
  /api/server-info` (`src/api/routes/show.rs`), `paths::detect_legacy_portable_library`
  (`src/storage/paths.rs`), `--skip-first-time-setup` (`src/main.rs`).
- No new settings-schema entries beyond what `paths.md` already adds
  (`storage` category) and what already existed (`churchName`) — this is a
  guided *flow* through existing settings, not new state.
