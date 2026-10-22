# ODP test fixtures

- `fixture.odp`: a real presentation built and saved by actual LibreOffice (via its pyuno Python
  scripting bridge, not hand-authored XML), so `odp_import.rs`'s tests run against the same shapes
  a real producer emits, not just what the OASIS spec says is legal. Contents: slide 1 has one text
  box with a plain run ("Hello ") followed by a bold+italic+red+28pt run ("World plain") across two
  right/left-aligned paragraphs; slide 2 has a filled rectangle, an embedded PNG image, and a second
  text box. Master page "Default" uses a 28cm x 15.75cm (16:9) page layout.
- `make_fixture.py`: the script that built it, kept so it can be regenerated. Needs a running
  `soffice --headless --invisible --accept="socket,host=localhost,port=2002;urp;" "-env:UserInstallation=file:///tmp/lo_profile"`
  listener (a writable, throwaway profile dir is required; the default one isn't writable headless
  in some environments). Not run by any test.
- `shape-test.odp`: a real ODP hand-authored by a person in actual LibreOffice Impress's shape
  gallery (not built via `pyuno` like `fixture.odp`): 9 shapes that inherit fill from a shared parent
  style, plus a 3D cube scene. Copied 2026-10-22 from the project owner's real-world test documents
  (docs/workflow.md, "Real-world test documents") with their permission to commit it; it has no
  text or author metadata. Used by `test_odp_import_against_a_real_world_shape_gallery_file` and
  `test_odp_export_of_a_real_world_shape_gallery_file_is_accepted_by_real_libreoffice`, which until
  then only ran on machines that had that folder.

