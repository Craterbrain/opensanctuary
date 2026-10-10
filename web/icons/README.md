# UI icons

One folder per color theme (`orange`, `slate`, `gold`, `white`, `crimson`), each with the
same 35 icons on a 24x24 grid. Each file is self-contained (its colors are in an
inline `<style>`), so it works as a plain `<img src="icons/<color>/<name>.svg">`.

The ribbon in `index.html` uses `icons/orange/`. To switch the ribbon to another
color, change the folder in those `<img>` paths.

All five sets are two-tone: a main color, a lighter highlight on secondary
details and a darker accent on structural parts. The tone assignment comes from
`crimson` (split from its sheet); the others reuse it with their own colors.
`orange` uses the app brand colors (`--os-brand-primary` `#FF5722`,
`--os-brand-amber` `#FFA726`, `--os-brand-flame` `#D84315`). The sheets in
`docs/icons/` are the original flat designs for slate, gold and white.

The source sheets (all icons in one preview image per color) are in
`docs/icons/`. These per-icon files were split out of them; if you edit a sheet,
re-split rather than editing both.

Packaging: `Cargo.toml` lists each color folder in both asset lists (main and
`no-tv-apk`) -- add a line there when adding a color. The tarball and Windows
installer scripts copy the whole `web/icons` directory.

## Color themes

Each set has a matching interface color theme (`colors` in
`src/core/icon_sets.ts`), applied by overriding the `--os-brand-*` CSS variables
that `style.css` builds its accents from. In Settings > Theme the two are one
cohesive theme by default (`themeLinked`); turn that off to pick the icon set
(`iconSet`) and the color theme (`colorTheme`) independently. Keep white text
readable on each theme's `primary` (a unit test checks this).
