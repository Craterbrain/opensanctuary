# TypeScript type checking (2026-09-30)

_Last edited: 2026-10-15_

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
`noUnusedLocals` and `noUnusedParameters`. The last two were switched on after removing 43 dead imports, unused constants and a dead function
(`app_core.ts`, `app_ui.ts`, `library_panel.ts` and others) and two unused private fields in the WebCodecs player. A parameter that a signature has to keep but a
function doesn't use is named with a leading underscore (`_filePath`).

Measured and **left off on purpose**:

| Flag | Errors | Why not |
|---|---|---|
| `noUncheckedIndexedAccess` | 225 | Almost all are regex capture groups (`m[1]`) and pixel/animation math (`gradient_canvas.ts` 43, `background_patterns.ts` 35) where the index is valid by construction; the only fix is a `!` on each, which adds noise and finds nothing. The one file with real findings is `editor/slide_editor.ts` (56: slides that may not exist at an index); fix those by hand if that file is reworked. |
| `exactOptionalPropertyTypes` | 39 | Almost all are `{ x: maybeUndefined }` passed where `x?:` is declared, i.e. style, not bugs. |
| `noPropertyAccessFromIndexSignature` | 225 | Style only. |

There are 364 explicit `any` (`as any` / `: any`) in 86 files: those are the real holes, and no flag closes them. The way to shrink them is to type the objects that cross the API boundary (`Song`, `ScheduleItem`, `StateSnapshot`, ...) once and replace the casts where they are used.
