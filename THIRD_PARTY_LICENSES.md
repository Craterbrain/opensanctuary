# Third-Party Licenses & Attribution

_Last edited: 2026-10-01 15:31_

OpenSanctuary (`os-next`) is MIT-licensed (see `LICENSE`). It builds on a
large number of open-source libraries, and a couple of external tools it
bundles or fetches at runtime. This file lists all of them, grouped by
license, so their terms are visible independent of any single binary.

This list is generated from `Cargo.lock` via
[`cargo-license`](https://github.com/onur/cargo-license)
(`cargo license --avoid-dev-deps --avoid-build-deps`) plus a manual check of
`web/package.json`'s runtime dependencies and `apps/android-tv`'s Gradle
dependencies. It covers what's actually compiled into a release build or
shipped in the web bundle / APK — not build-time-only tooling (`cargo-deb`,
`cargo-xwin`, `bun`, `tsc`, Gradle itself, etc.), which never ships to an
end user.

For brevity, this file lists each dependency's SPDX license identifier and
links to the canonical license text at [spdx.org](https://spdx.org/licenses/)
rather than reproducing dozens of near-identical permissive license texts
verbatim — every license named below is a standard, unmodified OSI-approved
license. `Cargo.lock` is the authoritative source for exact versions in any
given build.

## Rust crates (`Cargo.lock`)

Regenerate this section with:

```sh
cargo install cargo-license --locked
cargo license --avoid-dev-deps --avoid-build-deps
```

### [MIT](https://spdx.org/licenses/MIT.html)

atk, atk-sys, axum, axum-core, axum-macros, axum-server, block, block2,
bytes, cairo-rs, cairo-sys-rs, com, com_macros, com_macros_support, combine,
convert_case, data-encoding, derive_more, dispatch, dlib, dlopen2,
dlopen2_derive, endi, gdk, gdk-pixbuf, gdk-pixbuf-sys, gdk-sys,
gdkwayland-sys, gdkx11, gdkx11-sys, generic-array, gio, gio-sys, glib,
glib-macros, glib-sys, gobject-sys, gtk, gtk-sys, gtk3-macros, h2,
hassle-rs, http-body, http-body-util, http-range-header, hyper, hyper-util,
is-docker, is-wsl, javascriptcore-rs, javascriptcore-rs-sys, kuchikiki,
libflate, libflate_lz77, libredox, libsqlite3-sys, malloc_buf, matchers,
matches, memoffset, mime_guess, **minisign-verify**, mio, new_debug_unreachable,
nu-ansi-term, objc, objc2, objc2-encode, objc2-foundation, objc_exception,
objc_id, open, openssl-sys, pango, pango-sys, pem, phf, phf_generator,
phf_macros, phf_shared, precomputed-hash, quick-xml, redox_syscall,
redox_users, rfd, rusqlite, schannel, sharded-slab, simd-adler32, slab,
soup3, soup3-sys, spin, strsim, synstructure, tokio, tokio-macros,
tokio-native-tls, tokio-tungstenite, tokio-util, tower, tower-http,
tower-layer, tower-service, tracing, tracing-attributes, tracing-core,
tracing-log, tracing-subscriber, try-lock, uds_windows, urlencoding,
valuable, want, wayland-backend, wayland-client, wayland-protocols,
wayland-scanner, wayland-sys, webkit2gtk, webkit2gtk-sys, webview2-com,
webview2-com-macros, webview2-com-sys, winapi, winnow, x11, x11-dl, zbus,
zbus_macros, zbus_names, zcheapstr, zip, zstd, zvariant, zvariant_derive,
zvariant_utils

### [Apache-2.0](https://spdx.org/licenses/Apache-2.0.html)

codespan-reporting, glutin_wgl_sys, openssl, spirv, sync_wrapper, tao

### Apache-2.0 OR MIT (dual-licensed; used under either)

aes, ahash, allocator-api2, android_system_properties, anstream, anstyle,
anstyle-parse, anstyle-query, anstyle-wincon, anyhow,
apple-native-keyring-store, arc-swap, arrayvec, ash, async-broadcast,
async-channel, async-executor, async-io, async-lock, async-process,
async-recursion, async-signal, async-task, async-trait, atomic-waker,
base64, base64ct, bit-set, bit-vec, bitflags, block-buffer, block-padding,
blocking, bumpalo, bzip2, bzip2-sys, cbc, cesu8, cfg-if, chrono, cipher,
clap, clap_builder, clap_derive, clap_lex, cmov, cocoa, cocoa-foundation,
colorchoice, concurrent-queue, const-oid, core-foundation,
core-foundation-sys, core-graphics, core-graphics-types, cpubits,
cpufeatures, crc32fast, crossbeam-channel, crossbeam-utils, crypto-common,
ctutils, d3d12, dary_heap, deranged, digest, dirs, dirs-sys, displaydoc,
document-features, downcast-rs, dtoa, enumflags2, enumflags2_derive,
equivalent, errno, event-listener, event-listener-strategy,
fallible-iterator, fallible-streaming-iterator, fastrand, fdeflate,
field-offset, flate2, flume, fnv, foreign-types, foreign-types-macros,
foreign-types-shared, form_urlencoded, fs-err, futf, futures-channel,
futures-core, futures-executor, futures-io, futures-lite, futures-macro,
futures-sink, futures-task, futures-util, fxhash, getrandom,
**gpgrv**, gpu-alloc, gpu-alloc-types, gpu-allocator, gpu-descriptor,
gpu-descriptor-types, hashbrown, hashlink, heck, hermit-abi, hex, hkdf,
hmac, html5ever, http, httparse, httpdate, hybrid-array, hyper-tls,
iana-time-zone, iana-time-zone-haiku, idna, idna_adapter, image,
image-webp, indexmap, inout, iowrap, ipnet, is_terminal_polyfill, itoa,
jni, jni-sys, jni-sys-macros, js-sys, keyring, keyring-core, khronos-egl,
lazy_static, libc, litrs, lock_api, log, mac, markup5ever, mdns-sd, metal,
mime, naga, native-tls, ndk, ndk-context, ndk-sys, no_std_io2, nodrop, num,
num-bigint, num-complex, num-conv, num-integer, num-iter, num-rational,
num-traits, once_cell, once_cell_polyfill, openssl-macros, openssl-probe,
ordered-stream, parking, parking_lot, parking_lot_core, password-hash,
paste, pbkdf2, percent-encoding, pin-project-lite, piper, png,
polling, pollster, powerfmt, ppv-lite86, presser, proc-macro-crate,
proc-macro-error, proc-macro-error-attr, proc-macro-hack, proc-macro2,
profiling, qrcode, quick-error, quote, rand, rand_chacha, rand_core,
rand_hc, rand_pcg, range-alloc, rcgen, regex, regex-automata, regex-syntax,
renderdoc-sys, reqwest, rle-decode-fast, rustc-hash, rustls-pki-types,
rustversion, scoped-tls, scopeguard, secret-service, security-framework,
security-framework-sys, semver, serde, serde_core, serde_derive,
serde_json, serde_path_to_error, serde_repr, serde_spanned, serde_urlencoded,
servo_arc, sha1, sha2, signal-hook-registry, siphasher, smallvec, socket2,
stable_deref_trait, static_assertions, string_cache, syn,
system-configuration, system-configuration-sys, tao-macros, tempfile,
tendril, thiserror, thiserror-impl, thread_local, time, time-core,
tokio-rustls, toml_datetime, toml_edit, toml_parser, tungstenite, typenum,
unicase, unicode-segmentation, unicode-width, unicode-xid, url, utf-8,
utf8_iter, utf8parse, uuid, wasm-bindgen, wasm-bindgen-futures,
wasm-bindgen-macro, wasm-bindgen-macro-support, wasm-bindgen-shared,
web-sys, wgpu, wgpu-core, wgpu-hal, wgpu-types, widestring, winapi-i686-pc-windows-gnu,
winapi-x86_64-pc-windows-gnu, windows, windows-core, windows-implement,
windows-interface, windows-link, windows-native-keyring-store,
windows-registry, windows-result, windows-strings, windows-sys,
windows-targets, windows-version, windows_aarch64_gnullvm,
windows_aarch64_msvc, windows_i686_gnu, windows_i686_gnullvm,
windows_i686_msvc, windows_x86_64_gnu, windows_x86_64_gnullvm,
windows_x86_64_msvc, wry, yasna, zbus-secret-service-keyring-store,
zeroize, zstd-safe, zstd-sys

### Other Apache-2.0/MIT-family combinations

- **Apache-2.0 OR MIT OR Zlib**: bytemuck, dispatch2, glow, miniz_oxide,
  objc2-app-kit, objc2-core-foundation, raw-window-handle, zune-core,
  zune-jpeg
- **(Apache-2.0 OR MIT) AND BSD-3-Clause**: encoding_rs
- **(Apache-2.0 OR MIT) AND Unicode-3.0**: unicode-ident
- **0BSD OR Apache-2.0 OR MIT**: adler2
- **Apache-2.0 AND ISC**: ring
- **Apache-2.0 AND MIT**: dpi
- **Apache-2.0 OR Apache-2.0 WITH LLVM-exception OR MIT**: linux-raw-sys,
  rustix, wasi
- **Apache-2.0 OR BSD-2-Clause OR MIT**: zerocopy, zerocopy-derive
- **Apache-2.0 OR BSD-3-Clause**: moxcms, pxfm
- **Apache-2.0 OR BSD-3-Clause OR MIT**: num_enum, num_enum_derive
- **Apache-2.0 OR BSL-1.0**: ryu
- **Apache-2.0 OR CC0-1.0 OR MIT-0**: dunce
- **Apache-2.0 OR ISC OR MIT**: hyper-rustls, rustls, rustls-pemfile
- **Apache-2.0 OR LGPL-2.1-or-later OR MIT**: r-efi -- used here under the
  Apache-2.0/MIT option, not the LGPL option (that clause exists only
  because r-efi has no FSF copyright assignment for a pure Apache/MIT
  release; choosing the permissive option imposes no copyleft obligation)

### [BSD-3-Clause](https://spdx.org/licenses/BSD-3-Clause.html)

instant, sha1_smol, subtle

### BSD-3-Clause AND MIT

matchit

### BSD-2-Clause OR MIT / BSD-3-Clause OR MIT

c_linked_list, get_if_addrs, get_if_addrs-sys, if-addrs

### [ISC](https://spdx.org/licenses/ISC.html)

libloading, rustls-webpki, untrusted

### [CC0-1.0](https://spdx.org/licenses/CC0-1.0.html)

constant_time_eq, hexf-parse

### [MPL-2.0](https://spdx.org/licenses/MPL-2.0.html)

cssparser, cssparser-macros, dtoa-short, option-ext, selectors, thin-slice

(Weak/file-level copyleft: only applies to modifications made to these
crates' own files, which this project doesn't modify.)

### [Unicode-3.0](https://spdx.org/licenses/Unicode-3.0.html)

icu_collections, icu_locale_core, icu_normalizer, icu_normalizer_data,
icu_properties, icu_properties_data, icu_provider, litemap, potential_utf,
tinystr, writeable, yoke, yoke-derive, zerofrom, zerofrom-derive, zerotrie,
zerovec, zerovec-derive

### [Zlib](https://spdx.org/licenses/Zlib.html)

adler32, foldhash, slotmap

### MIT OR Unlicense

aho-corasick, byteorder, byteorder-lite, memchr, termcolor, winapi-util

## Web frontend (`web/package.json`, bundled into the shipped JS)

| Package | License |
|---|---|
| [@atlaskit/pragmatic-drag-and-drop](https://www.npmjs.com/package/@atlaskit/pragmatic-drag-and-drop) | Apache-2.0 |
| [@atlaskit/pragmatic-drag-and-drop-hitbox](https://www.npmjs.com/package/@atlaskit/pragmatic-drag-and-drop-hitbox) | Apache-2.0 |
| [jsqr](https://www.npmjs.com/package/jsqr) | Apache-2.0 |
| [qrcode](https://www.npmjs.com/package/qrcode) | MIT |
| [mediabunny](https://mediabunny.dev) | MPL-2.0 |

Build-only tooling (`typescript`, `playwright`, `@types/qrcode`) never ships
to an end user and is omitted.

## Android TV client (`apps/android-tv`, bundled into the APK)

| Library | License |
|---|---|
| AndroidX (core-ktx, appcompat, constraintlayout) | Apache-2.0 |
| Material Components for Android | Apache-2.0 |
| Kotlin Coroutines (kotlinx-coroutines-android) | Apache-2.0 |
| AndroidX Media3 (ExoPlayer) | Apache-2.0 |
| [ZXing](https://github.com/zxing/zxing) ("core") | Apache-2.0 |
| [OkHttp](https://square.github.io/okhttp/) | Apache-2.0 |

## Bundled or fetched external tools (not Rust/Cargo dependencies)

These aren't linked into `os-next`'s own binary -- they're invoked as
separate subprocesses (`src/storage/ytdlp_import.rs`,
`src/storage/transcode.rs`) -- but ship alongside it on Windows, so their
own license terms apply to those specific files.

### ffmpeg / ffprobe (Windows installer/portable build only)

- **License**: LGPLv3 (this project's build, specifically -- see below).
- **Source**: [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds),
  pinned release tag -- see `packaging/build-windows-installer.sh`'s
  "Bundled ffmpeg/ffprobe" comment for the exact tag and the reasoning for
  using this build specifically (an LGPL build, not gyan.dev's GPLv3-only
  builds).
- The exact `LICENSE-LGPLv3.txt` and a `SOURCE.txt` pointer to the precise
  build and upstream FFmpeg commit ship in the installer/portable zip's own
  `ffmpeg\` folder, as LGPLv3 requires.
- Linux (`.deb`/generic tarball) does not bundle ffmpeg; it's listed as an
  `apt`/package-manager `Recommends` instead (`Cargo.toml`'s
  `[package.metadata.deb]`) and is whatever license the user's distro
  packages it under.

### yt-dlp (fetched by the app itself, not bundled in any release artifact)

- **License**: [Unlicense](https://github.com/yt-dlp/yt-dlp/blob/master/LICENSE)
  (public-domain-equivalent) for the core project. The standalone
  platform executables yt-dlp itself publishes (what
  `src/network/ytdlp_updater.rs` downloads) are PyInstaller bundles that
  yt-dlp's own release notes document as GPLv3+ combined works -- see
  [yt-dlp's README "LICENSE" section](https://github.com/yt-dlp/yt-dlp#license)
  and its `THIRD_PARTY_LICENSES.txt` for the full breakdown of what's
  bundled inside that specific executable.
- Not shipped in any OpenSanctuary release artifact: `ytdlp_updater.rs`
  downloads it directly from yt-dlp's own GitHub releases at runtime (see
  `docs/installer.md`'s "Bundled ffmpeg + self-managed yt-dlp" section for
  why).
- `src/network/ytdlp_public_key.asc` is yt-dlp's own published GPG public
  key (RSA-4096, fingerprint
  `AC0C BBE6 848D 6A87 3464 AF4E 57CF 6593 3B5A 7581`), used only to verify
  the authenticity of what's downloaded -- not OpenSanctuary's own key, and
  not itself a license grant.
