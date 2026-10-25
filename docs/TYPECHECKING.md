# TypeScript type checking (2026-09-30)

_Last edited: 2026-10-15_

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
