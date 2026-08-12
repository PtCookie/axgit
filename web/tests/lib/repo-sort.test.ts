import { describe, expect, it } from "vitest";

import { orderToParam, parseOrder, sortRepos } from "@/lib/repo-sort";
import type { RepoInfo } from "@/lib/api/schemas";

function repo(name: string, desc: string | null = null, lastModified: string | null = null): RepoInfo {
  return {
    name,
    section: null,
    owner: null,
    description: desc,
    default_branch: null,
    last_modified: lastModified,
  };
}

describe("parseOrder", () => {
  it("defaults every key ascending except idle", () => {
    expect(parseOrder("name")).toEqual({ key: "name", reverse: false });
    expect(parseOrder("idle")).toEqual({ key: "idle", reverse: true });
  });

  it("flips the key's own default on a leading -", () => {
    expect(parseOrder("-name")).toEqual({ key: "name", reverse: true });
    expect(parseOrder("-idle")).toEqual({ key: "idle", reverse: false });
  });

  it("returns undefined for an unknown key", () => {
    expect(parseOrder("bogus")).toBeUndefined();
    expect(parseOrder("-")).toBeUndefined();
  });
});

describe("orderToParam", () => {
  it("round-trips every parseOrder result", () => {
    for (const raw of ["name", "-name", "idle", "-idle", "desc", "-desc"]) {
      const order = parseOrder(raw);
      if (order === undefined) {
        throw new Error(`parseOrder(${raw}) unexpectedly returned undefined`);
      }
      expect(orderToParam(order)).toBe(raw);
    }
  });
});

describe("sortRepos", () => {
  it("places a missing field last regardless of direction", () => {
    const repos = [repo("b", "beta"), repo("a", null), repo("c", "alpha")];

    expect(sortRepos(repos, { key: "desc", reverse: false }).map((r) => r.name)).toEqual(["c", "b", "a"]);
    expect(sortRepos(repos, { key: "desc", reverse: true }).map((r) => r.name)).toEqual(["b", "c", "a"]);
  });

  it("breaks ties by name ascending", () => {
    const repos = [repo("b"), repo("a")];
    expect(sortRepos(repos, { key: "name", reverse: false }).map((r) => r.name)).toEqual(["a", "b"]);
  });

  it("compares idle by parsed instant, not the formatted string", () => {
    // "+09:00" sorts after "-05:00" lexicographically, but the instant it
    // names is earlier.
    const repos = [repo("early", null, "2026-07-24T13:06:00+09:00"), repo("late", null, "2026-07-24T05:00:00-05:00")];
    expect(sortRepos(repos, { key: "idle", reverse: false }).map((r) => r.name)).toEqual(["early", "late"]);
  });

  it("does not mutate the input array", () => {
    const repos = [repo("b"), repo("a")];
    const sorted = sortRepos(repos, { key: "name", reverse: false });
    expect(repos.map((r) => r.name)).toEqual(["b", "a"]);
    expect(sorted).not.toBe(repos);
  });
});
