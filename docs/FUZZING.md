# Fuzzing untrusted input parsers

_Last edited: 2026-10-03 00:00_

OS-Next's biggest raw-input attack surface isn't the network API (that's
JSON over a host-token-gated Axum router, handled by `serde`) -- it's the
**import pipeline**. Every importer in `src/storage/` takes a file an
operator drags in from an unknown source (a USB stick, an email attachment,
a download from a church's old EasyWorship machine) and parses it with
hand-rolled logic, because none of these formats had a usable Rust crate:
PPTX's OOXML is scanned with manual string search (`pptx_import.rs`), RTF
has a hand-written tokenizer (`rtf.rs`), and EWSX/OpenLP/FreeShow all mix
zip archives with loosely-structured embedded JSON/XML. Hand-rolled parsing
over attacker-controlled bytes is exactly the class of code fuzzing exists
for -- a malformed import file should produce an `Err`, never a panic, an
infinite loop, or a crash.

This is **implemented**: `fuzz/` is a `cargo-fuzz` project (libFuzzer) with
one target per hand-rolled parser.

## What's covered

| Target | Parser | Input shape |
|---|---|---|
| `pptx_import` | `PptxImporter::import_pptx_bytes` | raw `.pptx` (zip) bytes |
| `ewsx_import` | `EwsxManager::load_schedule_from_bytes` | raw `.ewsx`/`.osz`/`.ewpx` bytes |
| `openlp_service_json` | `OpenLPImporter::import_openlp_service_json` | JSON text |
| `openlp_theme` | `parse_openlp_theme_json` + `parse_openlp_theme_xml` | JSON or XML text |
| `openlp_otz` | `OpenLPImporter::parse_openlp_otz` | raw `.otz` (zip) bytes |
| `freeshow_show_import` | `FreeShowShowImporter::import_show_bytes` | raw `.show` (JSON) bytes |
| `freeshow_fsb` | `FreeShowImporter::import_fsb_content` | JSON text |
| `scripture_ref` | `parse_scripture_reference` | free-text reference string |
| `genius_lyrics` | `parse_genius_lyrics_to_song` + `extract_ccli_from_text` | free-text lyrics |
| `rtf_parse` | `parse_rtf` | raw RTF bytes |

Each target calls the same `pub fn` entry point the real import routes
(`src/api/routes/schedule.rs` et al.) call, with no mocking -- a crash found
by the fuzzer is a crash a real imported file could trigger. `pptx_import`
and `freeshow_show_import` pass a shared, process-lifetime `tempfile`
directory as `media_dir` (both importers copy referenced media out to
disk); that directory accumulates files over a long run and should be
cleared between fuzzing sessions.

Not covered, deliberately: the Axum HTTP layer (payloads there are
`serde`-typed and rejected before reaching application code), and
`openlp_import.rs`'s SQLite-file importers (`import_songs_db`,
`import_bible_db`) -- fuzzing those would mean fuzzing the `rusqlite`/SQLite
parser itself, not this project's own code; a malformed `.sqlite`/`.db`
file is `rusqlite`'s problem to reject, and it already does.

## Running it

Requires a nightly toolchain (libFuzzer needs sanitizer codegen that stable
`rustc` doesn't expose) and `cargo-fuzz`, both one-time installs:

```sh
rustup toolchain install nightly
cargo install cargo-fuzz
```

Run one target (Ctrl-C to stop; libFuzzer runs until stopped or a crash):

```sh
cargo +nightly fuzz run pptx_import
```

Run for a bounded time instead (useful in CI or before a release):

```sh
cargo +nightly fuzz run pptx_import -- -max_total_time=120
```

Run every target for a fixed budget each:

```sh
for t in pptx_import ewsx_import openlp_service_json openlp_theme \
         openlp_otz freeshow_show_import freeshow_fsb scripture_ref \
         genius_lyrics rtf_parse; do
  cargo +nightly fuzz run "$t" -- -max_total_time=120 || break
done
```

A crash writes a reproducing input to `fuzz/artifacts/<target>/`, printable
back through the same target:

```sh
cargo +nightly fuzz run pptx_import fuzz/artifacts/pptx_import/crash-<hash>
```

## Findings so far

The first run (2026-10-05, ~20s per target from small seed corpora) found four
panics, all fixed; their crash inputs live in `tests/fixtures/fuzz_regressions/`
and are replayed by `test_fuzz_regressions_do_not_panic` in `tests/core_tests.rs`:

- `ewsx_import`: legacy-header fallback sliced `bytes[38..80]` when the file was 61-79 bytes.
- `openlp_theme`: `</background>` appearing before the opening tag's `>` produced a reversed slice range.
- `rtf_parse`: `\'hh` hex escape sliced the `&str` at a non-char boundary.
- `scripture_ref`: en-dash (3 bytes) verse ranges were split with `dash_pos + 1`.

Seed the corpora (gitignored) by copying real files into `fuzz/corpus/<target>/`,
e.g. `test1.ewsx` and `tests/fixtures/openlp/*` -- coverage grows much faster.

## What to do with a crash

1. Reproduce it locally with the command above to confirm it's real (not
   e.g. a corpus file from a prior run of a *different* target).
2. Minimize it: `cargo +nightly fuzz tmin pptx_import fuzz/artifacts/pptx_import/crash-<hash>`.
3. Fix the parser so the minimized input returns `Err`/`None` instead of
   panicking -- the fix almost always belongs in the parser itself (a
   missing bounds check, an `unwrap()` that should be a `?` or `.ok()?`),
   not in the fuzz target.
4. Add the minimized crashing input as a regular `#[test]` case in the
   relevant file under `tests/core_tests.rs` or the module's own inline
   tests, so a regression is caught by `cargo test` without needing the
   fuzzer re-run. `fuzz/` is a corpus-generation and crash-discovery tool,
   not a replacement for the deterministic test suite.

## Why not the whole app

Fuzzing works best on narrow, deterministic, pure(-ish) functions that take
bytes in and produce a value or an error -- exactly what every importer
above already is. The rest of the app (the Axum router, the `tao`/`wry`
event loop, SQLite access) either isn't parsing attacker-controlled bytes
directly, or is a much better target for the existing integration/e2e test
suite (`tests/core_tests.rs`, `web/tests/e2e_*.test.ts`) than for a fuzzer,
which can't drive a real browser or a native window.

## Open questions

- Whether to wire a short (`-max_total_time=60`-per-target) fuzz pass into
  CI as a standing regression gate, versus leaving this as a manual,
  occasional tool run before a release. Not decided yet.
- `pptx_import`/`freeshow_show_import`'s shared `media_dir` approach means
  two importers that both extract referenced media share one target's
  disk-growth tradeoff; revisit if that becomes a real problem running
  long fuzz sessions.
