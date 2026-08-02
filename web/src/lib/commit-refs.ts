import { useEffect, useState } from "react";

import { getRefs } from "@/lib/api/repos";
import type { RefsInfo } from "@/lib/api/schemas";

export interface CommitRef {
  name: string;
  kind: "branch" | "tag";
}

/** Groups a `RefsInfo` response by the commit sha it points at. Branches are
 *  indexed before tags (a commit that's both a branch tip and tagged shows
 *  the branch badge first), each already name-sorted by the api. */
export function indexRefsBySha(refs: RefsInfo): Map<string, CommitRef[]> {
  const bySha = new Map<string, CommitRef[]>();
  const push = (sha: string, ref: CommitRef) => {
    const existing = bySha.get(sha);
    if (existing) {
      existing.push(ref);
    } else {
      bySha.set(sha, [ref]);
    }
  };
  for (const branch of refs.branches) {
    push(branch.target, { name: branch.name, kind: "branch" });
  }
  for (const tag of refs.tags) {
    push(tag.target, { name: tag.name, kind: "tag" });
  }
  return bySha;
}

const EMPTY = new Map<string, CommitRef[]>();

/**
 * Fetches `/refs` and indexes it by target sha, for decorating commit rows
 * with branch/tag badges. Deliberately **not** folded into the page's own
 * commits/detail fetch — this is decoration, not required data, so it must
 * never block or error the page it's used on (docs/DECISIONS.md #34,
 * following #21's "a 404 renders nothing" precedent for `ReadmeView`).
 * Returns an empty map while loading and forever after a failure.
 */
export function useCommitRefs(repo: string): Map<string, CommitRef[]> {
  const [bySha, setBySha] = useState(EMPTY);

  useEffect(() => {
    let cancelled = false;
    setBySha(EMPTY);

    getRefs(repo)
      .then((refs) => {
        if (!cancelled) {
          setBySha(indexRefsBySha(refs));
        }
      })
      .catch(() => {
        // Silent by design — badges are decoration, not required data.
      });

    return () => {
      cancelled = true;
    };
  }, [repo]);

  return bySha;
}
