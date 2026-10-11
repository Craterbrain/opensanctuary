# Animated slide backgrounds and the pre-render cache

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

* **Resolution:** 2560x1440 by default, 1920x1080 when Settings > Live Output >
  *Pre-rendered Background Quality* says so, or in Automatic mode when a connected
  monitor is smaller than 1440p.
* **Eligible patterns:** animated patterns whose loop is at most 80 s
  (`PRERENDER_MAX_LOOP_SEC`). The very slow ones (Starfield Drift, Drifting Clouds,
  ...) stay live: they are cheap to draw and would need thousands of frames.
* **Cache key:** the file name holds a revision hash of the pattern definition
  (`patternRevision`), so editing a pattern or the frame-drawing code (bump
  `RENDER_REVISION`) simply makes new clips; old ones age out.
* **Cleanup:** a clip that has not been used for **3 months** is deleted
  (`CACHE_TTL`); the server checks at startup and hourly, and any lookup/status check
  that finds a clip counts as a use. Stalled renders are abandoned after 5 minutes.
  *Settings > Live Output > Clear Rendered Backgrounds* (`DELETE /api/pattern-cache`)
  empties the cache immediately; `GET /api/pattern-cache` reports its size.
* **Off switch:** *Pre-render Animated Backgrounds: Off* keeps everything live.
