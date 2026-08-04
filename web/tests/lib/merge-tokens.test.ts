import { describe, expect, it } from "vitest";

import { mergeTokensWithSegments } from "@/lib/diff/merge-tokens";
import type { HighlightedLine } from "@/lib/format/highlight";

describe("mergeTokensWithSegments", () => {
  it("passes through Shiki styling unchanged when nothing in the line changed", () => {
    const tokens: HighlightedLine = [
      { content: "let ", style: { color: "#ff0000" } },
      { content: "value", style: { color: "#00ff00" } },
    ];
    const segments = [{ text: "let value", changed: false }];
    expect(mergeTokensWithSegments(tokens, segments)).toEqual([
      { text: "let ", style: { color: "#ff0000" }, changed: false },
      { text: "value", style: { color: "#00ff00" }, changed: false },
    ]);
  });

  it("splits a token at a segment boundary that falls in its middle", () => {
    // One Shiki token spans the whole line; the intra-line diff only
    // changed the last word.
    const tokens: HighlightedLine = [{ content: "let value = old", style: { color: "#abc" } }];
    const segments = [
      { text: "let value = ", changed: false },
      { text: "old", changed: true },
    ];
    expect(mergeTokensWithSegments(tokens, segments)).toEqual([
      { text: "let value = ", style: { color: "#abc" }, changed: false },
      { text: "old", style: { color: "#abc" }, changed: true },
    ]);
  });

  it("splits a segment at a token boundary that falls in its middle", () => {
    // One intra-line segment (the whole line changed) spans two Shiki
    // tokens with different colors.
    const tokens: HighlightedLine = [
      { content: "old", style: { color: "#111" } },
      { content: "Name", style: { color: "#222" } },
    ];
    const segments = [{ text: "oldName", changed: true }];
    expect(mergeTokensWithSegments(tokens, segments)).toEqual([
      { text: "old", style: { color: "#111" }, changed: true },
      { text: "Name", style: { color: "#222" }, changed: true },
    ]);
  });

  it("handles independently-shaped boundaries on both sides at once", () => {
    const tokens: HighlightedLine = [
      { content: "ab", style: { color: "#1" } },
      { content: "cd", style: { color: "#2" } },
      { content: "ef", style: { color: "#3" } },
    ];
    const segments = [
      { text: "abc", changed: false },
      { text: "def", changed: true },
    ];
    expect(mergeTokensWithSegments(tokens, segments)).toEqual([
      { text: "ab", style: { color: "#1" }, changed: false },
      { text: "c", style: { color: "#2" }, changed: false },
      { text: "d", style: { color: "#2" }, changed: true },
      { text: "ef", style: { color: "#3" }, changed: true },
    ]);
  });
});
