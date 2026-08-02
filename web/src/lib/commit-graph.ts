import type { CommitInfo } from "@/lib/api/schemas";

/**
 * Client-side lane layout for the Log tab's graph column (docs/DECISIONS.md
 * #33). `GET /commits` already returns `parents` on every `CommitInfo` and no
 * frontend code read it before this — so this needs no API change.
 *
 * The walk backing this data (`api/src/repo/commits.rs::log`) sets no sort
 * flags, i.e. plain committer-date order, deliberately: any libgit2 sort flag
 * forces a full-history walk before the first commit is emitted (verified
 * against libgit2's `revwalk.c`), which would turn every cursor page into an
 * O(repo size) request. Date order is enough for a correct graph as long as
 * a parent's time never exceeds its child's, which holds for any history
 * without clock skew — within a single page a parent can in fact never be
 * *emitted* above its child at all, because libgit2's lazy walk only
 * discovers a commit by popping one of its already-emitted children. That
 * invariant is what makes this a single forward pass with no lookahead.
 *
 * Lanes never shift horizontally: a freed lane is left as a hole and reused
 * by the next allocation, so a `through` lane is always a straight vertical
 * and the only diagonals are a node's own half-edges. That trades a
 * (usually) one-or-two-lane-wider column for not needing any lane-identity
 * tracking across rows.
 */

/** Hard cap on concurrent lanes. A commit that would need a new lane beyond
 *  this reuses (clobbers) the last one instead — `overflow` reports this. */
export const MAX_LANES = 12;

/** One table row's worth of graph geometry, in lane indices (not pixels). */
export interface GraphRow {
  sha: string;
  /** Lane the commit's node sits in. */
  lane: number;
  /** `parents.length > 1`. */
  merge: boolean;
  /** Lanes crossing the top edge that converge into this node (includes
   *  `lane` itself when the node's own lane came from above). */
  in: number[];
  /** Lanes crossing the bottom edge that diverge from this node, one per
   *  distinct parent (includes `lane` when the first parent continues
   *  straight down). */
  out: number[];
  /** Lanes crossing the whole row untouched by this commit. */
  through: number[];
}

export interface CommitGraph {
  rows: GraphRow[];
  /** Page-wide lane count; every row's renderer uses this width so lanes
   *  line up column-to-column. Always at least 1. */
  lanes: number;
  /** True when `MAX_LANES` forced an edge to be dropped somewhere. */
  overflow: boolean;
}

export interface LayoutOptions {
  /** Set when this page was reached via `cursor` (i.e. it isn't the first
   *  page): row 0's children are on the previous page, so if row 0 would
   *  otherwise show no incoming edge, draw one to the top edge instead of a
   *  fake root dot. */
  continuesAbove?: boolean;
}

/** Lays out `commits` (in the order the API returns them: committer-date
 *  descending) into graph lanes. Pure function of its arguments. */
export function layoutCommitGraph(commits: readonly CommitInfo[], options: LayoutOptions = {}): CommitGraph {
  const rows: GraphRow[] = [];
  /** `lanes[j]` is the sha lane `j` is waiting for, or `null` if free. */
  const lanes: (string | null)[] = [];
  let width = 0;
  let overflow = false;

  const alloc = (sha: string): number => {
    const hole = lanes.indexOf(null);
    if (hole !== -1) {
      lanes[hole] = sha;
      return hole;
    }
    if (lanes.length < MAX_LANES) {
      lanes.push(sha);
      return lanes.length - 1;
    }
    // Cap reached: reuse the last lane. Its previous edge simply ends here.
    overflow = true;
    lanes[MAX_LANES - 1] = sha;
    return MAX_LANES - 1;
  };

  commits.forEach((commit, index) => {
    // 1. Lanes already waiting for this commit (i.e. this commit is their
    //    parent) converge here; the node takes the leftmost.
    const incoming: number[] = [];
    for (let j = 0; j < lanes.length; j += 1) {
      if (lanes[j] === commit.sha) incoming.push(j);
    }
    const lane = incoming.length > 0 ? incoming[0] : alloc(commit.sha);

    // 2. Every other still-active lane crosses this row untouched. Computed
    //    before parents are allocated, so a lane freshly created for a
    //    parent starts at this row's midpoint, not its top edge.
    const through: number[] = [];
    for (let j = 0; j < lanes.length; j += 1) {
      if (j !== lane && lanes[j] !== null && lanes[j] !== commit.sha) through.push(j);
    }

    // 3. Consume every lane that was waiting for this commit.
    for (const j of incoming) lanes[j] = null;
    lanes[lane] = null;

    // 4. Parents leave the node. The first parent always keeps the node's
    //    own lane, so the mainline stays straight; extra parents (merges)
    //    reuse a lane already awaiting that sha if one exists, else
    //    allocate leftmost-free.
    const out: number[] = [];
    commit.parents.forEach((parent, order) => {
      let target: number;
      if (order === 0) {
        lanes[lane] = parent;
        target = lane;
      } else {
        const existing = lanes.indexOf(parent);
        target = existing !== -1 ? existing : alloc(parent);
      }
      if (!out.includes(target)) out.push(target);
    });

    // 5. Row 0 of a cursor page has its children on the previous page: draw
    //    a half line to the top edge instead of a bare "root" dot.
    const inbound = incoming.length > 0 ? incoming : index === 0 && options.continuesAbove ? [lane] : [];

    while (lanes.length > 0 && lanes[lanes.length - 1] === null) lanes.pop();

    width = Math.max(
      width,
      lane + 1,
      ...inbound.map((j) => j + 1),
      ...out.map((j) => j + 1),
      ...through.map((j) => j + 1),
    );

    rows.push({
      sha: commit.sha,
      lane,
      merge: commit.parents.length > 1,
      in: [...inbound].sort((a, b) => a - b),
      out: [...out].sort((a, b) => a - b),
      through,
    });
  });

  return { rows, lanes: Math.min(Math.max(width, 1), MAX_LANES), overflow };
}
