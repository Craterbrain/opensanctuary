# EasyWorship `.ews` test fixtures

- `test1.ews`: a genuine EasyWorship 5 schedule file (2.7 KB), copied 2026-10-22 from the project
  owner's real-world test documents (`~/Documents/OpenSanctuary-test-docs`, see docs/workflow.md,
  "Real-world test documents") with their permission to commit it. One song: the Danish harvest
  hymn "Vi pløjed og vi så'de" by Matthias Claudius (1782), in the public domain; its copyright
  field reads "Den Danske". Used by `test_ews_import_of_a_real_easyworship_5_schedule`
  (`tests/core_tests/importers.rs`).

  Layout, as this file shows it (storage/ewsx.rs `parse_ews_v5_songs`): the header string
  "EasyWorship Schedule File Version    5", song count as a u32 at 0x38, record size as a u16 at
  0x3c (1816 here); from 0x3e each song is one record -- Latin-1 C strings, title at +0, author at
  +0x133, copyright at +0x582 -- followed by a u32 length and that many bytes of zlib-compressed
  RTF holding the words, with "Verse 1" / "Chorus" lines naming the sections.
