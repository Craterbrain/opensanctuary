# The public-domain hymn library

_Last edited: 2026-10-27_

The hymns live in the Bibles repository (`songs/public-domain-hymns.json` in Craterbrain/opensanctuary-bibles, with this document as its `SONGS.md`).
They are not bundled with the program: the first-run wizard's Songs step and **Import > Public-domain hymns** download them (`src/storage/pd_songs.rs`)
and add them by id and by title+author, so a song you already have, or edited, is never overwritten or duplicated. The program re-checks the download and
refuses it if any hymn isn't marked "Public Domain" or carries a CCLI number.

## The rule

A hymn is included only on **evidence**, never because its copyright field is blank:

* it has a **named historical author**, with the words first published **before 1931** and the author **dead by 1955** (so the text is public domain in the
  United States and in the countries that protect for life plus 70 years). Anything printed before 1881 qualifies on its date alone. Translators count: a
  translation is credited and held to the same test; or
* it is **anonymous or traditional but clearly ancient**, and says how we know in its `basis` field (the tests refuse an anonymous or traditional entry without one).

Every song also carries its **source** where it came from a reference page.

A blank copyright is not evidence. The app files a song with no copyright and no CCLI number under "public domain" by default
(`Database::is_public_domain_metadata`), which is how modern choruses ended up in the old `songs/public_domain.db`. Hymnary.org does the same: it shows
"Public Domain" for any text with no rights holder recorded, so its label is not proof either.

## Where the hymns came from

* 147 of the 178 songs in the original `songs/public_domain.db` passed the rule above (typos in titles and authors fixed).
* **74 more** were added from **The Cyber Hymnal** (hymntime.com), the one reliable public source found. Its own terms say material without a copyright notice is
  believed to be public domain and anything copyrighted or used by permission is clearly marked; every page was checked for such a notice, and for the author's
  dates, before the hymn was taken. Its hymn pages are fetched one at a time; it offers no bulk lyrics download. Each added song's `source` names the page.
  The Open Hymnal Project, the other obvious candidate, is gone: its domain now serves an unrelated site and its GitHub mirror is empty.
  Hymnary.org was used to check rights holders but not as a source of "public domain" status: it tags anything with no recorded owner that way.
* Hymns the Cyber Hymnal lists under "Frequently Requested" are ones it cannot carry because they are copyrighted, so that page is a handy list of what **not** to add.
* *Let Us Break Bread Together* is the traditional spiritual's text (first published 1925), replacing the modern adaptation that was left out.
* Texts are as the Cyber Hymnal prints them; refrains are repeated after each verse as in the printed hymn, and long stanzas are split over two slides.

Left out of the added set on purpose: *Jesus Paid It All*, *Showers of Blessing* and *Come, Thou Almighty King* (the page's attribution disagreed with the hymn
meant, or was disputed), *Lo, How a Rose E'er Blooming* (a verse by a translator whose dates could not be confirmed), *I Would Be True* (a co-author who died 1972),
*Day by Day* (a 1930s translation) and *My Hope Is Built on Nothing Less* (the page prints Mote's original six verses starting with one nobody sings first).

## What was left out of the original file (31 entries of the 178 in the old file)

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

### Truly traditional, but the file had the wrong words (now fixed)

*Let Us Break Bread Together* is an old spiritual (text first published 1925), so the real text is public domain. The old file held a modern adaptation ("we are
one ... with our face to the risen Son"), so that version was left out and the traditional text was added in its place.

## Typos fixed on the way

Wonderul, Unkown, Roberty Lowry, Chrisitan F. Gellert, Rosetti, Philips Brooks, Mantague, Annointed, "I Will Song of the Mercies", "Philipp Bliss".

## Changing the library

Edit `songs/public-domain-hymns.json` in the Bibles repository and run `python3 scripts/validate_songs.py --update`; its CI check (`validate_songs.py`)
refuses test data, duplicates, a missing "Public Domain" line, a rights notice inside the lyrics, and any song with no named author and no stated `basis`.
Add a song only with a source for who wrote it and when. A change goes live as soon as it is on the repository's `main` branch.
