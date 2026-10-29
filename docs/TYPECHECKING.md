# TypeScript type checking (2026-09-30)

_Last edited: 2026-10-29_

`web/` had no `tsconfig.json` and no `tsc` invocation anywhere before this —
`bun build` transpiles TypeScript (strips types) without ever checking them,
so every `as any`, every loosely-typed DOM lookup, and any real type error
was silently unchecked. This adds a real `tsc --noEmit` gate.

## What's wired where

- `web/tsconfig.json` — `moduleResolution: "Bundler"`,
  `allowImportingTsExtensions: true` (this codebase imports with explicit
  `.ts` extensions), `strict: true` (see "Flags" below), `noEmit: true`.
- `web/package.json`: new `"typecheck": "tsc --noEmit"` script, and `"build"`
  now runs it first (`bun run typecheck && bun build ...`) — a type error
  fails the whole `build` script before any bundler even runs.
- At the time this was written, no separate CI step was needed: the repo had
  no `.github/` workflows yet, and `build.rs` already runs `bun run build` on
  every `cargo build`/`cargo run` (panicking, with stdout/stderr captured, on
  a non-zero exit — see its own doc comment), and `web/tests/_pretest_build.ts`
  already runs `bun run build` before every `bun test`. Both inherited the
  typecheck gate for free by it living inside `build`, with zero changes to
  either file. Verified directly: a deliberately-injected type error in
  `main.ts` failed `bun run build` with the real `tsc` error surfaced, then
  reverted; a real `cargo check` after deleting `web/dist/` rebuilt it
  cleanly end to end.
  **Since superseded**: `.github/workflows/ci.yml` now exists (see
  `docs/workflow.md`) and its `frontend` job runs `bun run typecheck` as its
  own explicit step, before `bun run build`/`bun test` — belt-and-suspenders
  with the `build.rs`/`_pretest_build.ts` gates above, not a replacement for
  them (a Rust-only contributor building locally still gets the same gate
  through `build.rs`).

## Flags (updated 2026-10-28)

On, all checked by `bun run typecheck` and CI: `strict` (it includes `noImplicitAny`), `noImplicitReturns`, `noFallthroughCasesInSwitch`, `noImplicitOverride`,
`noUnusedLocals`, `noUnusedParameters` and `noUncheckedIndexedAccess`.

* `noUnusedLocals` / `noUnusedParameters` came on after removing 43 dead imports, constants and a dead function. A parameter a signature has to keep but a function
  doesn't use is named with a leading underscore (`_filePath`).
* `noUncheckedIndexedAccess` came on after guarding all 225 index sites (`array[i]`, regex groups, `split()` destructuring). No `!` was added: each site got a guard or a
  sensible fallback. `editor/slide_editor.ts` (56 sites) now keeps one invariant in one place (`getActiveSlide()` always returns a slide) and guards the swaps, reorders and
  empty selections.

Left off on purpose:

| Flag | Errors | Why not |
|---|---|---|
| `exactOptionalPropertyTypes` | 39 | Almost all are `{ x: maybeUndefined }` passed where `x?:` is declared, i.e. style, not bugs. |
| `noPropertyAccessFromIndexSignature` | 225 | Style only. |

## The shapes that cross the API boundary

`web/src/core/models.ts` mirrors the serde structs in `src/core/models.rs` (`StateSnapshot`, `ShowState`, `Schedule`, `ScheduleItem`, `Slide`, `Song`, `ScriptureItem`,
`MediaItem`, `Presentation`, `MediaPlayback`) plus the console's own row types (`LibraryItem`, `ThemeRow`, `InstalledBible`, `ShowCommand`). JSON has no `undefined`, so these use
`| null`; the slide *editor* keeps its own `EditorSlide` (with `undefined`) in `editor/types.ts`, and the conversion between the two is explicit. When the server adds a
field the console reads, add it here first; the compiler then shows every place that has to handle it.

Typing these found things the `any` had hidden: places that assumed a snapshot or a live item was there, a `Slide.footer` and a `MediaPlayback.preroll_target_pts` the
server never sends, a `.target.closest(...)` on a possibly-null event target, and a live-output call that passed `isLooping` where the engine reads `is_looping` (fixed:
a looping background video now loops).

## Explicit `any`

368 on 2026-10-27, 95 now. What is left, and why:

| Where | Count | Why |
|---|---|---|
| `global_debug_surface.d.ts` | 22 | The ambient debug names mirrored onto `globalThis` for devtools and the browser tests (see that file's header). |
| `core/plugins.ts`, `core/plugin_bridge.ts` | ~8 | Third-party plugin objects and messages: the host cannot know their shape. These should become `unknown` + checks. |
| `core/api_client.ts` | ~10 | Request/response bodies are whatever the endpoint sends; each call site should pass its own `<T>`. |
| `live_output.ts` `TimecodeSyncEngine` | 4 | Reshapes the server's media state into its own camelCase form; needs its own interface. |
| `app_ui.ts`, `app_core.ts` debug mirrors, `ui/filter_bar.ts`, small JSON bodies | the rest | Mostly one-offs; replace as the code around them is touched. |

Do not add new ones without a reason in a comment; use `unknown` and narrow, or add the type to `core/models.ts`.
