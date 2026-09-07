import { describe, expect, it } from "vitest";

import type { RepoInfo } from "@/lib/api/schemas";
import { groupBySection } from "@/lib/repo-group";

function repo(name: string, section: string | null, lastModified: string | null = null): RepoInfo {
  return {
    name,
    section,
    owner: null,
    description: null,
    default_branch: null,
    last_modified: lastModified,
    homepage: null,
  };
}

/** Section labels in group order — `null` for the unsectioned group. */
function sectionOrder(repos: RepoInfo[]): (string | null)[] {
  return groupBySection(repos).map((group) => group.section);
}

describe("groupBySection", () => {
  it("puts the unsectioned group first, whatever its own activity", () => {
    const repos = [
      repo("infra-a", "infra", "2026-07-30T09:00:00+09:00"),
      repo("loose", null, "2020-01-01T00:00:00+09:00"),
    ];
    expect(sectionOrder(repos)).toEqual([null, "infra"]);
  });

  it("orders named groups by their most recently active repository, newest first", () => {
    const repos = [
      repo("a", "alpha", "2026-01-01T00:00:00+09:00"),
      repo("b", "beta", "2025-01-01T00:00:00+09:00"),
      // charlie's newest repo beats both, even though its other repo is the oldest of all.
      repo("c1", "charlie", "2024-01-01T00:00:00+09:00"),
      repo("c2", "charlie", "2026-06-01T00:00:00+09:00"),
    ];
    expect(sectionOrder(repos)).toEqual(["charlie", "alpha", "beta"]);
  });

  it("compares instants, not the formatted string, across differing UTC offsets", () => {
    // "+09:00" sorts after "-05:00" lexicographically, but names the earlier instant.
    const repos = [
      repo("early", "early", "2026-07-24T13:06:00+09:00"),
      repo("late", "late", "2026-07-24T05:00:00-05:00"),
    ];
    expect(sectionOrder(repos)).toEqual(["late", "early"]);
  });

  it("sorts a group whose repositories all lack a last activity last", () => {
    const repos = [repo("quiet", "aaa-quiet", null), repo("loud", "zzz-loud", "2020-01-01T00:00:00+09:00")];
    expect(sectionOrder(repos)).toEqual(["zzz-loud", "aaa-quiet"]);
  });

  it("breaks ties by section name ascending", () => {
    const same = "2026-07-30T09:00:00+09:00";
    const repos = [
      repo("b", "beta", same),
      repo("a", "alpha", same),
      repo("y", "yankee", null),
      repo("x", "xray", null),
    ];
    expect(sectionOrder(repos)).toEqual(["alpha", "beta", "xray", "yankee"]);
  });

  it("preserves the incoming repository order within each group", () => {
    const repos = [
      repo("second", "infra", "2026-01-01T00:00:00+09:00"),
      repo("first", "infra", "2026-06-01T00:00:00+09:00"),
    ];
    expect(groupBySection(repos)[0].repos.map((r) => r.name)).toEqual(["second", "first"]);
  });
});
