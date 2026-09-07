import { CaretDownIcon } from "@phosphor-icons/react/dist/ssr/CaretDown";
import { CaretUpIcon } from "@phosphor-icons/react/dist/ssr/CaretUp";
import { ClockCounterClockwiseIcon } from "@phosphor-icons/react/dist/ssr/ClockCounterClockwise";
import { FolderOpenIcon } from "@phosphor-icons/react/dist/ssr/FolderOpen";
import { HouseIcon } from "@phosphor-icons/react/dist/ssr/House";
import { useEffect, useState } from "react";

import { ApiError } from "@/lib/api/client";
import { encodeSegment } from "@/lib/api/path";
import { listRepos } from "@/lib/api/repos";
import type { RepoInfo } from "@/lib/api/schemas";
import { formatAbsoluteTime, formatRelativeTime } from "@/lib/format/time";
import { filterRepos } from "@/lib/repo-filter";
import { groupBySection } from "@/lib/repo-group";
import { paramFromSearch } from "@/lib/repo-param";
import { logHref, treeHref } from "@/lib/repo-href";
import { defaultOrder, orderToParam, parseOrder, sortRepos, type RepoOrder, type RepoSortKey } from "@/lib/repo-sort";
import IconLink from "@/components/IconLink";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

const UNSECTIONED_LABEL = "Uncategorized";
/** Query-string param the filter box reads its initial value from and keeps
 *  in sync with (`?q=`, docs/DECISIONS.md #25). */
const QUERY_PARAM = "q";
/** Query-string param the sortable column headers read/write
 *  (`?sort=`, docs/DECISIONS.md #65) — the client-side twin of the API's own
 *  `?sort=` (docs/DECISIONS.md #64), applied here in memory rather than by
 *  refetching, since the whole list is already in hand. */
const SORT_PARAM = "sort";

/** Sortable columns, in header order. `section` has no header button — it's
 *  the group heading (`groupBySection`), not a column — but stays reachable
 *  via `?sort=section` or `AXGIT_REPOSITORY_SORT`. Since `groupBySection`
 *  always orders groups itself (unsectioned first, then most recently active
 *  first), `sort=section` only affects the within-group order (which, since
 *  every repo in a group shares the same section, collapses to the `name`
 *  tiebreak) — not which group comes first. */
const SORT_COLUMNS: readonly { key: RepoSortKey; label: string }[] = [
  { key: "name", label: "Name" },
  { key: "desc", label: "Description" },
  { key: "owner", label: "Owner" },
  { key: "idle", label: "Last activity" },
];

type State =
  { status: "loading" } | { status: "error"; error: ApiError } | { status: "data"; repos: RepoInfo[]; sort: string };

/** A clickable `TableHead` for a sortable column: toggles direction when
 *  already active, else switches to `sortKey`'s own default direction
 *  (`defaultOrder`) — clicking a different column never inherits the
 *  previous one's direction. */
function SortableHead({
  label,
  sortKey,
  order,
  onSort,
}: {
  label: string;
  sortKey: RepoSortKey;
  order: RepoOrder;
  onSort: (key: RepoSortKey) => void;
}) {
  const active = order.key === sortKey;
  return (
    <TableHead aria-sort={active ? (order.reverse ? "descending" : "ascending") : "none"}>
      <button
        type="button"
        onClick={() => onSort(sortKey)}
        className="text-foreground hover:text-foreground inline-flex items-center gap-1 font-medium"
      >
        {label}
        {active &&
          (order.reverse ? (
            <CaretDownIcon className="size-3" aria-hidden="true" />
          ) : (
            <CaretUpIcon className="size-3" aria-hidden="true" />
          ))}
      </button>
    </TableHead>
  );
}

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
  // `undefined` means "no client-side override" — the fetched list is
  // already in whatever order the API applied (`state.sort`, its own
  // `?sort=`/`AXGIT_REPOSITORY_SORT` default). Set only once the user clicks
  // a column header, or seeded from a deep-linked `?sort=`.
  const [sortParam, setSortParam] = useState<string | undefined>(() =>
    paramFromSearch(SORT_PARAM, window.location.search),
  );

  useEffect(() => {
    let cancelled = false;

    listRepos()
      .then((response) => {
        if (!cancelled) {
          setState({ status: "data", repos: response.repos, sort: response.sort });
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

  // Mirrors `?sort=` the same way the effect above mirrors `?q=` — same
  // `replaceState`-not-`pushState` rationale (docs/DECISIONS.md #25).
  useEffect(() => {
    const url = new URL(window.location.href);
    if (sortParam === undefined) {
      url.searchParams.delete(SORT_PARAM);
    } else {
      url.searchParams.set(SORT_PARAM, sortParam);
    }
    if (url.href !== window.location.href) {
      window.history.replaceState(window.history.state, "", url);
    }
  }, [sortParam]);

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

  // A client-side override (a header click, or a deep-linked `?sort=`) wins;
  // otherwise the fetched order — and `state.sort` — reflect the API's own
  // `?sort=`/`AXGIT_REPOSITORY_SORT` default. An unparseable `sortParam`
  // (an unrecognized deep link) falls back the same way as if it were unset.
  const clientOrder = sortParam !== undefined ? parseOrder(sortParam) : undefined;
  const order = clientOrder ?? parseOrder(state.sort) ?? defaultOrder("name");
  const displayedRepos = clientOrder ? sortRepos(state.repos, clientOrder) : state.repos;

  function handleSort(key: RepoSortKey) {
    const next = order.key === key ? { key, reverse: !order.reverse } : defaultOrder(key);
    setSortParam(orderToParam(next));
  }

  const filtered = filterRepos(displayedRepos, query);
  // `filterRepos` returns the same array reference when the query is
  // empty/whitespace-only — a cheap way to tell "no filter active" apart
  // from "filter active, matched everything" without recomputing.
  const isFiltered = filtered !== displayedRepos;

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
              {/* The unsectioned group has no visible label — it's the "everything else" bucket
                  pinned to the top, not a category — but keeps a heading for screen readers so
                  the section landmark stays announced like every other group's. */}
              <h2 className={section === null ? "sr-only" : "text-muted-foreground mb-2 text-sm font-medium"}>
                {section ?? UNSECTIONED_LABEL}
              </h2>
              <Table>
                <TableHeader>
                  <TableRow>
                    {SORT_COLUMNS.map(({ key, label }) => (
                      <SortableHead key={key} sortKey={key} label={label} order={order} onSort={handleSort} />
                    ))}
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
                          {repo.homepage && (
                            <IconLink
                              href={repo.homepage}
                              label={`Homepage for ${repo.name}`}
                              Icon={HouseIcon}
                              external
                            />
                          )}
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
