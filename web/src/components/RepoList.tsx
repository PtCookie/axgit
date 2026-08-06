import { ClockCounterClockwiseIcon } from "@phosphor-icons/react/dist/ssr/ClockCounterClockwise";
import { FolderOpenIcon } from "@phosphor-icons/react/dist/ssr/FolderOpen";
import { useEffect, useState } from "react";

import { ApiError } from "@/lib/api/client";
import { encodeSegment } from "@/lib/api/path";
import { listRepos } from "@/lib/api/repos";
import type { RepoInfo } from "@/lib/api/schemas";
import { formatAbsoluteTime, formatRelativeTime } from "@/lib/format/time";
import { filterRepos } from "@/lib/repo-filter";
import { paramFromSearch } from "@/lib/repo-param";
import { logHref, treeHref } from "@/lib/repo-href";
import IconLink from "@/components/IconLink";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

const UNSECTIONED_LABEL = "Other";
/** Query-string param the filter box reads its initial value from and keeps
 *  in sync with (`?q=`, docs/DECISIONS.md #25). */
const QUERY_PARAM = "q";

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
 *  `slot="fallback"`, so the prerendered HTML is not blank. The filter input
 *  is included but disabled — there's nothing to filter yet, and enabling it
 *  would let a keystroke land before hydration takes over. */
export function RepoListSkeleton() {
  return (
    <div className="space-y-4" aria-busy="true">
      <Input type="search" placeholder="Filter repositories…" aria-label="Filter repositories" disabled />
      <div className="space-y-2">
        <Skeleton className="h-8 w-full" />
        <Skeleton className="h-8 w-full" />
        <Skeleton className="h-8 w-full" />
      </div>
    </div>
  );
}

export default function RepoList() {
  const [state, setState] = useState<State>({ status: "loading" });
  // Seeded from `?q=` so a deep link (or a reload) lands already filtered —
  // `lib/repo-param.ts`'s usual pattern for reading page-URL state, reused
  // here instead of a dedicated `props` default since this is the one island
  // that isn't mounted on a `/{repo}/…` shell.
  const [query, setQuery] = useState(() => paramFromSearch(QUERY_PARAM, window.location.search) ?? "");

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

  // Keeps `?q=` mirrored to the typed query, so the current filter survives
  // a reload and can be shared/bookmarked. `replaceState`, not `pushState` —
  // a history entry per keystroke would break the back button; the
  // `<ClientRouter />` (DECISIONS.md #24) only reads `location` on a link
  // click, so it never observes this in between.
  useEffect(() => {
    const url = new URL(window.location.href);
    if (query.trim() === "") {
      url.searchParams.delete(QUERY_PARAM);
    } else {
      url.searchParams.set(QUERY_PARAM, query);
    }
    if (url.href !== window.location.href) {
      window.history.replaceState(window.history.state, "", url);
    }
  }, [query]);

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

  const filtered = filterRepos(state.repos, query);
  // `filterRepos` returns the same array reference when the query is
  // empty/whitespace-only — a cheap way to tell "no filter active" apart
  // from "filter active, matched everything" without recomputing.
  const isFiltered = filtered !== state.repos;

  return (
    <div className="space-y-4">
      <Input
        type="search"
        value={query}
        onChange={(event) => setQuery(event.target.value)}
        placeholder="Filter repositories…"
        aria-label="Filter repositories"
      />
      {isFiltered && (
        <p role="status" className="text-muted-foreground text-sm">
          {filtered.length} of {state.repos.length} repositories
        </p>
      )}
      {filtered.length === 0 ? (
        <p className="text-muted-foreground text-sm">No repositories match &quot;{query.trim()}&quot;.</p>
      ) : (
        <div className="space-y-8">
          {groupBySection(filtered).map(({ section, repos }) => (
            <section key={section ?? UNSECTIONED_LABEL}>
              <h2 className="text-muted-foreground mb-2 text-sm font-medium">{section ?? UNSECTIONED_LABEL}</h2>
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Name</TableHead>
                    <TableHead>Description</TableHead>
                    <TableHead>Owner</TableHead>
                    <TableHead>Last activity</TableHead>
                    <TableHead className="w-px">
                      <span className="sr-only">Links</span>
                    </TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {repos.map((repo) => (
                    <TableRow key={repo.name}>
                      <TableCell className="font-medium">
                        <a className="hover:underline" href={`/${encodeSegment(repo.name)}`}>
                          {repo.name}
                        </a>
                      </TableCell>
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
                      <TableCell className="w-px">
                        <div className="flex items-center gap-2">
                          <IconLink
                            href={logHref(repo.name)}
                            label={`Log for ${repo.name}`}
                            Icon={ClockCounterClockwiseIcon}
                          />
                          <IconLink
                            href={treeHref(repo.name, "", undefined)}
                            label={`Tree for ${repo.name}`}
                            Icon={FolderOpenIcon}
                          />
                        </div>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </section>
          ))}
        </div>
      )}
    </div>
  );
}
