import { afterEach, describe, expect, it, vi } from "vitest";

import { highlightCode, languageForFence, languageForPath } from "@/lib/format/highlight";

describe("languageForPath", () => {
  it("maps known extensions to a Shiki language id", () => {
    expect(languageForPath("src/main.rs")).toBe("rust");
    expect(languageForPath("src/App.tsx")).toBe("tsx");
    expect(languageForPath("README.md")).toBe("markdown");
  });

  it("matches Dockerfile case-insensitively regardless of extension", () => {
    expect(languageForPath("Dockerfile")).toBe("docker");
    expect(languageForPath("dockerfile")).toBe("docker");
  });

  it("returns undefined for an unmapped or missing extension", () => {
    expect(languageForPath("data.bin")).toBeUndefined();
    expect(languageForPath("Makefile")).toBeUndefined();
    expect(languageForPath("noextension")).toBeUndefined();
  });
});

describe("languageForFence", () => {
  it("accepts a canonical Shiki language id", () => {
    expect(languageForFence("rust")).toBe("rust");
    expect(languageForFence("typescript")).toBe("typescript");
  });

  it("accepts an extension alias, same table as languageForPath", () => {
    expect(languageForFence("rs")).toBe("rust");
    expect(languageForFence("ts")).toBe("typescript");
    expect(languageForFence("sh")).toBe("bash");
  });

  it("is case-insensitive and takes only the first whitespace-separated token", () => {
    expect(languageForFence("JS title=example.js")).toBe("javascript");
  });

  it("returns undefined for an unmapped or empty info string", () => {
    expect(languageForFence("brainfuck")).toBeUndefined();
    expect(languageForFence("")).toBeUndefined();
    expect(languageForFence("   ")).toBeUndefined();
  });
});

describe("highlightCode", () => {
  it("tokenizes a small supported file, preserving line content", async () => {
    const code = 'fn main() {\n    println!("hi");\n}';
    const lines = await highlightCode(code, "src/main.rs");
    if (lines === null) throw new Error("expected highlightCode to return tokens");

    expect(lines).toHaveLength(3);
    const rebuilt = lines.map((line) => line.map((token) => token.content).join("")).join("\n");
    expect(rebuilt).toBe(code);
    // Every token carries at least a light-theme color plus the dark-theme
    // CSS variable (`global.css`'s `.dark .shiki-code span` override).
    for (const line of lines) {
      for (const token of line) {
        if (token.content.trim() === "") continue;
        expect(token.style).toHaveProperty("color");
      }
    }
  });

  it("returns null for a language with no loader entry", async () => {
    expect(await highlightCode("some content", "data.bin")).toBeNull();
    expect(await highlightCode("some content", "Makefile")).toBeNull();
  });

  it("returns null above the size threshold", async () => {
    const huge = "a".repeat(600 * 1024);
    expect(await highlightCode(huge, "src/main.rs")).toBeNull();
  });

  it("returns null above the line-count threshold", async () => {
    const manyLines = "a\n".repeat(5001);
    expect(await highlightCode(manyLines, "src/main.rs")).toBeNull();
  });
});

describe("configured syntax themes", () => {
  function clearThemeMetas() {
    document.head.querySelectorAll('meta[name^="axgit:syntax-theme-"]').forEach((meta) => {
      meta.remove();
    });
  }

  /** Highlights `fn` with the given `axgit:syntax-theme-*` metas in the
   *  document, and returns its inline style. The highlighter is a module-level
   *  singleton that reads the metas once, so each case needs a fresh module
   *  instance — hence `resetModules` plus a dynamic import. */
  async function keywordStyle(metas: { light?: string; dark?: string }) {
    clearThemeMetas();
    for (const [mode, content] of Object.entries(metas)) {
      const meta = document.createElement("meta");
      meta.name = `axgit:syntax-theme-${mode}`;
      meta.content = content;
      document.head.append(meta);
    }

    vi.resetModules();
    const { highlightCode: fresh } = await import("@/lib/format/highlight");
    const lines = await fresh("fn main() {}", "src/main.rs");
    if (lines === null) throw new Error("expected tokens");

    const keyword = lines[0]?.find((token) => token.content.trim() !== "");
    if (!keyword) throw new Error("expected a non-whitespace token");
    return keyword.style;
  }

  afterEach(() => {
    clearThemeMetas();
    vi.restoreAllMocks();
  });

  it("keeps both theme colors on a token so the mode toggle needs no re-highlight", async () => {
    const style = await keywordStyle({});
    expect(style).toHaveProperty("color");
    expect(style).toHaveProperty("--shiki-dark");
  });

  it("honors a configured theme id", async () => {
    const configured = await keywordStyle({ light: "one-light", dark: "dracula" });
    const defaults = await keywordStyle({});

    expect(configured.color).not.toBe(defaults.color);
    expect(configured["--shiki-dark"]).not.toBe(defaults["--shiki-dark"]);
  });

  it("falls back to the default theme and warns for an id it cannot load", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);

    const unknown = await keywordStyle({ dark: "githbu-dark" });

    expect(warn).toHaveBeenCalledWith(expect.stringContaining("githbu-dark"));
    // Same singleton state as an unconfigured deployment.
    warn.mockClear();
    const defaults = await keywordStyle({});
    expect(unknown["--shiki-dark"]).toBe(defaults["--shiki-dark"]);
  });
});
