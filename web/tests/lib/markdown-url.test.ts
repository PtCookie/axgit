import { describe, expect, it } from "vitest";

import { isExternalUrl, resolveRepoPath } from "@/lib/markdown-url";

describe("isExternalUrl", () => {
  it("treats http(s)/mailto/data URLs as external", () => {
    expect(isExternalUrl("https://example.com/x")).toBe(true);
    expect(isExternalUrl("http://example.com/x")).toBe(true);
    expect(isExternalUrl("mailto:a@example.com")).toBe(true);
    expect(isExternalUrl("data:image/png;base64,AAAA")).toBe(true);
  });

  it("treats protocol-relative URLs and same-page anchors as external", () => {
    expect(isExternalUrl("//example.com/x")).toBe(true);
    expect(isExternalUrl("#section")).toBe(true);
  });

  it("treats a repo-relative reference as not external", () => {
    expect(isExternalUrl("./docs/x.md")).toBe(false);
    expect(isExternalUrl("images/logo.png")).toBe(false);
    expect(isExternalUrl("/docs/x.md")).toBe(false);
    expect(isExternalUrl("")).toBe(false);
  });
});

describe("resolveRepoPath", () => {
  it("resolves a plain relative reference against the root", () => {
    expect(resolveRepoPath("", "images/logo.png")).toBe("images/logo.png");
    expect(resolveRepoPath("", "./docs/x.md")).toBe("docs/x.md");
  });

  it("resolves a repo-root-relative reference (leading slash)", () => {
    expect(resolveRepoPath("", "/docs/x.md")).toBe("docs/x.md");
  });

  it("returns null when the reference escapes the repository root", () => {
    expect(resolveRepoPath("", "../escape.md")).toBeNull();
    expect(resolveRepoPath("", "../../escape.md")).toBeNull();
  });

  it("returns null for an external URL passed by mistake", () => {
    expect(resolveRepoPath("", "https://example.com/x")).toBeNull();
  });

  it("strips a query or hash suffix before resolving", () => {
    expect(resolveRepoPath("", "images/logo.png?raw=true")).toBe("images/logo.png");
    expect(resolveRepoPath("", "docs/x.md#section")).toBe("docs/x.md");
  });

  it("resolves relative to a non-root base directory", () => {
    expect(resolveRepoPath("docs", "./x.md")).toBe("docs/x.md");
    expect(resolveRepoPath("docs", "../top.md")).toBe("top.md");
    // Multi-segment bases: TreeView passes the listed directory when
    // resolving a symlink target, so `..` has to unwind one level at a time.
    expect(resolveRepoPath("src/lib", "../README.md")).toBe("src/README.md");
    expect(resolveRepoPath("src/lib", "../../README.md")).toBe("README.md");
    expect(resolveRepoPath("src/lib", "../../../escape.md")).toBeNull();
  });
});
