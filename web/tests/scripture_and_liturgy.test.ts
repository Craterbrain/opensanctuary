import { describe, expect, test } from "bun:test";
import {
    parseScriptureReference,
    matchLiturgicalSectionKey,
    escapeHtml
} from "../src/core/presentation_helpers.ts";

describe("Scripture Reference Parsing", () => {
    test("parses simple book, chapter, and verse: John 3:16", () => {
        const res = parseScriptureReference("John 3:16");
        expect(res.isReference).toBe(true);
        expect(res.book).toBe("John");
        expect(res.chapter).toBe(3);
        expect(res.verseStart).toBe(16);
        expect(res.verseEnd).toBe(16);
        expect(res.hasSpecificVerse).toBe(true);
    });

    test("parses verse range: 1 Corinthians 13:4-8", () => {
        const res = parseScriptureReference("1 Corinthians 13:4-8");
        expect(res.isReference).toBe(true);
        expect(res.book).toBe("1 Corinthians");
        expect(res.chapter).toBe(13);
        expect(res.verseStart).toBe(4);
        expect(res.verseEnd).toBe(8);
        expect(res.hasSpecificVerse).toBe(true);
    });

    test("parses chapter only: Psalm 23", () => {
        const res = parseScriptureReference("Psalm 23");
        expect(res.isReference).toBe(true);
        expect(res.book).toBe("Psalm");
        expect(res.chapter).toBe(23);
        expect(res.verseStart).toBeNull();
        expect(res.verseEnd).toBeNull();
        expect(res.hasSpecificVerse).toBe(false);
    });

    test("parses multi-word book: Song of Solomon 2:1", () => {
        const res = parseScriptureReference("Song of Solomon 2:1");
        expect(res.isReference).toBe(true);
        expect(res.book).toBe("Song of Solomon");
        expect(res.chapter).toBe(2);
        expect(res.verseStart).toBe(1);
    });

    test("returns isReference: false for empty or non-scripture search query", () => {
        expect(parseScriptureReference("").isReference).toBe(false);
        expect(parseScriptureReference("   ").isReference).toBe(false);
        expect(parseScriptureReference(null).isReference).toBe(false);
    });
});

describe("Liturgical Song Section Key Navigation", () => {
    test("maps liturgical shortcut keys to capitalized section codes", () => {
        expect(matchLiturgicalSectionKey('v')).toBe('V');
        expect(matchLiturgicalSectionKey('V')).toBe('V');
        expect(matchLiturgicalSectionKey('c')).toBe('C');
        expect(matchLiturgicalSectionKey('b')).toBe('B');
        expect(matchLiturgicalSectionKey('e')).toBe('E');
        expect(matchLiturgicalSectionKey('p')).toBe('P');
        expect(matchLiturgicalSectionKey('i')).toBe('I');
        expect(matchLiturgicalSectionKey('o')).toBe('O');
    });

    test("returns null for non-liturgical keys to prevent accidental command triggers", () => {
        expect(matchLiturgicalSectionKey('a')).toBeNull();
        expect(matchLiturgicalSectionKey('z')).toBeNull();
        expect(matchLiturgicalSectionKey('1')).toBeNull();
        expect(matchLiturgicalSectionKey('Enter')).toBeNull();
        expect(matchLiturgicalSectionKey('')).toBeNull();
    });
});

describe("HTML Sanitization (XSS Prevention)", () => {
    test("escapes HTML entities safely", () => {
        const raw = `<script>alert("xss & fun")</script>`;
        const safe = escapeHtml(raw);
        expect(safe).toBe('&lt;script&gt;alert(&quot;xss &amp; fun&quot;)&lt;/script&gt;');
    });

    test("handles null and undefined without throwing", () => {
        expect(escapeHtml(null)).toBe('');
        expect(escapeHtml(undefined)).toBe('');
    });

    test("escapes apostrophes, so escaped text is safe inside a single-quoted attribute too", () => {
        const raw = `O'Brien's "favorite" <verse>`;
        const safe = escapeHtml(raw);
        expect(safe).not.toContain("'");
        expect(safe).toBe('O&#39;Brien&#39;s &quot;favorite&quot; &lt;verse&gt;');
    });
});
