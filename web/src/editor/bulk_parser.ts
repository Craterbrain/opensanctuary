/**
 * Bulk Text Parsing Engine for OpenSanctuary Slide Editor
 * Pure, side-effect-free utilities for lyrics, scripture, and presentation parsing.
 */

export type BulkSplitRule = 'paragraphs' | 'lines-2' | 'lines-4' | 'tag-boundary' | 'custom-delimiter';

export interface BulkParseOptions {
  splitRule?: BulkSplitRule;
  customDelimiter?: string;
  editorType?: 'song' | 'presentation' | 'scripture';
  autoCleanChords?: boolean;
  extractMetadata?: boolean;
}

export interface ExtractedMetadata {
  title?: string;
  author?: string;
  copyright?: string;
  ccli_number?: string;
}

export interface BulkParsedSlide {
  text: string;
  tag?: string;
  label?: string;
  header?: string;
}

export interface BulkParseResult {
  slides: BulkParsedSlide[];
  metadata: ExtractedMetadata;
}

// Single chord token regex: e.g. G, Em7, C#m, Bbmaj7, D/F#, Asus4, F#m7b5, C7b9
const CHORD_TOKEN_REGEX = /^[A-G][#b]?(?:m|maj|min|dim|aug|sus|add|[0-9]|M|[#b][0-9])*(?:\/[A-G][#b]?)?$/;

// Performance annotations and musical cues (never match bare section headers like "Verse 1" or "Chorus")
const CUE_PATTERNS = [
  /^\s*\(?\s*(?:repeat\s+(?:chorus|verse|bridge|tag|outro|refrain)|(?:chorus|verse|bridge|tag|outro|refrain)\s*(?:x\s*\d+|\d+\s*x))\s*\)?\s*$/i,
  /^\s*\(?\s*(?:repeat\s+)?\d+\s*x\s*\)?\s*$/i,
  /^\s*\(?\s*x\s*\d+\s*\)?\s*$/i,
  /^\s*[\(\[]?\s*(?:instrumental|solo|guitar\s+solo|interlude|instrumental\s+break)\s*[\)\]]?\s*$/i,
  /^\s*capo\s*:?\s*\d+\s*$/i,
  /^\s*bpm\s*:?\s*\d+\s*$/i,
  /^\s*tempo\s*:?\s*\d+\s*$/i,
  /^\s*key\s*:?\s*[a-g][#b]?\s*(?:major|minor|m)?\s*$/i,
];

/**
 * Checks if a trimmed line is purely musical chords (e.g. "G   D/F#   Em7   Cadd9" or "| C | G | D |")
 */
export function isChordLine(line: string): boolean {
  const trimmed = line.trim();
  if (!trimmed) return false;

  // Split line by whitespace and bar characters
  const rawTokens = trimmed.replace(/[|:()[\]]/g, ' ').split(/\s+/).filter(t => t.length > 0);
  if (rawTokens.length === 0) return false;

  // Filter out standalone numbering or slashes
  const validTokens = rawTokens.filter(t => t !== '/' && t !== '-');
  if (validTokens.length === 0) return false;

  return validTokens.every(token => CHORD_TOKEN_REGEX.test(token));
}

/**
 * Checks if a trimmed line is a performance cue / annotation (e.g. "(Repeat 2x)", "[Instrumental]", "Capo 3")
 */
export function isCueLine(line: string): boolean {
  const trimmed = line.trim();
  if (!trimmed) return false;
  return CUE_PATTERNS.some(pat => pat.test(trimmed));
}

/**
 * Strips chord brackets (e.g. [Em7]), standalone chord lines, capo/BPM headers,
 * and performance cues from lyrics/presentation text.
 */
export function stripChordsAndAnnotations(text: string): string {
  if (!text) return '';

  const lines = text.split(/\r?\n/);
  const filteredLines: string[] = [];

  for (const rawLine of lines) {
    const trimmed = rawLine.trim();

    // Check if line is a standalone chord line or performance cue
    if (isChordLine(trimmed) || isCueLine(trimmed)) {
      continue;
    }

    // Strip bracketed chords inline e.g. "Bless [G]the Lord [D/F#]O my soul" -> "Bless the Lord O my soul"
    // Also strip chords wrapped in square brackets: [C], [Em7], [G/B]
    let cleaned = rawLine.replace(/\[[A-G][#b]?(?:m|maj|min|dim|aug|sus|add|2|4|5|6|7|9|11|13|M7|m7|maj7|min7|dim7|aug7)*(?:\/[A-G][#b]?)?\]/gi, '');

    // Strip inline performance parentheticals like "(x2)" or "(repeat 2x)"
    cleaned = cleaned.replace(/\s*\((?:repeat\s*)?(?:x\s*\d+|\d+\s*x)\)/gi, '');

    // Collapse multiple spaces inside the line and trim leading/trailing whitespace
    const collapsed = cleaned.trim().replace(/[ \t]{2,}/g, ' ');
    filteredLines.push(collapsed);
  }

  // Remove excessive consecutive blank lines (max 2 consecutive newlines)
  return filteredLines.join('\n').replace(/\n{3,}/g, '\n\n').trim();
}

/**
 * Detects CCLI, Author, and Copyright lines from lyrics/text.
 * Strips them from the text so they do not produce stray slides, and returns the extracted metadata.
 */
export function extractSongMetadata(text: string): { cleanedText: string; metadata: ExtractedMetadata } {
  if (!text) return { cleanedText: '', metadata: {} };

  const lines = text.split(/\r?\n/);
  const cleanedLines: string[] = [];
  const metadata: ExtractedMetadata = {};

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (!line) {
      cleanedLines.push(lines[i]);
      continue;
    }

    // CCLI Number detection
    const ccliMatch = line.match(/^(?:CCLI\s*(?:Song\s*)?(?:#|No\.?|Number)?\s*:?\s*)(\d+)/i);
    if (ccliMatch) {
      metadata.ccli_number = ccliMatch[1].trim();
      continue;
    }

    // Author / Writer / Music / Lyrics detection
    const authorMatch = line.match(/^(?:Words\s+(?:&|and)\s+Music\s+by|Music\s+(?:&|and)\s+Words\s+by|Words\s+by|Music\s+by|Written\s+by|Author\s*:?|Words\s*:|Music\s*:|Lyrics\s*(?:by)?\s*:?)\s+(.+)/i);
    if (authorMatch) {
      const authorVal = authorMatch[1].trim();
      metadata.author = metadata.author ? `${metadata.author}, ${authorVal}` : authorVal;
      continue;
    }

    // Copyright detection
    const copyrightMatch = line.match(/^(?:©|Copyright|Admin\.\s+by|Public\s+Domain)\s*(.*)/i);
    if (copyrightMatch) {
      metadata.copyright = line;
      continue;
    }

    cleanedLines.push(lines[i]);
  }

  const cleanedText = cleanedLines.join('\n').trim();
  return { cleanedText, metadata };
}

export interface DetectedSection {
  tag: string;
  label: string;
  isHeader: boolean;
}

/**
 * Recognizes liturgical or presentation section headers:
 * e.g. "Verse 1", "[Verse 2]", "Chorus", "Chorus 2", "Bridge", "Pre-Chorus", "Intro", "Ending", "Tag", "Point 1"
 */
export function detectSectionHeader(rawLine: string): DetectedSection | null {
  if (!rawLine) return null;
  const line = rawLine.trim().replace(/^[\[(]/, '').replace(/[\]):]$/, '').trim();

  // 1. Verse: Verse 1, V1, Verse One, etc.
  const vMatch = line.match(/^(?:Verse|V)\s*(\d+)?$/i);
  if (vMatch) {
    const num = vMatch[1] || '1';
    return { tag: `V${num}`, label: `Verse ${num}`, isHeader: true };
  }

  // 2. Chorus: Chorus, Chorus 1, Chorus 2, C, C1, C2
  const cMatch = line.match(/^(?:Chorus|C)\s*(\d+)?$/i);
  if (cMatch) {
    const num = cMatch[1];
    const tag = num && num !== '1' ? `C${num}` : 'C';
    const label = num ? `Chorus ${num}` : 'Chorus';
    return { tag, label, isHeader: true };
  }

  // 3. Bridge: Bridge, Bridge 1, Bridge 2, B, B1
  const bMatch = line.match(/^(?:Bridge|B)\s*(\d+)?$/i);
  if (bMatch) {
    const num = bMatch[1];
    const tag = num && num !== '1' ? `B${num}` : 'B';
    const label = num ? `Bridge ${num}` : 'Bridge';
    return { tag, label, isHeader: true };
  }

  // 4. Pre-Chorus: Pre-Chorus, PreChorus, PC, P
  const pMatch = line.match(/^(?:Pre-?Chorus|PC|Pre\s*Chorus)\s*(\d+)?$/i);
  if (pMatch) {
    const num = pMatch[1];
    const tag = num && num !== '1' ? `P${num}` : 'P';
    const label = num ? `Pre-Chorus ${num}` : 'Pre-Chorus';
    return { tag, label, isHeader: true };
  }

  // 5. Intro: Intro, Intro 1, I, I1
  const iMatch = line.match(/^(?:Intro|I)\s*(\d+)?$/i);
  if (iMatch) {
    const num = iMatch[1];
    const tag = num && num !== '1' ? `I${num}` : 'I';
    const label = num ? `Intro ${num}` : 'Intro';
    return { tag, label, isHeader: true };
  }

  // 6. Ending / Outro: Ending, Outro, E, E1
  const eMatch = line.match(/^(?:Ending|Outro|E)\s*(\d+)?$/i);
  if (eMatch) {
    const num = eMatch[1];
    const tag = num && num !== '1' ? `E${num}` : 'E';
    const label = num ? `Ending ${num}` : 'Ending';
    return { tag, label, isHeader: true };
  }

  // 7. Tag: Tag, Tag 1
  const tagMatch = line.match(/^Tag\s*(\d+)?$/i);
  if (tagMatch) {
    const num = tagMatch[1];
    const tag = num && num !== '1' ? `Tag${num}` : 'Tag';
    const label = num ? `Tag ${num}` : 'Tag';
    return { tag, label, isHeader: true };
  }

  // 8. Interlude / Refrain
  if (/^Interlude/i.test(line)) {
    return { tag: 'Int', label: 'Interlude', isHeader: true };
  }
  if (/^Refrain/i.test(line)) {
    return { tag: 'Ref', label: 'Refrain', isHeader: true };
  }

  // 9. Presentation Point or Slide: "Slide 1", "Point 1"
  const slideMatch = line.match(/^(?:Slide|Point)\s*(\d+)$/i);
  if (slideMatch) {
    const num = slideMatch[1];
    return { tag: `S${num}`, label: `${line}`, isHeader: true };
  }

  return null;
}

/**
 * Splits Scripture text with verse numbers (e.g. "1 In the beginning... 2 And the earth...") into verse slides.
 */
function splitScriptureByVerses(text: string): BulkParsedSlide[] {
  // Matches verse numbers like "1 ", " 2 ", "\n3 "
  const verseRegex = /(?:^|\s+)(\d{1,3})\s+/g;
  const matches: { index: number; verseNum: string; length: number }[] = [];
  let m: RegExpExecArray | null;

  while ((m = verseRegex.exec(text)) !== null) {
    matches.push({ index: m.index, verseNum: m[1], length: m[0].length });
  }

  if (matches.length <= 1) {
    // Not verse-numbered scripture, fall back to blank-line paragraphs
    return splitByParagraphs(text);
  }

  const slides: BulkParsedSlide[] = [];
  for (let i = 0; i < matches.length; i++) {
    const start = matches[i].index + matches[i].length;
    const end = i < matches.length - 1 ? matches[i + 1].index : text.length;
    const body = text.substring(start, end).trim();
    if (body) {
      const vNum = matches[i].verseNum;
      slides.push({
        text: `${vNum} ${body}`,
        tag: `V${vNum}`,
        label: `Verse ${vNum}`,
      });
    }
  }

  return slides.length > 0 ? slides : splitByParagraphs(text);
}

/**
 * Splits text into paragraphs by blank lines (1+ empty lines)
 */
function splitByParagraphs(text: string): BulkParsedSlide[] {
  const blocks = text.split(/\n\s*\n+/).map(b => b.trim()).filter(b => b.length > 0);
  const slides: BulkParsedSlide[] = [];

  for (const blk of blocks) {
    const lines = blk.split(/\r?\n/);
    const firstLine = lines[0].trim();
    const section = detectSectionHeader(firstLine);

    if (section) {
      const body = lines.slice(1).join('\n').trim();
      slides.push({
        text: body || firstLine,
        tag: section.tag,
        label: section.label,
        header: section.label,
      });
    } else {
      slides.push({
        text: blk,
      });
    }
  }

  return slides;
}

/**
 * Splits text every N non-empty lines, keeping section tags associated with their slides.
 */
function splitByLineCount(text: string, linesPerSlide: number): BulkParsedSlide[] {
  const paragraphs = text.split(/\n\s*\n+/).map(b => b.trim()).filter(b => b.length > 0);
  const slides: BulkParsedSlide[] = [];

  for (const para of paragraphs) {
    const allLines = para.split(/\r?\n/).map(l => l.trim()).filter(l => l.length > 0);
    if (allLines.length === 0) continue;

    let currentTag: string | undefined;
    let currentLabel: string | undefined;
    let startIdx = 0;

    const section = detectSectionHeader(allLines[0]);
    if (section) {
      currentTag = section.tag;
      currentLabel = section.label;
      startIdx = 1;
    }

    const contentLines = allLines.slice(startIdx);
    if (contentLines.length === 0) {
      slides.push({
        text: allLines[0],
        tag: currentTag,
        label: currentLabel,
      });
      continue;
    }

    for (let i = 0; i < contentLines.length; i += linesPerSlide) {
      const chunk = contentLines.slice(i, i + linesPerSlide).join('\n');
      const partNum = Math.floor(i / linesPerSlide) + 1;
      const totalParts = Math.ceil(contentLines.length / linesPerSlide);
      const partSuffix = totalParts > 1 ? ` (${partNum}/${totalParts})` : '';

      slides.push({
        text: chunk,
        tag: currentTag,
        label: currentLabel ? `${currentLabel}${partSuffix}` : undefined,
      });
    }
  }

  return slides;
}

/**
 * Splits text strictly on section tag boundaries.
 */
function splitByTagBoundary(text: string): BulkParsedSlide[] {
  const rawLines = text.split(/\r?\n/);
  const slides: BulkParsedSlide[] = [];

  let curSection: DetectedSection | null = null;
  let curLines: string[] = [];

  const flush = () => {
    const body = curLines.join('\n').trim();
    if (body || curSection) {
      slides.push({
        text: body || (curSection ? curSection.label : ''),
        tag: curSection?.tag,
        label: curSection?.label,
        header: curSection?.label,
      });
    }
    curLines = [];
  };

  for (const rawLine of rawLines) {
    const section = detectSectionHeader(rawLine.trim());
    if (section) {
      flush();
      curSection = section;
    } else {
      curLines.push(rawLine);
    }
  }
  flush();

  return slides.length > 0 ? slides : splitByParagraphs(text);
}

/**
 * Splits text by custom delimiter (e.g. "---" or "===")
 */
function splitByCustomDelimiter(text: string, delimiter: string = '---'): BulkParsedSlide[] {
  const cleanDelim = delimiter.trim() || '---';
  const escaped = cleanDelim.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const regex = new RegExp(`(?:^|\\n)\\s*${escaped}\\s*(?:\\n|$)`, 'g');

  const blocks = text.split(regex).map(b => b.trim()).filter(b => b.length > 0);
  const slides: BulkParsedSlide[] = [];

  for (const blk of blocks) {
    const lines = blk.split(/\r?\n/);
    const section = detectSectionHeader(lines[0].trim());
    if (section) {
      const body = lines.slice(1).join('\n').trim();
      slides.push({
        text: body || lines[0],
        tag: section.tag,
        label: section.label,
        header: section.label,
      });
    } else {
      slides.push({ text: blk });
    }
  }

  return slides;
}

/**
 * Main entrance: parses raw bulk text into structured slides with tags, labels,
 * and extracted song metadata.
 */
export function splitTextToSlides(rawText: string, options: BulkParseOptions = {}): BulkParseResult {
  if (!rawText || !rawText.trim()) {
    return { slides: [], metadata: {} };
  }

  let text = rawText.trim();
  let metadata: ExtractedMetadata = {};

  // 1. Extract metadata if requested (default true)
  if (options.extractMetadata !== false) {
    const metaRes = extractSongMetadata(text);
    text = metaRes.cleanedText;
    metadata = metaRes.metadata;
  }

  // 2. Clean chords if requested (default false in splitTextToSlides unless opted-in)
  if (options.autoCleanChords) {
    text = stripChordsAndAnnotations(text);
  }

  // 3. Select splitting strategy
  let slides: BulkParsedSlide[] = [];
  const rule = options.splitRule || 'paragraphs';

  if (options.editorType === 'scripture') {
    slides = splitScriptureByVerses(text);
  } else {
    switch (rule) {
      case 'lines-2':
        slides = splitByLineCount(text, 2);
        break;
      case 'lines-4':
        slides = splitByLineCount(text, 4);
        break;
      case 'tag-boundary':
        slides = splitByTagBoundary(text);
        break;
      case 'custom-delimiter':
        slides = splitByCustomDelimiter(text, options.customDelimiter || '---');
        break;
      case 'paragraphs':
      default:
        slides = splitByParagraphs(text);
        break;
    }
  }

  // Filter out any blank slides resulting from edge-case double delimiters
  slides = slides.filter(s => s.text.trim().length > 0 || (s.tag && s.tag.length > 0));

  return { slides, metadata };
}
