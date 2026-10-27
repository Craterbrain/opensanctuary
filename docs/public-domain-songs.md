# The bundled public-domain hymn library

_Last edited: 2026-10-27_

`assets/public_domain_songs.json` holds the hymns that ship inside the program (`src/storage/pd_songs.rs`). A brand-new library gets them once, and
**Import > Public-domain hymns** adds any that are missing on request. They are added by id and by title+author, so a song you already have, or edited, is never
overwritten or duplicated.

## The rule

A hymn is included only on **evidence**, never because its copyright field is blank:

* it has a **named historical author** (a text from before about 1930), or
* it is one of the few **anonymous but clearly ancient** carols and hymns, listed by name in the tests (`The First Nowell`, `Away in a Manger`,
  `Jesus Christ is Risen Today`).

A blank copyright is not evidence. The app files a song with no copyright and no CCLI number under "public domain" by default
(`Database::is_public_domain_metadata`), which is how modern choruses ended up in the old `songs/public_domain.db`. Hymnary.org does the same: it shows
"Public Domain" for any text with no rights holder recorded, so its label is not proof either.

## What was left out (31 entries of the 178 in the old file)

Researched 2026-10-27 against hymnary.org and publisher/chart pages. **Not one of the 28 real songs could be shown to be public domain as the old file had it.**
Two are test data (`Test Song Save`, `ufu`).

### A rights holder was found

| Song | Who holds the rights (source) |
|---|---|
| Bless the Lord, O My Soul | Andrae Crouch; © Bud John Songs, Inc. / ASCAP (hymnary LEVS1993/65); also credited © 1973 Lexicon Music |
| Here I Am to Worship | Tim Hughes; © 2001 Thankyou Music (admin. EMI Christian Music Publishing); CCLI 3266032 |
| Come Bless the Lord | Phil Lawson Johnston; © 1975 Kingsway's Thankyou Music (Complete Mission Praise 88) |
| The Virgin Mary Had a Baby Boy | text © 1945 Boosey & Co.; arrangement © 1993 GIA Publications (hymnary GC2/378) |
| He Is Lord | © 1977 Marvin V. Frey; authorship disputed (Steve Vest claims 1969); not public domain either way (hymnary comment 45693) |
| God of Grace and God of Glory | H. E. Fosdick; "Words by permission of Elinor Fosdick Downs" (hymnary UMH 577); first published 1932, so US public domain from 2028; author d. 1969 |
| The Lord Has Given | "Copyright control; Arr. © 1987 Phil Burt" (Complete Mission Praise 653) |
| Set My Spirit Free | "Copyright control; © 1987 Phil Burt" (Complete Mission Praise 595) |
| Peace Is Flowing Like a River | "Copyright control; Arr. © 1974 Celebration / Kingsway's Thankyou Music" (Complete Mission Praise 554); other books credit Carey Landry, © NALR |
| O Come, Let Us Adore Him (the "For He alone is worthy" stanzas) | Hope Publishing Company owns or administers it (This Far By Faith, 1999, hymnary TFBF1999/284); credited "Traditional" |
| Be Still and Know | text anonymous, but every printing found carries an arrangement copyright (© 1994 CRC Publications; © 1989 Hope) |
| In the Name of Jesus | arrangements © 1999 Augsburg Fortress (James M. Capers), © 2000 GIA (Walter Owens Jr.) |
| When the Spirit of the Lord | English version arranged © 1999 Kevin Mayhew (Andrew Moore); the Spanish original is anonymous |
| Holy, Holy, Holy Lord (God of power and might) | an ancient liturgical text, but every English setting found is © (GIA 1971/77, Hope 1995, Boosey & Hawkes 1997, Malted Milk 1989) |

### No rights holder found, but nothing shows it is public domain

Songbooks print these as "Anonymous" or "Traditional" and their earliest known printing is modern (1958 to 1999). Hymnary's "Public Domain" label on them has no
research behind it.

Heavenly Father I Appreciate You (earliest found 1979), His Name Is Higher (1988), Give Me Oil in My Lamp (credited A. Sevison; earliest found 1958),
God Is So Good, Great Is the Lord (the Psalm 48 chorus; a different song of that name is © 1985 Body Songs), How Lovely Is Thy Dwelling, I See the Lord
(Paul Oakley's song of that name is © Integrity, but the wording differs), Where Will I Go, Holy Holy Holy Is the Lord, The Lord Your God Is in Your Midst
(Zephaniah 3:17), For Unto Us a Child Is Born (several modern settings exist), A New Commandment (John 13:34 set many ways; the second copy is one church's own
slides), Mukti Dilaye (could not be matched to a source).

### Truly traditional, but the file had the wrong words

Let Us Break Bread Together is an old spiritual (text first published 1925), so the real text is public domain. The old file held a modern adaptation ("we are
one ... with our face to the risen Son"), so the song was left out. It can be added back from the 1925 text.

## Typos fixed on the way

Wonderul, Unkown, Roberty Lowry, Chrisitan F. Gellert, Rosetti, Philips Brooks, Mantague, Annointed, "I Will Song of the Mercies", "Philipp Bliss".

## Changing the library

Edit `assets/public_domain_songs.json`. The tests in `src/storage/pd_songs.rs` refuse test data, duplicates, a missing "Public Domain" line, a rights notice inside the
lyrics, and any song with no named author that isn't on the short ancient list. Add a song only with a source for who wrote it and when.
