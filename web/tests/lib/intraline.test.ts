import { describe, expect, it } from "vitest";

import { intralineSegments } from "@/lib/diff/intraline";

describe("intralineSegments", () => {
  it("returns a single unchanged segment for identical strings", () => {
    const [oldSegments, newSegments] = intralineSegments("same text", "same text");
    expect(oldSegments).toEqual([{ text: "same text", changed: false }]);
    expect(newSegments).toEqual([{ text: "same text", changed: false }]);
  });

  it("trims a common prefix and suffix around a pure insertion (tier 1 only)", () => {
    const [oldSegments, newSegments] = intralineSegments("foobar", "fooXbar");
    expect(oldSegments).toEqual([
      { text: "foo", changed: false },
      { text: "bar", changed: false },
    ]);
    expect(newSegments).toEqual([
      { text: "foo", changed: false },
      { text: "X", changed: true },
      { text: "bar", changed: false },
    ]);
  });

  it("keeps a common word unchanged inside a word-level diff of the middle", () => {
    const [oldSegments, newSegments] = intralineSegments("foo bar baz", "qux bar quux");
    const oldUnchanged = oldSegments
      .filter((segment) => !segment.changed)
      .map((segment) => segment.text)
      .join("");
    const newUnchanged = newSegments
      .filter((segment) => !segment.changed)
      .map((segment) => segment.text)
      .join("");
    expect(oldUnchanged).toContain("bar");
    expect(newUnchanged).toContain("bar");
    expect(oldSegments.some((segment) => segment.changed && segment.text === "foo")).toBe(true);
    expect(newSegments.some((segment) => segment.changed && segment.text === "qux")).toBe(true);
    expect(oldSegments.some((segment) => segment.changed && segment.text === "baz")).toBe(true);
    expect(newSegments.some((segment) => segment.changed && segment.text === "quux")).toBe(true);
  });

  it("falls back to one whole-middle segment above the word-diff budget", () => {
    const oldMid = "a".repeat(501);
    const newMid = "b".repeat(501);
    const [oldSegments, newSegments] = intralineSegments(`PREFIX${oldMid}SUFFIX`, `PREFIX${newMid}SUFFIX`);
    expect(oldSegments).toEqual([
      { text: "PREFIX", changed: false },
      { text: oldMid, changed: true },
      { text: "SUFFIX", changed: false },
    ]);
    expect(newSegments).toEqual([
      { text: "PREFIX", changed: false },
      { text: newMid, changed: true },
      { text: "SUFFIX", changed: false },
    ]);
  });

  it("never splits a surrogate-pair character across a prefix/suffix boundary", () => {
    // U+1F4A9 is two UTF-16 code units; naive index-based trimming could
    // slice between them and produce two lone (invalid) surrogates.
    const [oldSegments, newSegments] = intralineSegments("abc\u{1F4A9}", "abc");
    expect(oldSegments).toEqual([
      { text: "abc", changed: false },
      { text: "\u{1F4A9}", changed: true },
    ]);
    expect(newSegments).toEqual([{ text: "abc", changed: false }]);
  });
});
