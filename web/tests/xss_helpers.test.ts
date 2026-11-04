import { describe, test, expect } from "bun:test";
import { escapeCssUrl, escapeHtml, safeHttpUrl } from "../src/core/presentation_helpers.ts";

describe("escapeCssUrl", () => {
    test("cannot terminate url('...') or the style attribute", () => {
        const out = escapeCssUrl(`x') ;} " onerror="alert(1)`);
        expect(out).not.toContain("'");
        expect(out).not.toContain('"');
        expect(out).not.toContain(")");
        expect(out).not.toContain(" ");
    });
    test("leaves ordinary URLs intact", () => {
        expect(escapeCssUrl("https://img.example.com/a/b.jpg?x=1")).toBe("https://img.example.com/a/b.jpg?x=1".replace("&", "&amp;"));
    });
    test("handles null/undefined", () => {
        expect(escapeCssUrl(null)).toBe("");
        expect(escapeCssUrl(undefined)).toBe("");
    });
});

describe("safeHttpUrl", () => {
    test("allows http and https", () => {
        expect(safeHttpUrl("https://example.com/x")).toBe("https://example.com/x");
        expect(safeHttpUrl("http://example.com/")).toBe("http://example.com/");
    });
    test("rejects javascript:, data:, vbscript:, relative and non-strings", () => {
        expect(safeHttpUrl("javascript:alert(1)")).toBeNull();
        expect(safeHttpUrl(" JavaScript:alert(1)")).toBeNull();
        expect(safeHttpUrl("data:text/html,<script>alert(1)</script>")).toBeNull();
        expect(safeHttpUrl("vbscript:x")).toBeNull();
        expect(safeHttpUrl("/relative")).toBeNull();
        expect(safeHttpUrl(null)).toBeNull();
        expect(safeHttpUrl(42)).toBeNull();
    });
});

describe("escapeHtml", () => {
    test("escapes all five characters, so the text is safe in element content and in either kind of quoted attribute", () => {
        expect(escapeHtml(`<script>alert("xss & fun")</script>`)).toBe("&lt;script&gt;alert(&quot;xss &amp; fun&quot;)&lt;/script&gt;");
        expect(escapeHtml(`O'Brien's <verse>`)).toBe("O&#39;Brien&#39;s &lt;verse&gt;");
        for (const payload of [`"><img src=x onerror=alert(1)>`, `#fff" autofocus onfocus="alert(1)`, `x' onmouseover='alert(1)`]) {
            expect(escapeHtml(payload)).not.toMatch(/[<>"']/);
        }
    });
    test("null and undefined become empty text", () => {
        expect(escapeHtml(null)).toBe("");
        expect(escapeHtml(undefined)).toBe("");
    });
});
