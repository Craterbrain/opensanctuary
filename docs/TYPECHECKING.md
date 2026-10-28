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

## Flags (updated 2026-10-27)

`strict` is **on** (it was turned on after the first pass; `noImplicitAny` is part of it), and so are three more that cost nothing today:
`noImplicitReturns`, `noFallthroughCasesInSwitch` and `noImplicitOverride` (zero errors when they were switched on).

Measured on 2026-10-27, not turned on:

| Flag | Errors now | Verdict |
|---|---|---|
| `noUnusedLocals` | 37 | Mostly dead imports and bindings in `app_core.ts` / `app_ui.ts`. Worth a cleanup pass, then switch it on. Some of those imports may exist for their side effects, so each needs a look rather than a blanket delete. |
| `noUnusedParameters` | 6 | Same cleanup. |
| `exactOptionalPropertyTypes` | 39 | Catches `undefined` assigned to an optional field; real findings but fiddly. Later. |
| `noUncheckedIndexedAccess` | 225 | Every `array[i]` becomes possibly undefined. The noisiest and the one that finds the most real bugs; do it per file, not all at once. |
| `noPropertyAccessFromIndexSignature` | 225 | Style only. Skip. |

There are 364 explicit `any` (`as any` / `: any`) in 86 files: those are the real holes, and no flag closes them. The way to shrink them is to type the objects that cross the API boundary (`Song`, `ScheduleItem`, `StateSnapshot`, ...) once and replace the casts where they are used.
