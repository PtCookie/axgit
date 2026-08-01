import { useEffect, useState } from "react";

import { ApiError } from "@/lib/api/client";
import { listRepos } from "@/lib/api/repos";
import type { RepoInfo } from "@/lib/api/schemas";
import { formatAbsoluteTime, formatRelativeTime } from "@/lib/format/time";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

const UNSECTIONED_LABEL = "Other";

interface RepoGroup {
  section: string | null;
  repos: RepoInfo[];
}

function groupBySection(repos: RepoInfo[]): RepoGroup[] {
  const groups: RepoGroup[] = [];

  for (const repo of repos) {
    const existing = groups.find((group) => group.section === repo.section);
    if (existing) {
      existing.repos.push(repo);
    } else {
      groups.push({ section: repo.section, repos: [repo] });
    }
  }

  // section: null ("Other") always sorts last. Other groups keep their first-appearance order
  // (= the repos list's sort order).
  groups.sort((a, b) => (a.section === null ? 1 : b.section === null ? -1 : 0));
  return groups;
}

type State = { status: "loading" } | { status: "error"; error: ApiError } | { status: "data"; repos: RepoInfo[] };

/** Also rendered statically into the page shell as the island's
 *  `slot="fallback"`, so the prerendered HTML is not blank. */
export function RepoListSkeleton() {
  return (
    <div className="space-y-2" aria-busy="true">
      <Skeleton className="h-8 w-full" />
      <Skeleton className="h-8 w-full" />
      <Skeleton className="h-8 w-full" />
    </div>
  );
}

export default function RepoList() {
  const [state, setState] = useState<State>({ status: "loading" });

  useEffect(() => {
    let cancelled = false;

    listRepos()
      .then((response) => {
        if (!cancelled) {
          setState({ status: "data", repos: response.repos });
        }
      })
      .catch((error: unknown) => {
        if (cancelled) {
          return;
        }
        setState({
          status: "error",
          error: error instanceof ApiError ? error : new ApiError("internal", "unknown error", 0),
        });
      });

    return () => {
      cancelled = true;
    };
  }, []);

  if (state.status === "loading") {
    return <RepoListSkeleton />;
  }

  if (state.status === "error") {
    return (
      <p role="alert" className="text-destructive text-sm">
        Failed to load repositories: {state.error.message}
      </p>
    );
  }

  if (state.repos.length === 0) {
    return <p className="text-muted-foreground text-sm">No repositories found.</p>;
  }

  return (
    <div className="space-y-8">
      {groupBySection(state.repos).map(({ section, repos }) => (
        <section key={section ?? UNSECTIONED_LABEL}>
          <h2 className="text-muted-foreground mb-2 text-sm font-medium">{section ?? UNSECTIONED_LABEL}</h2>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Description</TableHead>
                <TableHead>Owner</TableHead>
                <TableHead>Last activity</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {repos.map((repo) => (
                <TableRow key={repo.name}>
                  <TableCell className="font-medium">{repo.name}</TableCell>
                  <TableCell className="text-muted-foreground">{repo.description ?? "—"}</TableCell>
                  <TableCell className="text-muted-foreground">{repo.owner ?? "—"}</TableCell>
                  <TableCell className="text-muted-foreground">
                    {repo.last_modified ? (
                      <span title={formatAbsoluteTime(repo.last_modified)}>
                        {formatRelativeTime(repo.last_modified)}
                      </span>
                    ) : (
                      "—"
                    )}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </section>
      ))}
    </div>
  );
}
