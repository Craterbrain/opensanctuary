import { test, expect, describe, beforeEach, afterEach } from "bun:test";
import { api, apiFetch, ApiError } from "../src/core/api_client";

describe("Unified API Client: apiFetch & api helpers", () => {
  const originalFetch = globalThis.fetch;

  afterEach(() => {
    globalThis.fetch = originalFetch;
  });

  test("api.get performs GET request and parses JSON", async () => {
    globalThis.fetch = (async (url: string, init?: RequestInit) => {
      expect(init?.method).toBe("GET");
      return new Response(JSON.stringify({ success: true, items: ["hymn1", "hymn2"] }), {
        status: 200,
        headers: { "Content-Type": "application/json" }
      });
    }) as any;

    const res = await api.get<{ success: boolean; items: string[] }>("/api/test");
    expect(res.success).toBe(true);
    expect(res.items.length).toBe(2);
  });

  test("api.post automatically serializes JSON object body and sets header", async () => {
    globalThis.fetch = (async (url: string, init?: RequestInit) => {
      expect(init?.method).toBe("POST");
      const headers = new Headers(init?.headers);
      expect(headers.get("Content-Type")).toBe("application/json");
      expect(init?.body).toBe(JSON.stringify({ action: "test" }));
      return new Response(JSON.stringify({ ok: true }), {
        status: 200,
        headers: { "Content-Type": "application/json" }
      });
    }) as any;

    const res = await api.post("/api/action", { action: "test" });
    expect(res.ok).toBe(true);
  });

  test("api.sendCommand dispatches to /api/command with serialized payload", async () => {
    let capturedUrl = "";
    let capturedBody = "";
    globalThis.fetch = (async (url: string, init?: RequestInit) => {
      capturedUrl = url;
      capturedBody = init?.body as string;
      return new Response(JSON.stringify({ sequence_number: 42 }), {
        status: 200,
        headers: { "Content-Type": "application/json" }
      });
    }) as any;

    const cmd = { GoLive: { item_index: 0, slide_index: 1 } };
    const res = await api.sendCommand(cmd);
    expect(capturedUrl).toBe("/api/command");
    expect(capturedBody).toBe(JSON.stringify(cmd));
    expect(res.sequence_number).toBe(42);
  });

  test("throws ApiError with status and parsed error details on failure", async () => {
    globalThis.fetch = (async () => {
      return new Response(JSON.stringify({ error: "Item not found in schedule" }), {
        status: 404,
        statusText: "Not Found",
        headers: { "Content-Type": "application/json" }
      });
    }) as any;

    try {
      await api.get("/api/missing");
      expect.unreachable("Should have thrown ApiError");
    } catch (err: any) {
      expect(err).toBeInstanceOf(ApiError);
      expect(err.status).toBe(404);
      expect(err.message).toBe("Item not found in schedule");
      expect(err.body.error).toBe("Item not found in schedule");
    }
  });

  test("high-level resource methods route to expected endpoints", async () => {
    const calls: string[] = [];
    globalThis.fetch = (async (url: string) => {
      calls.push(url);
      return new Response(JSON.stringify([]), {
        status: 200,
        headers: { "Content-Type": "application/json" }
      });
    }) as any;

    await api.themes.list();
    await api.media.list();
    await api.settings.get();
    await api.network.info();

    expect(calls).toEqual([
      "/api/themes",
      "/api/media",
      "/api/settings",
      "/api/network/info"
    ]);
  });

  test("handles 204 No Content and empty responses gracefully without JSON parse error", async () => {
    globalThis.fetch = (async () => {
      return new Response("", {
        status: 204,
        statusText: "No Content"
      });
    }) as any;

    const res = await api.delete("/api/themes/test");
    expect(res).toBeUndefined();
  });
});
