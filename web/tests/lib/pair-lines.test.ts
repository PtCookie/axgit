import { describe, expect, it } from "vitest";

import { pairHunkLines } from "@/lib/diff/pair-lines";
import type { Line } from "@/lib/api/schemas";

function context(content: string, oldLineno: number, newLineno: number): Line {
  return { origin: " ", content, old_lineno: oldLineno, new_lineno: newLineno };
}

function del(content: string, oldLineno: number): Line {
  return { origin: "-", content, old_lineno: oldLineno, new_lineno: null };
}

function add(content: string, newLineno: number): Line {
  return { origin: "+", content, old_lineno: null, new_lineno: newLineno };
}

describe("pairHunkLines", () => {
  it("pairs a pure context run one-for-one", () => {
    const lines = [context("one", 1, 1), context("two", 2, 2)];
    expect(pairHunkLines(lines)).toEqual([
      { kind: "context", old: lines[0], new: lines[0] },
      { kind: "context", old: lines[1], new: lines[1] },
    ]);
  });

  it("pairs a balanced deletion/addition block index-for-index", () => {
    const d = del("old", 1);
    const a = add("new", 1);
    expect(pairHunkLines([d, a])).toEqual([{ kind: "change", old: d, new: a }]);
  });

  it("pairs 3 deletions against 1 addition, leaving two rows with new: null", () => {
    const dels = [del("a", 1), del("b", 2), del("c", 3)];
    const adds = [add("x", 1)];
    expect(pairHunkLines([...dels, ...adds])).toEqual([
      { kind: "change", old: dels[0], new: adds[0] },
      { kind: "change", old: dels[1], new: null },
      { kind: "change", old: dels[2], new: null },
    ]);
  });

  it("pairs 1 deletion against 3 additions, leaving two rows with old: null", () => {
    const dels = [del("a", 1)];
    const adds = [add("x", 1), add("y", 2), add("z", 3)];
    expect(pairHunkLines([...dels, ...adds])).toEqual([
      { kind: "change", old: dels[0], new: adds[0] },
      { kind: "change", old: null, new: adds[1] },
      { kind: "change", old: null, new: adds[2] },
    ]);
  });

  it("handles a leading addition run with no preceding deletions", () => {
    const a = add("new", 1);
    expect(pairHunkLines([a])).toEqual([{ kind: "change", old: null, new: a }]);
  });

  it("starts a fresh change block when a second run appears after a context line", () => {
    const d1 = del("a", 2);
    const a1 = add("x", 2);
    const c = context("mid", 3, 3);
    const d2 = del("b", 4);
    const a2 = add("y", 4);
    expect(pairHunkLines([d1, a1, c, d2, a2])).toEqual([
      { kind: "change", old: d1, new: a1 },
      { kind: "context", old: c, new: c },
      { kind: "change", old: d2, new: a2 },
    ]);
  });

  it("starts a fresh change block when a deletion follows an addition with no context between", () => {
    // Not something libgit2 emits in practice, but the pairing must not
    // silently merge two unrelated runs into one.
    const a1 = add("x", 1);
    const d1 = del("a", 1);
    expect(pairHunkLines([a1, d1])).toEqual([
      { kind: "change", old: null, new: a1 },
      { kind: "change", old: d1, new: null },
    ]);
  });
});
