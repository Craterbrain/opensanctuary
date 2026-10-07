/**
 * bun:test preload hook (wired via ../bunfig.toml's [test] preload) that
 * rebuilds the frontend bundle before any test file runs.
 *
 * Why this exists: `web/tests/*.test.ts` splits into two very different
 * kinds of test. Plain unit tests (`import ... from '../src/...'`) always
 * exercise current source -- bun transpiles TypeScript on the fly, so
 * `dist/` is irrelevant to them. But the e2e_*.test.ts files (and
 * theme_picker.test.ts) spawn the real release binary and drive it with an
 * actual browser, which loads `index.html`'s `<script src="dist/bundle.js">`
 * -- i.e. whatever was last built, not current source. Nothing rebuilds
 * `dist/` automatically (no build.rs hook, no file watcher running by
 * default), so editing web/src/*.ts and then running e2e tests without an
 * explicit `bun run build` in between silently exercises stale JS. That's
 * exactly what happened investigating a real bug this same session: a fix
 * was already committed, but the actual running app kept showing the old,
 * broken behavior because `dist/` hadn't been rebuilt.
 *
 * `bun test` itself does not run npm-style `pretest` scripts (confirmed:
 * only `bun run <script>` honors pre/post hooks) -- this preload,
 * configured via bunfig.toml's `[test]` section, is what actually runs
 * before every `bun test` invocation regardless of which files are
 * targeted, exactly once per run (not once per file).
 */
import { spawnSync } from "bun";
import { resolve } from "path";

const webDir = resolve(import.meta.dir, "..");
const result = spawnSync(["bun", "run", "build"], {
  cwd: webDir,
  stdout: "inherit",
  stderr: "inherit",
});

if (!result.success) {
  console.error("\n[_pretest_build] `bun run build` failed -- aborting the test run rather than letting e2e tests silently exercise a stale dist/bundle.js.\n");
  process.exit(result.exitCode ?? 1);
}
