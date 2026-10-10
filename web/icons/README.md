# UI icons

One folder per color theme (`orange`, `slate`, `gold`, `white`, `crimson`), each with the
same 35 icons on a 24x24 grid. Each file is self-contained (its colors are in an
inline `<style>`), so it works as a plain `<img src="icons/<color>/<name>.svg">`.

The ribbon in `index.html` uses `icons/orange/`. To switch the ribbon to another
color, change the folder in those `<img>` paths.

`orange` is the app's brand color (`--os-brand-primary` `#FF5722`, amber accent
`#FFA726`); it is the slate set recolored. The other four were split from the
sheets.

The source sheets (all icons in one preview image per color) are in
`docs/icons/`. These per-icon files were split out of them; if you edit a sheet,
re-split rather than editing both.

Packaging: `Cargo.toml` lists each color folder in both asset lists (main and
`no-tv-apk`) -- add a line there when adding a color. The tarball and Windows
installer scripts copy the whole `web/icons` directory.
