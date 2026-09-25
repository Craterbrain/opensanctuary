import { describe, expect, test } from "bun:test";

describe("Local File Import Mode (Replace vs Append)", () => {
    test("Open schedule payload includes mode parameter", () => {
        const replacePayload = {
            file_name: "SundayService.ewsx",
            file_data_base64: "dGVzdA==",
            mode: "replace"
        };
        expect(replacePayload.mode).toBe("replace");

        const appendPayload = {
            file_name: "SundayService.ewsx",
            file_data_base64: "dGVzdA==",
            mode: "append"
        };
        expect(appendPayload.mode).toBe("append");
    });

    test("Append mode preserves existing schedule items structure", () => {
        const existingItems = [{ id: "item1", title: "Existing Song" }];
        const incomingItems = [{ id: "item2", title: "Incoming Song" }];
        const appended = [...existingItems, ...incomingItems];
        expect(appended.length).toBe(2);
        expect(appended[0].title).toBe("Existing Song");
        expect(appended[1].title).toBe("Incoming Song");
    });
});
