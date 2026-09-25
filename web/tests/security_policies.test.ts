import { describe, test, expect } from "bun:test";
import { SETTINGS_SCHEMA } from "../src/core/settings_schema.ts";
import { escapeHtml } from "../src/core/presentation_helpers.ts";

describe("Network Security Policies & Settings Schema", () => {
    test("SETTINGS_SCHEMA includes securityCspMode with balanced default options", () => {
        const cspDef = SETTINGS_SCHEMA.find(s => s.key === 'securityCspMode');
        expect(cspDef).toBeDefined();
        expect(cspDef!.category).toBe('network');
        expect(cspDef!.control).toBe('select');
        expect(cspDef!.options).toBeDefined();

        const vals = cspDef!.options!.map(o => o.value);
        expect(vals).toContain('balanced');
        expect(vals).toContain('strict');
        expect(vals).toContain('disabled');
    });

    test("SETTINGS_SCHEMA includes securityCorsMode with permissive and restricted options", () => {
        const corsDef = SETTINGS_SCHEMA.find(s => s.key === 'securityCorsMode');
        expect(corsDef).toBeDefined();
        expect(corsDef!.category).toBe('network');
        expect(corsDef!.control).toBe('select');

        const vals = corsDef!.options!.map(o => o.value);
        expect(vals).toContain('permissive');
        expect(vals).toContain('restricted');
    });

    test("SETTINGS_SCHEMA includes securityFrameOptions with sameorigin, deny, and disabled", () => {
        const frameDef = SETTINGS_SCHEMA.find(s => s.key === 'securityFrameOptions');
        expect(frameDef).toBeDefined();
        expect(frameDef!.category).toBe('network');
        expect(frameDef!.control).toBe('select');

        const vals = frameDef!.options!.map(o => o.value);
        expect(vals).toContain('sameorigin');
        expect(vals).toContain('deny');
        expect(vals).toContain('disabled');
    });
});

describe("Frontend XSS Sanitization Vectors", () => {
    test("escapeHtml neutralizes attribute breakout payload for image/video paths", () => {
        const maliciousPath = `"><img src=x onerror=alert(1)>`;
        const sanitized = escapeHtml(maliciousPath);
        expect(sanitized).not.toContain('>');
        expect(sanitized).not.toContain('<');
        expect(sanitized).not.toContain('"');
        expect(sanitized).toBe('&quot;&gt;&lt;img src=x onerror=alert(1)&gt;');
    });

    test("escapeHtml safely neutralizes stage alert payload with embedded tags", () => {
        const alertPayload = `<script>fetch('/api/command', {method:'POST', body:'...'})</script><b>FIRE</b>`;
        const sanitized = escapeHtml(alertPayload);
        expect(sanitized).not.toContain('<script>');
        expect(sanitized).toBe('&lt;script&gt;fetch(&#39;/api/command&#39;, {method:&#39;POST&#39;, body:&#39;...&#39;})&lt;/script&gt;&lt;b&gt;FIRE&lt;/b&gt;');
    });

    test("escapeHtml safely escapes monitor device names", () => {
        const monitorName = `Display 1 <img src=x onerror="steal()">`;
        const sanitized = escapeHtml(monitorName);
        expect(sanitized).not.toContain('<img');
        expect(sanitized).toBe('Display 1 &lt;img src=x onerror=&quot;steal()&quot;&gt;');
    });
});
