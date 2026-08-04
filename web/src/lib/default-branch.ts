import { useEffect, useState } from "react";

import { getRepo } from "@/lib/api/repos";

/**
 * Fetches `/repos/{repo}` purely to read `default_branch` — decoration, not
 * required data, following `useCommitRefs`'s rule (docs/DECISIONS.md #34):
 * a failure never touches the caller's own loading/error state, it just
 * leaves the returned value `null`. Same fetch `RefsView`'s Compare column
 * (#40) and `DiffView`'s `to` prefill both need, so it's a shared hook
 * rather than two copies of the same effect.
 *
 * Returns `null` while loading, on failure, for an empty repository (where
 * `default_branch` itself is `null`), and whenever `enabled` is `false`
 * (skips the request entirely — used by `DiffView` to avoid it once a
 * comparison is already in progress).
 */
export function useDefaultBranch(repo: string, enabled = true): string | null {
  const [defaultBranch, setDefaultBranch] = useState<string | null>(null);

  useEffect(() => {
    if (!enabled) {
      return;
    }
    let cancelled = false;

    getRepo(repo)
      .then((summary) => {
        if (!cancelled) {
          setDefaultBranch(summary.default_branch);
        }
      })
      .catch(() => {
        /* decoration only */
      });

    return () => {
      cancelled = true;
    };
  }, [repo, enabled]);

  return defaultBranch;
}
