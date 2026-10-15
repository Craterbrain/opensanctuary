Tiny real/synthetic MPEG audio fixtures for `src/storage/media_sniff.rs`'s tests:

- `bare_real.mp3`: a genuine MP3 stream (no ID3 tag) made with `ffmpeg -f lavfi -i "sine=frequency=440:duration=0.3" -ar 44100 -id3v2_version 0 -write_xing 0`.
- `synthetic_vbr.bin`: two hand-built, structurally valid MPEG-1 Layer III frame headers at different bitrates (VBR) with filler payload bytes, back to back, no real audio -- exercises frame-chaining across a bitrate change without needing an encoder.
