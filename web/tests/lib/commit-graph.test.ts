import { describe, expect, it } from "vitest";

import type { CommitInfo } from "@/lib/api/schemas";
import { layoutCommitGraph, MAX_LANES } from "@/lib/commit-graph";

/** Builds a minimal `CommitInfo` for layout tests — only `sha`/`parents`
 *  matter to `layoutCommitGraph`. */
function commit(sha: string, parents: string[]): CommitInfo {
  return {
    sha,
    parents,
    summary: sha,
    author: { name: "Test", email_hash: "deadbeef" },
    authored_at: null,
  };
}

describe("layoutCommitGraph", () => {
  it("returns an empty graph for no commits", () => {
    expect(layoutCommitGraph([])).toEqual({ rows: [], lanes: 1, overflow: false });
  });

  it("keeps a linear history in a single straight lane", () => {
    const commits = [commit("c", ["b"]), commit("b", ["a"]), commit("a", [])];
    const graph = layoutCommitGraph(commits);

    expect(graph.lanes).toBe(1);
    expect(graph.overflow).toBe(false);
    expect(graph.rows.map((r) => r.lane)).toEqual([0, 0, 0]);
    expect(graph.rows.every((r) => r.through.length === 0)).toBe(true);
    expect(graph.rows[0].in).toEqual([]); // no continuesAbove -> top row is a bare start
    expect(graph.rows[0].out).toEqual([0]);
    expect(graph.rows[1].in).toEqual([0]);
    expect(graph.rows[2].out).toEqual([]); // root: no parents
  });

  it("draws an incoming edge to the top row when continuing from a previous page", () => {
    const commits = [commit("c", ["b"]), commit("b", [])];
    const graph = layoutCommitGraph(commits, { continuesAbove: true });

    expect(graph.rows[0].in).toEqual([0]);
  });

  it("lays out a merge: two parents fan out, then converge at the shared ancestor", () => {
    // A merges B and C, both of which descend from D.
    const commits = [commit("A", ["B", "C"]), commit("B", ["D"]), commit("C", ["D"]), commit("D", [])];
    const graph = layoutCommitGraph(commits);
    const [a, b, c, d] = graph.rows;

    expect(a.merge).toBe(true);
    expect(a.out).toEqual([0, 1]);
    expect(b.lane).toBe(0);
    expect(c.lane).toBe(1);
    // Each of B/C carries the other's lane through as a passthrough.
    expect(b.through).toEqual([1]);
    expect(c.through).toEqual([0]);
    expect(d.in).toEqual([0, 1]); // branch point: both lanes converge
    expect(graph.lanes).toBe(2);
  });

  it("reuses a lane already awaiting a merge's second parent instead of allocating a new one", () => {
    // A merges (B, D). D is also awaited by another lane (D is E's first
    // parent's sibling path via B->E and D->E), so A's second parent should
    // attach to that existing lane rather than opening a third one.
    const commits = [commit("A", ["B", "D"]), commit("B", ["E"]), commit("D", ["E"]), commit("E", [])];
    const graph = layoutCommitGraph(commits);

    expect(graph.lanes).toBe(2);
    expect(graph.overflow).toBe(false);
  });

  it("keeps a lane alive across rows when its parent is off the page", () => {
    const commits = [commit("b", ["a"]), commit("a", ["off-page"])];
    const graph = layoutCommitGraph(commits);

    expect(graph.rows[1].out).toEqual([0]);
    expect(graph.lanes).toBe(1);
  });

  it("carries an off-page parent lane through every later row as a passthrough", () => {
    // "a" and "z" are unrelated tips; "a"'s parent never appears on the page,
    // so its lane must show up in "through" for every row below it.
    const commits = [commit("a", ["missing"]), commit("z", ["y"]), commit("y", [])];
    const graph = layoutCommitGraph(commits);

    expect(graph.rows[1].through).toContain(0);
    expect(graph.rows[2].through).toContain(0);
  });

  it("reuses a freed interior lane instead of appending a new one", () => {
    // B opens lane 0 (pending parent C); Q opens lane 1 (pending parent R),
    // since lane 0 is still occupied. C then resolves lane 0's pending
    // parent as a root, freeing lane 0 while lane 1 (R, still pending)
    // stays occupied — an interior hole, not a trailing one, so it is not
    // trimmed away. Y, a fresh unrelated root, should reuse lane 0 rather
    // than opening a third lane.
    const commits = [commit("B", ["C"]), commit("Q", ["R"]), commit("C", []), commit("Y", [])];
    const graph = layoutCommitGraph(commits);
    const [b, q, c, y] = graph.rows;

    expect(b.lane).toBe(0);
    expect(q.lane).toBe(1);
    expect(q.through).toEqual([0]); // B's pending parent C still occupies lane 0
    expect(c.lane).toBe(0); // reuses lane 0 via `incoming`
    expect(c.through).toEqual([1]); // Q's pending parent R still occupies lane 1
    expect(y.lane).toBe(0); // reuses the interior hole left by C, not a new lane 2
    expect(graph.lanes).toBe(2);
  });

  it("caps concurrent lanes at MAX_LANES and reports overflow", () => {
    const parents = Array.from({ length: MAX_LANES + 8 }, (_, i) => `p${i}`);
    const commits = [commit("root", parents), ...parents.map((p) => commit(p, []))];
    const graph = layoutCommitGraph(commits);

    expect(graph.lanes).toBe(MAX_LANES);
    expect(graph.overflow).toBe(true);
    for (const row of graph.rows) {
      expect(row.lane).toBeLessThan(MAX_LANES);
      for (const lane of [...row.in, ...row.out, ...row.through]) {
        expect(lane).toBeLessThan(MAX_LANES);
      }
    }
  });

  it("de-duplicates a commit that lists the same parent twice", () => {
    const commits = [commit("a", ["b", "b"]), commit("b", [])];
    const graph = layoutCommitGraph(commits);

    expect(graph.rows[0].merge).toBe(true);
    expect(graph.rows[0].out).toEqual([0]);
  });

  it("keeps every lane index within bounds and in/out/through mostly disjoint across a larger DAG", () => {
    const commits = [
      commit("m1", ["m2", "side1"]),
      commit("side1", ["base1"]),
      commit("m2", ["m3", "side2"]),
      commit("side2", ["base2"]),
      commit("m3", ["base1"]),
      commit("base2", ["root"]),
      commit("base1", ["root"]),
      commit("root", []),
    ];
    const graph = layoutCommitGraph(commits);

    expect(graph.rows).toHaveLength(commits.length);
    for (const row of graph.rows) {
      for (const lane of [...row.in, ...row.out, ...row.through, row.lane]) {
        expect(lane).toBeGreaterThanOrEqual(0);
        expect(lane).toBeLessThan(graph.lanes);
      }
      // `through` never includes the node's own lane.
      expect(row.through).not.toContain(row.lane);
    }
  });
});
