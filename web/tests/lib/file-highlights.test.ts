import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/format/highlight", () => ({
  highlightCode: vi.fn(),
}));

import type { FileDiff, Hunk, Line } from "@/lib/api/schemas";
import { highlightFileDiff } from "@/lib/diff/file-highlights";
import { highlightCode } from "@/lib/format/highlight";

const mockedHighlightCode = vi.mocked(highlightCode);

beforeEach(() => {
  mockedHighlightCode.mockReset();
});

function line(origin: Line["origin"], content: string, oldLineno: number | null, newLineno: number | null): Line {
  return { origin, content, old_lineno: oldLineno, new_lineno: newLineno };
}

function fileWith(hunks: Hunk[], overrides: Partial<FileDiff> = {}): FileDiff {
  return {
    path: "a.rs",
    old_path: null,
    status: "modified",
    additions: 0,
    deletions: 0,
    binary: false,
    truncated: false,
    hunks,
    ...overrides,
  };
}

describe("highlightFileDiff", () => {
  it("returns null for a binary file without calling highlightCode", async () => {
    const file = fileWith([], { binary: true });
    const result = await highlightFileDiff(file);
    expect(result).toBeNull();
    expect(mockedHighlightCode).not.toHaveBeenCalled();
  });

  it("reconstructs each side's shown lines and maps tokens back by Line identity", async () => {
    const contextLine = line(" ", "one", 1, 1);
    const deletedLine = line("-", "two", 2, null);
    const addedLine = line("+", "three", null, 2);
    const hunk: Hunk = {
      header: "@@ -1,2 +1,2 @@",
      old_start: 1,
      old_lines: 2,
      new_start: 1,
      new_lines: 2,
      lines: [contextLine, deletedLine, addedLine],
    };
    const file = fileWith([hunk]);

    const oldTokens = [[{ content: "one", style: {} }], [{ content: "two", style: {} }]];
    const newTokens = [[{ content: "one", style: {} }], [{ content: "three", style: {} }]];
    mockedHighlightCode.mockImplementation((text) => {
      if (text === "one\ntwo") return Promise.resolve(oldTokens);
      if (text === "one\nthree") return Promise.resolve(newTokens);
      return Promise.resolve(null);
    });

    const result = await highlightFileDiff(file);

    expect(mockedHighlightCode).toHaveBeenCalledWith("one\ntwo", "a.rs");
    expect(mockedHighlightCode).toHaveBeenCalledWith("one\nthree", "a.rs");
    // The context line appears on both sides; whichever resolves last wins,
    // but the content (and therefore the tokens) are equivalent either way.
    expect(result?.get(contextLine)).toEqual(newTokens[0]);
    expect(result?.get(deletedLine)).toEqual(oldTokens[1]);
    expect(result?.get(addedLine)).toEqual(newTokens[1]);
  });

  it("highlights the old side against old_path, for a renamed file", async () => {
    const deletedLine = line("-", "one", 1, null);
    const hunk: Hunk = {
      header: "@@ -1 +0,0 @@",
      old_start: 1,
      old_lines: 1,
      new_start: 0,
      new_lines: 0,
      lines: [deletedLine],
    };
    const file = fileWith([hunk], { path: "new-name.rs", old_path: "old-name.rs", status: "renamed" });
    mockedHighlightCode.mockResolvedValue(null);

    await highlightFileDiff(file);

    expect(mockedHighlightCode).toHaveBeenCalledWith("one", "old-name.rs");
  });

  it("skips calling highlightCode for a side with no lines (a wholly added file)", async () => {
    const addedLine = line("+", "one", null, 1);
    const hunk: Hunk = {
      header: "@@ -0,0 +1 @@",
      old_start: 0,
      old_lines: 0,
      new_start: 1,
      new_lines: 1,
      lines: [addedLine],
    };
    const file = fileWith([hunk], { status: "added" });
    mockedHighlightCode.mockResolvedValue([[{ content: "one", style: {} }]]);

    await highlightFileDiff(file);

    expect(mockedHighlightCode).toHaveBeenCalledTimes(1);
    expect(mockedHighlightCode).toHaveBeenCalledWith("one", "a.rs");
  });

  it("returns null when neither side's language is supported", async () => {
    const hunk: Hunk = {
      header: "@@ -1 +1 @@",
      old_start: 1,
      old_lines: 1,
      new_start: 1,
      new_lines: 1,
      lines: [line("-", "one", 1, null), line("+", "uno", null, 1)],
    };
    const file = fileWith([hunk], { path: "a.txt" });
    mockedHighlightCode.mockResolvedValue(null);

    const result = await highlightFileDiff(file);
    expect(result).toBeNull();
  });
});
