import { afterEach, describe, expect, it, vi } from "vitest";

import { ApiError, apiFetch } from "@/lib/api/client";

describe("apiFetch", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("parses a successful JSON response", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({ repos: [] }), { status: 200 })));

    await expect(apiFetch("/repos")).resolves.toEqual({ repos: [] });
  });

  it("throws an ApiError built from the error envelope", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation(
        () =>
          new Response(JSON.stringify({ error: { code: "repo_not_found", message: "repository not found" } }), {
            status: 404,
          }),
      ),
    );

    const error: unknown = await apiFetch("/repos/missing").catch((err: unknown) => err);
    expect(error).toBeInstanceOf(ApiError);
    expect(error).toMatchObject({ code: "repo_not_found", message: "repository not found", status: 404 });
  });

  it("falls back to a status-based ApiError for a non-JSON error body", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("<html>gateway error</html>", { status: 502 })));

    await expect(apiFetch("/repos")).rejects.toMatchObject({ code: "internal", status: 502 });
  });

  it("normalizes a network failure into an ApiError", async () => {
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("network down")));

    await expect(apiFetch("/repos")).rejects.toMatchObject({ code: "internal", status: 0 });
  });
});
