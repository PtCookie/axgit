import { describe, expect, it } from "vitest";

import type { RefsInfo } from "@/lib/api/schemas";
import { indexRefsBySha } from "@/lib/commit-refs";

function refs(overrides: Partial<RefsInfo> = {}): RefsInfo {
  return { branches: [], tags: [], ...overrides };
}

describe("indexRefsBySha", () => {
  it("returns an empty map for an empty response", () => {
    expect(indexRefsBySha(refs())).toEqual(new Map());
  });

  it("indexes a branch by its target sha", () => {
    const bySha = indexRefsBySha(refs({ branches: [{ name: "main", target: "abc123", committed_at: null }] }));

    expect(bySha.get("abc123")).toEqual([{ name: "main", kind: "branch" }]);
  });

  it("indexes a tag by its target sha", () => {
    const bySha = indexRefsBySha(
      refs({ tags: [{ name: "v1.0.0", target: "def456", annotation: null, tagged_at: null }] }),
    );

    expect(bySha.get("def456")).toEqual([{ name: "v1.0.0", kind: "tag" }]);
  });

  it("orders branches before tags on a commit that is both", () => {
    const bySha = indexRefsBySha(
      refs({
        branches: [{ name: "main", target: "abc123", committed_at: null }],
        tags: [{ name: "v1.0.0", target: "abc123", annotation: null, tagged_at: null }],
      }),
    );

    expect(bySha.get("abc123")).toEqual([
      { name: "main", kind: "branch" },
      { name: "v1.0.0", kind: "tag" },
    ]);
  });

  it("collects multiple branches pointing at the same commit", () => {
    const bySha = indexRefsBySha(
      refs({
        branches: [
          { name: "main", target: "abc123", committed_at: null },
          { name: "release", target: "abc123", committed_at: null },
        ],
      }),
    );

    expect(bySha.get("abc123")).toEqual([
      { name: "main", kind: "branch" },
      { name: "release", kind: "branch" },
    ]);
  });

  it("has no entry for a sha nothing points at", () => {
    const bySha = indexRefsBySha(refs({ branches: [{ name: "main", target: "abc123", committed_at: null }] }));

    expect(bySha.has("someothersha")).toBe(false);
  });
});
