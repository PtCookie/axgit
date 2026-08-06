import { lazy, Suspense, useEffect, useState } from "react";

import { ApiError } from "@/lib/api/client";
import { getStats } from "@/lib/api/repos";
import type { BucketStats, StatsPeriod, StatsResults } from "@/lib/api/schemas";
import { paramFromSearch, repoFromPathname } from "@/lib/repo-param";
import { statsHref } from "@/lib/repo-href";
import { cn } from "@/lib/utils";
import AuthorAvatar from "@/components/repo/AuthorAvatar";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableFooter, TableHead, TableHeader, TableRow } from "@/components/ui/table";

// Module scope, not inside a component — otherwise every render would mint a
// new lazy type and remount the chart. `importStatsChart` is also called
// directly (outside `lazy()`) to warm the chunk in parallel with the `/stats`
// fetch below, since this page renders a chart for almost every response.
const importStatsChart = () => import("@/components/repo/StatsChart");
const StatsChart = lazy(importStatsChart);

type State = { status: "loading" } | { status: "error"; error: ApiError } | { status: "data"; results: StatsResults };

interface StatsViewProps {
  /**
   * Omitted by the prerendered `/{repo}/stats` shell, which is built under a
   * placeholder param (`lib/shell.ts`) — the real name is read from the URL
   * in the browser. `client:only` guarantees this default is only ever
   * evaluated there.
   */
  repo?: string;
  /** `period`/`ref` follow the same "prop overrides, `location` is the
   *  default source" pattern as `repo` — see `CommitLog`/`SearchView`. */
  period?: string;
  ref?: string;
}

const PERIOD_OPTIONS: { value: StatsPeriod; label: string }[] = [
  { value: "week", label: "Week" },
  { value: "month", label: "Month" },
  { value: "quarter", label: "Quarter" },
  { value: "year", label: "Year" },
];

function isStatsPeriod(value: string | undefined): value is StatsPeriod {
  return value === "week" || value === "month" || value === "quarter" || value === "year";
}

/** Short, period-appropriate label for a bucket's `start` (UTC RFC 3339) —
 *  `timeZone: "UTC"` keeps this aligned with the API's UTC bucket boundaries
 *  regardless of the viewer's local zone. */
function bucketLabel(startIso: string, period: StatsPeriod): string {
  const date = new Date(startIso);
  switch (period) {
    case "week":
      return new Intl.DateTimeFormat("en", { month: "short", day: "numeric", timeZone: "UTC" }).format(date);
    case "month":
      return new Intl.DateTimeFormat("en", { month: "short", year: "numeric", timeZone: "UTC" }).format(date);
    case "quarter": {
      const quarter = Math.floor(date.getUTCMonth() / 3) + 1;
      return `Q${quarter} ${date.getUTCFullYear()}`;
    }
    case "year":
      return String(date.getUTCFullYear());
  }
}

/** Also rendered statically into the page shell as the island's
 *  `slot="fallback"`, so the prerendered HTML is not blank. */
export function StatsViewSkeleton() {
  return (
    <div className="space-y-4" aria-busy="true">
      <Skeleton className="h-8 w-64" />
      <Skeleton className="h-64 w-full" />
      <Skeleton className="h-32 w-full" />
    </div>
  );
}

