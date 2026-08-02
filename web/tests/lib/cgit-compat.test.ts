import { describe, expect, it } from "vitest";

import { redirectFor } from "@/lib/cgit-compat";

// Same case table as `api/src/cgit_compat.rs`'s unit tests — the two must
// change together (see the doc comment on `redirectFor`).
describe("redirectFor", () => {
  it("redirects a bare .git suffix to the repo page", () => {
    expect(redirectFor("/axgit.git", "")).toBe("/axgit");
  });

  it("redirects the commit query shape to the commit page", () => {
    const sha = "abc123def456";
    expect(redirectFor("/axgit.git/commit/", `id=${sha}`)).toBe(`/axgit/commit/${sha}`);
    expect(redirectFor("/axgit/commit", `id=${sha}`)).toBe(`/axgit/commit/${sha}`);
  });

  it("redirects the diff query shape to the commit page", () => {
    const sha = "abc123def456";
    expect(redirectFor("/axgit.git/diff/", `id=${sha}`)).toBe(`/axgit/commit/${sha}`);
  });

  it("redirects log with h to the log page with a ref query", () => {
    expect(redirectFor("/axgit.git/log/", "h=main")).toBe("/axgit/log?ref=main");
    expect(redirectFor("/axgit/log", "h=main")).toBe("/axgit/log?ref=main");
  });

  it("redirects .git refs dropping the query", () => {
    expect(redirectFor("/axgit.git/refs/", "h=main")).toBe("/axgit/refs");
  });

  it("strips .git elsewhere and preserves the query", () => {
    expect(redirectFor("/axgit.git/tree/src/main.rs", "id=abc123")).toBe("/axgit/tree/src/main.rs?id=abc123");
  });

  it("leaves native routes alone", () => {
    for (const [pathname, query] of [
      ["/", ""],
      ["/axgit", ""],
      ["/axgit/", ""],
      ["/axgit/log", ""],
      ["/axgit/log/", ""],
      ["/axgit/refs", ""],
      ["/axgit/commit/abc123", ""],
      ["/axgit/tree/src", ""],
    ] as const) {
      expect(redirectFor(pathname, query)).toBeNull();
    }
  });

  it("does not redirect commit without a valid id", () => {
    expect(redirectFor("/axgit/commit", "")).toBeNull();
    expect(redirectFor("/axgit/commit", "id=not-a-sha")).toBeNull();
    expect(redirectFor("/axgit/commit", "id=")).toBeNull();
  });

  it("does not redirect log without h when .git is absent", () => {
    expect(redirectFor("/axgit/log", "other=1")).toBeNull();
  });

  it("still strips .git from log without h", () => {
    expect(redirectFor("/axgit.git/log", "")).toBe("/axgit/log");
  });

  it("does not treat a bare .git as a .git suffix", () => {
    expect(redirectFor("/.git", "")).toBeNull();
    expect(redirectFor("/.git/tree/src", "")).toBeNull();
  });
});
