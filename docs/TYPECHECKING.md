# TypeScript type checking (2026-09-30)

_Last edited: 2026-09-30 09:49_

`web/` had no `tsconfig.json` and no `tsc` invocation anywhere before this —
`bun build` transpiles TypeScript (strips types) without ever checking them,
so every `as any`, every loosely-typed DOM lookup, and any real type error
was silently unchecked. This adds a real `tsc --noEmit` gate.

## What's wired where

- `web/tsconfig.json` — `moduleResolution: "Bundler"`,
  `allowImportingTsExtensions: true` (this codebase imports with explicit
  `.ts` extensions), `strict: false` (see below), `noEmit: true`.
- `web/package.json`: new `"typecheck": "tsc --noEmit"` script, and `"build"`
  now runs it first (`bun run typecheck && bun build ...`) — a type error
  fails the whole `build` script before any bundler even runs.
- **No separate CI step needed**: this repo has no `.github/` workflows.
  `build.rs` already runs `bun run build` on every `cargo build`/`cargo run`
  (panicking, with stdout/stderr captured, on a non-zero exit — see its own
  doc comment), and `web/tests/_pretest_build.ts` already runs `bun run
  build` before every `bun test`. Both inherited the typecheck gate for
  free by it living inside `build`, with zero changes to either file.
  Verified directly: a deliberately-injected type error in `main.ts` failed
  `bun run build` with the real `tsc` error surfaced, then reverted; a real
  `cargo check` after deleting `web/dist/` rebuilt it cleanly end to end.

## Why `strict: false`

Turning on `strict` cold, with zero prior type-checking history, would
likely produce a very large `noImplicitAny`-driven error count unrelated to
any real bug (this codebase's whole style is untyped `function foo(a, b) {}`
declarations). `strict: false` gets meaningful checking (wrong property
access, wrong argument count, wrong assignment type, unresolved names)
without demanding a full-codebase retype as a prerequisite. Turning on
individual strict flags later (`noImplicitAny` first, most likely) is a
reasonable follow-up once the codebase's typing habits catch up — not done
here.

## Getting to a clean baseline (191 → 0 errors)

Turning it on for the first time surfaced 191 errors. Investigated all of
them rather than blanket-suppressing:

- **One genuine, currently-broken bug**: `web/src/editor/slide_editor.ts`
  called `this.canvas.selection.selectMultiple(ids)` at 3 call sites
  (ungroup, paste, select-all) — `SelectionManager`
  (`web/src/editor/selection.ts`) has never had a `selectMultiple` method,
  only `select(id, multi)` and `setSelection(ids)`. Every one of these 3
  code paths would throw `TypeError: ... is not a function` today. Fixed by
  calling the existing `setSelection(ids)`, which does exactly what these
  call sites want.
- **~168 errors, one root cause**: DOM lookups typed as the generic
  `Element`/`HTMLElement` (or left to `document.getElementById`'s default
  return type) used with subtype-specific properties (`.currentTime`,
  `.value`, `.dataset`, `.style`, `.checked`, `.disabled`, etc.) that only
  exist on `HTMLVideoElement`/`HTMLInputElement`/etc. All were legitimate,
  runtime-correct code — fixed by adding the accurate type
  (`as HTMLVideoElement | null`, `querySelectorAll<HTMLElement>`, casting
  `event.target`, etc.) at each declaration site, not by changing behavior.
- **A handful of real, minor type-accuracy gaps**, each a one-line fix:
  `ThemeDefinition` had no `id` field even though backend-loaded themes
  carry one (`DEFAULT_THEMES` entries legitimately don't — now `id?: string`);
  `broadcastLocalMediaSync`'s `executeAtEpoch` parameter wasn't marked
  optional even though 5 of its 6 call sites never pass it (matches its own
  `MediaSyncManager.broadcastSync`, which already had it as optional);
  a couple of `input.value = <number>` assignments (DOM auto-coerces this
  fine at runtime, `tsc` doesn't accept it without `String(...)`); a stray
  `start_arrow`/`end_arrow: string` in `slide_render.ts`'s `RenderableElement`
  that should have matched `SlideElement`'s real `boolean` type.
- **13 names needed ambient `declare global` coverage**, extending
  `web/src/global_debug_surface.d.ts` (an ambient type-declaration file for
  the small set of names a later cleanup pass deliberately left mirrored via
  `(globalThis as any).NAME`, because something outside app_core.ts/app_ui.ts
  genuinely reads them). Turning on typecheck is what surfaced that
  `app_ui.ts` also reads two of them (`openModal`, `appOptions`) as *bare*
  identifiers, not just via the cast — a real gap the previous grep-based
  sweep missed because it only searched for `NAME(` call patterns, not bare
  variable references. No behavior changed; only the type declarations were
  added.

## Verification

`bun run build` (typecheck + all 5 bundles) succeeds; `tsc --noEmit` alone
reports 0 errors; `bun test` — 340/341 pass, the 1 failure
(`e2e_compare_windows_linux.test.ts`, a port-9062 startup timeout) is
pre-existing and unrelated — confirmed via `git stash` that it fails
identically on the commit before this change.
