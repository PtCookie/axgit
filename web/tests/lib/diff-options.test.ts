import { describe, expect, it } from "vitest";

import { diffApiParams, diffOptionsQuery, parseDiffOptions } from "@/lib/diff-options";

describe("parseDiffOptions", () => {
  it("defaults to unified, context 3, ignorews off when the query is empty", () => {
    expect(parseDiffOptions("")).toEqual({ view: "unified", context: 3, ignorews: false });
  });

  it("accepts every allowed context value", () => {
    for (const context of [1, 3, 5, 10, 20, 40]) {
      expect(parseDiffOptions(`?context=${context}`).context).toBe(context);
    }
  });

  it("falls back to the default context for an out-of-range or non-numeric value", () => {
    for (const bad of ["0", "2", "41", "100", "abc", ""]) {
      expect(parseDiffOptions(`?context=${bad}`).context).toBe(3);
    }
  });

  it("only recognizes view=split and view=stat, anything else stays unified", () => {
    expect(parseDiffOptions("?view=split").view).toBe("split");
    expect(parseDiffOptions("?view=stat").view).toBe("stat");
    expect(parseDiffOptions("?view=unified").view).toBe("unified");
    expect(parseDiffOptions("?view=bogus").view).toBe("unified");
  });

  it("only recognizes ignorews=1", () => {
    expect(parseDiffOptions("?ignorews=1").ignorews).toBe(true);
    expect(parseDiffOptions("?ignorews=true").ignorews).toBe(false);
    expect(parseDiffOptions("?ignorews=0").ignorews).toBe(false);
  });
});

describe("diffOptionsQuery", () => {
  it("omits every field at its default", () => {
    expect(diffOptionsQuery({ view: "unified", context: 3, ignorews: false })).toEqual({});
  });

  it("includes only the fields that differ from the default", () => {
    expect(diffOptionsQuery({ view: "split", context: 10, ignorews: true })).toEqual({
      view: "split",
      context: "10",
      ignorews: "1",
    });
  });

  it("includes view=stat", () => {
    expect(diffOptionsQuery({ view: "stat", context: 3, ignorews: false })).toEqual({ view: "stat" });
  });

  it("round-trips through parseDiffOptions", () => {
    const options = { view: "split" as const, context: 20, ignorews: true };
    const query = new URLSearchParams(diffOptionsQuery(options)).toString();
    expect(parseDiffOptions(`?${query}`)).toEqual(options);
  });
});

describe("diffApiParams", () => {
  it("omits both fields at their default", () => {
    expect(diffApiParams({ view: "unified", context: 3, ignorews: false })).toEqual({});
  });

  it("never includes view — it's a display-only choice", () => {
    const params = diffApiParams({ view: "split", context: 10, ignorews: true });
    expect(params).not.toHaveProperty("view");
    expect(params).toEqual({ context: 10, ignorews: 1 });
  });
});
