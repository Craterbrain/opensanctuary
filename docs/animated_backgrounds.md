# Animated slide backgrounds and the pre-render cache

_Last edited: 2026-10-22_

Animated backgrounds are procedural: a slide stores only `pattern:<key>`
(`background: "pattern:sprouting-leaves"`), and the pattern is built from SVG
and CSS in `web/src/core/background_patterns.ts`.

## Pre-rendered clips

Redrawing a pattern every frame is hard on a slow TV browser, so the console
renders one seamless loop of each eligible pattern into a cached video:

1. When an animated pattern is picked in the editor, or appears in the Preview or
   Live decks, the console (`web/src/core/pattern_render.ts`) asks
   `POST /api/pattern-cache/start` for a render job.
2. The page draws every frame off-screen (`gradient_canvas.ts` paints the
   layers straight onto a canvas) and uploads each as a PNG to
   `POST /api/pattern-cache/jobs/<id>/frame`. A drifting layer moves by whole tile
   sizes and ping-pong patterns render both legs, so the loop is seamless.
3. `src/storage/pattern_cache.rs` pipes the frames into the bundled `ffmpeg`
   (`libvpx-vp9`, yuv420p, constant quality, 30 fps, no audio) and publishes
   `<media_dir>/pattern_cache/<key>_<rev>_<w>x<h>_30fps.webm`.
4. The live output (`web/src/live_output.ts`) calls
   `GET /api/pattern-cache/lookup?key=&rev=`; if a clip exists it plays it as an
   ordinary looping video background, otherwise it keeps drawing the pattern
   live. Nothing waits on the render and any failure just leaves the live version.

* **Resolution:** 2560x1440 by default, 1920x1080 when Settings > Animated
  Backgrounds > *Quality* says so, or in Automatic mode when a connected monitor is
  smaller than 1440p.
* **Which backgrounds render (per background: Auto / Always / Never).** Every
  animated pattern has a *cost* (Light / Medium / Heavy: how hard the live version is
  to draw, measured as frame time at 1080p; see `PatternCost` in
  `background_patterns.ts`).
  * **Auto** (default): render it when it is used if it is Medium or Heavy and needs at
    most 2400 frames (`AUTO_MAX_FRAMES`). Light ones and very slow ones are left live.
  * **Always**: render it ahead of time (queued a few seconds after startup, or as soon
    as you pick Always), whatever its cost or length.
  * **Never**: always draw it live. The master *Pre-render* switch overrides all of
    these.
  Loops longer than 80 s (Drifting Clouds, Starfield Drift, ...) render at 10 fps: they
  drift a few px/s so the steps are invisible. The Starfield needs 4200 frames, so only
  renders when set to Always.
* **Seamless handoff.** The live version is always shown first. It is phase-locked to
  the wall clock (`patternPhaseSec`: CSS `animation-delay`, SMIL negative `begin` for
  the plant), and the clip plays at the same clock position (WebCodecs anchor 0, i.e.
  position = server time mod clip length). When the live output finds a new clip (it
  re-checks every 20 s while a pattern is on screen) the video fades in over the
  pattern for 0.8 s and the pattern is then dropped. The Android TV app's native
  player is not given clips; it keeps drawing the pattern.
* **Cache key:** the file name holds a revision hash of the pattern definition
  (`patternRevision`), so editing a pattern or the frame-drawing code (bump
  `RENDER_REVISION`) simply makes new clips; old ones age out.
* **Cleanup:** a clip that has not been used for **3 months** is deleted
  (`CACHE_TTL`); the server checks at startup and hourly, and any lookup/status check
  that finds a clip counts as a use. Stalled renders are abandoned after 5 minutes.
  *Settings > Live Output > Clear Rendered Backgrounds* (`DELETE /api/pattern-cache`)
  empties the cache immediately; `GET /api/pattern-cache` reports its size.
* **Off switch:** *Pre-render Animated Backgrounds: Off* keeps everything live.
* **Settings:** Settings > Animated Backgrounds has the switch, quality, the cache size and
  clear button, and the per-background choices with their cost badge.

## Thumbnail previews

Anywhere many patterns show at once at thumbnail size (the background pickers, the theme cards in
the Background > CSS picker, the ribbon's background swatch, the slide thumbnails in the Preview and
Live decks and the editor's filmstrip), each pattern is shown as a small pre-rendered looping image
instead of being drawn live. Drawing every pattern live at once (layered SVG and CSS animations)
kept the console's CPU and GPU flat out. Only the full-size canvases (the editor stage, Preview,
Live and the live output) still draw the pattern live.

* **Files:** `web/pattern-thumbs/<key>.webp` (256x144 animated WebP, 12 fps, loops forever) plus
  `manifest.json`, which records the `patternRevision` each one was made from. They are committed
  and shipped in the packages; nothing is rendered on the operator's machine.
* **Making them:** `cd web && bun scripts/render_pattern_thumbs.ts` renders the missing or
  out-of-date ones (`--all` for every one). It draws each frame with the same code as the
  pre-rendered clips (`renderPatternFrame`) at 1920x1080 in headless Chromium, then ffmpeg scales
  it down and encodes it (`libwebp_anim`, quality 92; lower qualities break the smooth dark
  gradients into blocks). A loop longer than 6 s is sped up so the thumbnail still loops seamlessly.
* **Keeping them current:** `web/src/core/pattern_thumbs.ts` only uses a preview whose revision
  matches the pattern; otherwise it falls back to drawing the pattern live. `tests/pattern_thumbs.test.ts`
  fails when any animated pattern lacks an up-to-date preview, so editing a pattern means running
  the script and committing the new files.
