# Broadcasting OS-Next output (OBS setup)

_Last edited: 2026-10-01 14:19_

OS-Next has no built-in streaming encoder, RTMP/SRT output, or NDI sender, and it isn't getting one. The Live Display (FOH) page is a plain web page, and any screen-capture/streaming tool that can capture a web page can broadcast it. The recommended tool is **OBS Studio**, using its built-in **Browser Source** — no OS-Next code changes or plugins required.

## Why no native "pro broadcast" output

That's out of scope by design. OBS (or vMix, Wirecast, etc.) already does encoding, scene composition, multi-camera switching, and streaming/recording far better than a presentation tool should try to reimplement. OS-Next's job is to drive the display; broadcast is OBS's job.

## Quick setup

1. Start the server as usual:
   ```
   os-next --port 8080
   ```
   (`--port` defaults to `8080` if omitted. The server also prints the exact URLs it's serving on startup.)

2. In OBS, add a **Browser Source** to your scene:
   - **OBS on the same machine as OS-Next (recommended, simplest)**: URL
     `http://127.0.0.1:8080/live.html` — unaffected by anything below,
     this port is loopback-only by design and needs no certificate.
   - **OBS on a different machine**: URL `https://<server-LAN-IP>:8443/live.html`
     — the loopback port above isn't reachable from another machine at all
     (see `src/main.rs`'s loopback-bind comment), so this is the only
     option. **Real caveat, not yet resolved**: OBS's Browser Source
     (CEF-based) does not accept a self-signed certificate out of the box
     and has no in-app option to bypass it
     ([obsproject/obs-studio#3846](https://github.com/obsproject/obs-studio/issues/3846))
     — the source will likely just show blank rather than prompting for a
     click-through the way a normal browser tab does. Two ways around it,
     neither independently verified against a current OBS version:
     - Put [Caddy](https://caddyserver.com) in front of OS-Next
       (`docs/TUNNELS.md`) so the URL OBS loads has a real, CA-signed
       certificate — this should work cleanly since OBS's objection is
       specifically to self-signed/untrusted certs, not HTTPS itself.
     - OBS can reportedly take extra Chromium/CEF command-line flags via a
       `cef_command_line_args.txt` file next to its browser plugin,
       including `--ignore-certificate-errors` — worth trying, but check
       the current state of that GitHub issue before relying on it for a
       real service.
   - Width/Height: match your target output resolution (e.g. `1920x1080`).
   - FPS: leave at OBS's default, or match your stream/recording FPS.
   - Leave **"Control audio via OBS"** checked if you want OS-Next's video/audio playback (background videos, media items) mixed into your stream/recording audio — `live.html` plays real `<video>`/`<audio>` elements, so OBS's Chromium-based capture picks up both video frames and audio natively.
   - Leave **"Shutdown source when not visible"** *unchecked* so the WebSocket connection to OS-Next stays alive and the display doesn't go blank/reset when you switch away from that scene.

3. That's it — no query parameters or special "broadcast mode" are needed. `live.html` is already broadcast-clean out of the box:
   - Full-bleed black background, no browser chrome (Browser Source never shows any anyway).
   - `cursor: none` and `user-select: none` are already set, so there's never a stray mouse cursor or text-selection highlight in the capture.

## Other display endpoints you can capture the same way

- `/live.html` — main FOH/congregation display (the one you'll broadcast).
- `/stage.html` — stage confidence monitor feed. Add it as a second Browser Source if you also want a stage-monitor feed in a multiview or a separate output.

## The in-app "🌐 Web Integration & Streamer" modal (unrelated to OBS broadcasting)

The New ▾ → Website/Web Stream modal (`web-modal` / "▶ Project Web Stream" button) is a **different feature** from the OBS setup above — it's for pulling an external web page or YouTube video *into* the FOH display (inbound), not for sending OS-Next's own output out to a stream (that's what the OBS setup above is for).

- Enter a URL and pick a mode (Background layer / Picture-in-Picture / Fullscreen), then **▶ Project Web Stream**. YouTube watch/share links are automatically converted to a muted, autoplaying embed; any other URL is loaded directly in an iframe.
- **⏹ Stop Stream** clears it.
- This goes out as a new `SetWebStream` show command / `web_stream` field on the state snapshot, rendered on `live.html` as an 8th compositing layer (`#screen-web-stream`), so it also shows up in whatever OBS Browser Source you've pointed at `/live.html` — no separate OBS source needed for it.
- Raw RTMP/HLS feeds are **not** supported this way (a plain iframe can't play those) — use a Live Camera Feed schedule item for that instead. The modal's help text was corrected to say so.
- "📋 Add to Schedule" (queuing a web feed as a future schedule item, rather than projecting it immediately) still just creates a DB row today; making it a first-class slide type in the Go-Live rotation would be a separate, larger change to the arrangement engine if that's wanted later.