export default function StatsView({ repo, period: periodParam, ref: refParam }: StatsViewProps) {
  const resolvedRepo = repo ?? repoFromPathname(window.location.pathname);
  const rawPeriod = periodParam ?? paramFromSearch("period", window.location.search);
  const resolvedPeriod: StatsPeriod = isStatsPeriod(rawPeriod) ? rawPeriod : "month";
  const resolvedRef = refParam ?? paramFromSearch("ref", window.location.search);
  const [state, setState] = useState<State>({ status: "loading" });

  useEffect(() => {
    let cancelled = false;

    // Warm the chart chunk in parallel with the API call rather than after
    // it — this page renders a chart for almost every response, so there's
    // no reason to serialize the two.
    void importStatsChart();

    getStats(resolvedRepo, { period: resolvedPeriod, ref: resolvedRef })
      .then((results) => {
        if (!cancelled) {
          setState({ status: "data", results });
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
  }, [resolvedRepo, resolvedPeriod, resolvedRef]);

  const periodSwitcher = (
    <nav className="flex flex-wrap gap-2" aria-label="Bucket period">
      {PERIOD_OPTIONS.map((option) => (
        <a
          key={option.value}
          href={statsHref(resolvedRepo, { period: option.value, ref: resolvedRef })}
          aria-current={option.value === resolvedPeriod ? "page" : undefined}
          className={cn(
            "rounded-3xl border px-3 py-1 text-sm font-medium",
            option.value === resolvedPeriod
              ? "border-primary bg-primary/10 text-foreground"
              : "border-input text-muted-foreground hover:text-foreground",
          )}
        >
          {option.label}
        </a>
      ))}
    </nav>
  );

  if (state.status === "loading") {
    return (
      <div className="space-y-4">
        {periodSwitcher}
        <StatsViewSkeleton />
      </div>
    );
  }

  if (state.status === "error") {
    const message = state.error.status === 404 ? "Repository not found." : state.error.message;
    return (
      <div className="space-y-4">
        {periodSwitcher}
        <p role="alert" className="text-destructive text-sm">
          Failed to load statistics: {message}
        </p>
      </div>
    );
  }

  const { results } = state;

  return (
    <div className="space-y-4">
      {periodSwitcher}
      {results.sha === null ? (
        <p className="text-muted-foreground text-sm">No commits yet.</p>
      ) : (
        <StatsResultsView results={results} period={resolvedPeriod} />
      )}
    </div>
  );
}

interface StatsResultsViewProps {
  results: StatsResults;
  period: StatsPeriod;
}

function StatsResultsView({ results, period }: StatsResultsViewProps) {
  const chartData = results.buckets.map((bucket) => ({
    label: bucketLabel(bucket.start, period),
    commits: bucket.commits,
  }));
  const bucketColumns: { start: string; label: string }[] = results.buckets.map((bucket: BucketStats) => ({
    start: bucket.start,
    label: bucketLabel(bucket.start, period),
  }));
  const totalCommits = results.buckets.reduce((sum, bucket) => sum + bucket.commits, 0);

  return (
    <div className="space-y-6">
      {results.truncated && (
        <p role="status" className="text-muted-foreground text-sm">
          Showing a partial result — narrow the window or raise the limit for a complete match set.
        </p>
      )}

      <Suspense fallback={<Skeleton className="h-64 w-full" />}>
        <StatsChart data={chartData} />
      </Suspense>

      <div className="space-y-2">
        <p className="text-muted-foreground text-sm">
          {results.authors.length < results.author_count
            ? `Showing top ${results.authors.length} of ${results.author_count} authors.`
            : `${results.author_count} author${results.author_count === 1 ? "" : "s"}.`}
        </p>
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Author</TableHead>
              <TableHead className="text-right">Commits</TableHead>
              {bucketColumns.map((bucket) => (
                <TableHead key={bucket.start} className="text-right">
                  {bucket.label}
                </TableHead>
              ))}
            </TableRow>
          </TableHeader>
          <TableBody>
            {results.authors.length === 0 ? (
              <TableRow>
                <TableCell colSpan={2 + bucketColumns.length} className="text-muted-foreground">
                  No commits in this window.
                </TableCell>
              </TableRow>
            ) : (
              results.authors.map((author) => (
                <TableRow key={author.author.email_hash}>
                  <TableCell className="font-medium">
                    <div className="flex items-center gap-2">
                      <AuthorAvatar author={author.author} className="size-5 shrink-0 rounded-full" />
                      {author.author.name}
                    </div>
                  </TableCell>
                  <TableCell className="text-right">{author.commits}</TableCell>
                  {author.buckets.map((commits, index) => (
                    <TableCell key={bucketColumns[index].start} className="text-muted-foreground text-right">
                      {commits}
                    </TableCell>
                  ))}
                </TableRow>
              ))
            )}
            {/* The authors cut by `limit`, aggregated rather than dropped
             *  (docs/API.md). No `AuthorAvatar`: the identicon seeds from
             *  `email_hash` and this row has no identity — reading as plainly
             *  different from a real author is the point. */}
            {results.others && (
              <TableRow>
                <TableCell className="text-muted-foreground font-medium">Others ({results.others.count})</TableCell>
                <TableCell className="text-muted-foreground text-right">{results.others.commits}</TableCell>
                {results.others.buckets.map((commits, index) => (
                  <TableCell key={bucketColumns[index].start} className="text-muted-foreground text-right">
                    {commits}
                  </TableCell>
                ))}
              </TableRow>
            )}
          </TableBody>
          {/* Totals come from the top-level `buckets`, not from summing the
           *  rows above — they have always included authors cut by `limit`
           *  (docs/API.md). Since `others` landed those authors get their own
           *  row, so the two now agree column for column; that's a property of
           *  the response, not something to enforce by re-deriving here. */}
          <TableFooter>
            <TableRow>
              <TableCell>Total</TableCell>
              <TableCell className="text-right">{totalCommits}</TableCell>
              {results.buckets.map((bucket) => (
                <TableCell key={bucket.start} className="text-right">
                  {bucket.commits}
                </TableCell>
              ))}
            </TableRow>
          </TableFooter>
        </Table>
      </div>
    </div>
  );
}
