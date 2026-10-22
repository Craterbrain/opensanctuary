/**
 * Renders the animated-background thumbnails the pickers show (web/pattern-thumbs/), so a grid of
 * previews is a handful of small looping images instead of every pattern drawn live at once.
 *
 *   cd web && bun scripts/render_pattern_thumbs.ts            # only missing or out-of-date ones
 *   cd web && bun scripts/render_pattern_thumbs.ts --all      # every one
 *
 * Each frame is drawn by the same code that renders the live clips (renderPatternFrame), at full
 * slide size in headless Chromium, and ffmpeg scales it down and encodes the loop (libwebp_anim). A loop
 * longer than MAX_SEC plays sped up so the thumbnail still loops seamlessly. manifest.json
 * records each thumbnail's patternRevision; tests/pattern_thumbs.test.ts fails when a pattern
 * changes without its thumbnail being rendered again.
 */
import { chromium } from "playwright";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync, existsSync } from "fs";
import { join, resolve } from "path";
import { tmpdir } from "os";

export const THUMB_W = 256, THUMB_H = 144, THUMB_FPS = 12, MAX_SEC = 6;
/** Frames are drawn at full slide size and scaled down: blurred layers drawn small come out blocky. */
const RENDER_W = 1920, RENDER_H = 1080;
const OUT = resolve(import.meta.dir, "../pattern-thumbs");
const all = process.argv.includes("--all");

const entry = resolve(import.meta.dir, "pattern_thumbs_page.ts");
const built = await Bun.build({ entrypoints: [entry], target: "browser" });
if (!built.success) throw new Error(built.logs.join("\n"));
const script = await built.outputs[0].text();

mkdirSync(OUT, { recursive: true });
const manifestPath = join(OUT, "manifest.json");
const manifest: Record<string, { rev: string; file: string }> = existsSync(manifestPath) ? JSON.parse(readFileSync(manifestPath, "utf8")) : {};

const browser = await chromium.launch({ headless: true });
const page = await browser.newPage();
await page.setContent("<!doctype html><html><body></body></html>");
await page.addScriptTag({ content: script });
const patterns: { key: string; rev: string; loopSec: number }[] = await page.evaluate(() => (window as any).thumbPatterns());

let made = 0;
for (const p of patterns) {
  const file = `${p.key}.webp`;
  if (!all && manifest[p.key]?.rev === p.rev && existsSync(join(OUT, file))) continue;
  const sec = Math.min(p.loopSec, MAX_SEC);
  const frames = Math.max(2, Math.round(sec * THUMB_FPS));
  const dir = mkdtempSync(join(tmpdir(), "pattern-thumb-"));
  for (let i = 0; i < frames; i++) {
    // Evenly over the whole loop, so the last frame leads straight back into the first.
    const t = (i / frames) * p.loopSec;
    const b64: string = await page.evaluate(([k, t, w, h]) => (window as any).thumbFrame(k, t, w, h), [p.key, t, RENDER_W, RENDER_H] as const);
    writeFileSync(join(dir, `${String(i).padStart(4, "0")}.png`), Buffer.from(b64, "base64"));
  }
  const ff = Bun.spawnSync(["ffmpeg", "-y", "-loglevel", "error", "-framerate", String(THUMB_FPS), "-i", join(dir, "%04d.png"),
    "-vf", `scale=${THUMB_W}:${THUMB_H}:flags=lanczos`, "-c:v", "libwebp_anim", "-lossless", "0", "-q:v", process.env.THUMB_Q ?? "92", "-preset", "picture", "-compression_level", "6", "-loop", "0", join(OUT, file)]);
  rmSync(dir, { recursive: true, force: true });
  if (ff.exitCode !== 0) throw new Error(`ffmpeg failed for ${p.key}: ${ff.stderr.toString()}`);
  manifest[p.key] = { rev: p.rev, file };
  made++;
  console.log(`${p.key}: ${frames} frames${p.loopSec > MAX_SEC ? ` (${p.loopSec}s loop sped up)` : ""}`);
}
for (const k of Object.keys(manifest)) if (!patterns.some(p => p.key === k)) delete manifest[k];
writeFileSync(manifestPath, JSON.stringify(Object.fromEntries(Object.entries(manifest).sort()), null, 1) + "\n");
await browser.close();
console.log(`${made} rendered, ${patterns.length - made} already up to date -> ${OUT}`);
