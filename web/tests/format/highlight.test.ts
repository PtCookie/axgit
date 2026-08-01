import { describe, expect, it } from "vitest";

import { highlightCode, languageForPath } from "@/lib/format/highlight";

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
