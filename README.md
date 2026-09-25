# OpenSanctuary (os-next)

Next-generation church presentation engine, written in Rust with a TypeScript
web-based control UI. Cross-platform (Linux, Windows, macOS), local-network
multi-display (operator/live/stage/remote), EasyWorship/OpenLP import, live
scripture and song scheduling, and theming.

Status: **alpha**. See [Releases](../../releases) for a prebuilt Linux build,
or [BUILDING.md](BUILDING.md) to build from source (all platforms).

## License

MIT — see [LICENSE](LICENSE). This covers the code only. Bundled Bible
translations (`bibles/`) are public-domain texts (KJV, ASV, World English
Bible); bundled songs (`songs/public_domain.db`) are public-domain hymns.
Copyrighted content (modern Bible translations, CCLI-licensed songs, stock
media) is intentionally not included — see [BUILDING.md](BUILDING.md) for how
to supply your own.

## Repository layout

- `src/` — Rust backend (show engine, storage, network, media pipeline)
- `web/` — TypeScript/HTML operator UI and live/stage/remote output pages
- `apps/` — companion apps (Android TV, Roku) for remote display output
- `tests/` — Rust integration tests and import-format fixtures
- `docs/` — architecture and protocol notes
- `vms/` — sandboxed VM test automation (see `vms/vms.md`)
