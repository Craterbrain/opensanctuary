import { describe, test, expect } from "bun:test";
import { parseParallelSlide, formatParallelSlide } from "../src/core/presentation_helpers.ts";
import { PluginManager } from "../src/core/plugins.ts";
import bibleContract from "../../../spec-db-rs/contracts/bible_parallel_display.schema.json";
import pluginContract from "../../../spec-db-rs/contracts/frontend_plugins.schema.json";

describe("Formal Contract Validation: bible_parallel_display.schema.json", () => {
    test("Schema definitions and thresholds match canonical standards", () => {
        expect(bibleContract.values.PARALLEL_COLUMN_DELIMITER).toBe("|||");
        expect(bibleContract.values.MAX_VERSES_PER_DUAL_SLIDE).toBe(2);
        expect(bibleContract.values.BACKWARD_COMPATIBLE_FALLBACK).toBe(true);
        expect(bibleContract.required).toContain("PARALLEL_COLUMN_DELIMITER");
    });

    test("parseParallelSlide output adheres strictly to ParallelSlideContent schema properties", () => {
        const primaryText = "For God so loved the world, that he gave his only begotten Son.";
        const secondaryText = "Car Dieu a tellement aimé le monde qu'il a donné son Fils unique.";
        const wirePayload = formatParallelSlide("KJV", primaryText, "LSG", secondaryText);

        expect(wirePayload).toContain(bibleContract.values.PARALLEL_COLUMN_DELIMITER);

        const parsed = parseParallelSlide(wirePayload);
        const requiredProps = bibleContract.definitions.ParallelSlideContent.required;

        // Verify every required property in contract exists on parsed object
        for (const prop of requiredProps) {
            expect(parsed).toHaveProperty(prop);
        }

        expect(typeof parsed.isParallel).toBe("boolean");
        expect(parsed.isParallel).toBe(true);
        expect(parsed.primaryHeader).toBe("KJV");
        expect(parsed.secondaryHeader).toBe("LSG");
        expect(parsed.primaryText).toBe(primaryText);
        expect(parsed.secondaryText).toBe(secondaryText);
    });

    test("Backward compatible fallback: non-parallel slides yield valid fallback schema object", () => {
        const plainText = "The Lord is my shepherd; I shall not want.";
        const parsed = parseParallelSlide(plainText);

        expect(parsed.isParallel).toBe(false);
        expect(parsed.leftText).toBe(plainText);
        expect(parsed.rightText).toBe("");
        expect(parsed.primaryText).toBe(plainText);
        expect(parsed.secondaryText).toBe("");
    });
});

describe("Formal Contract Validation: frontend_plugins.schema.json", () => {
    test("Schema invariants enforce isolated keyring vault prefix", () => {
        expect(pluginContract.values.PLUGIN_KEYRING_PREFIX).toBe("OpenSanctuary:Plugin:");
        expect(pluginContract.values.CRASH_BOUNDARY_ENABLED).toBe(true);
        expect(pluginContract.values.ROOT_KEYRING_MASKING).toBe(true);
    });

    test("PluginManager isolates vault namespace conforming to contract regex", () => {
        const manager = new PluginManager();
        const testPluginName = "church_ccli_sync";
        let capturedContext: any = null;

        const dummyPlugin = {
            name: testPluginName,
            version: "1.0.0",
            init: (ctx: any) => {
                capturedContext = ctx;
            }
        };

        const registered = manager.registerPlugin(testPluginName, dummyPlugin);
        expect(registered).toBe(true);
        expect(capturedContext).not.toBeNull();

        // Check keyring namespace pattern
        const vaultRegex = new RegExp(pluginContract.definitions.PluginKeyringVault.properties.service.pattern);
        expect(capturedContext.pluginKeyring.service).toMatch(vaultRegex);
        expect(capturedContext.pluginKeyring.service).toBe(`OpenSanctuary:Plugin:${testPluginName}`);
        // Ensure root keyring is masked to plugin vault
        expect(capturedContext.keyring).toBe(capturedContext.pluginKeyring);
    });

    test("Crash boundary: failing plugin init() does not throw or crash host", () => {
        const manager = new PluginManager();
        const failingPlugin = {
            name: "broken_plugin",
            version: "0.1.0",
            init: () => {
                throw new Error("Simulated third-party plugin fatal exception");
            }
        };

        expect(() => {
            const success = manager.registerPlugin("broken_plugin", failingPlugin);
            expect(success).toBe(false);
        }).not.toThrow();
    });
});
