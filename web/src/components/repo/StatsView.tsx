import { useEffect, useState } from "react";

import { Bar, BarChart, CartesianGrid, XAxis, YAxis } from "recharts";

import { ApiError } from "@/lib/api/client";
import { getStats } from "@/lib/api/repos";
import type { BucketStats, StatsPeriod, StatsResults } from "@/lib/api/schemas";
import { paramFromSearch, repoFromPathname } from "@/lib/repo-param";
import { statsHref } from "@/lib/repo-href";
import { cn } from "@/lib/utils";
import AuthorAvatar from "@/components/repo/AuthorAvatar";
import { type ChartConfig, ChartContainer, ChartTooltip, ChartTooltipContent } from "@/components/ui/chart";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableFooter, TableHead, TableHeader, TableRow } from "@/components/ui/table";

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

const chartConfig: ChartConfig = {
  commits: { label: "Commits", color: "var(--chart-1)" },
};

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

      <ChartContainer config={chartConfig} className="aspect-auto h-64 w-full">
        <BarChart data={chartData} margin={{ left: 0, right: 0, top: 8, bottom: 0 }}>
          <CartesianGrid vertical={false} />
          <XAxis dataKey="label" tickLine={false} axisLine={false} tickMargin={8} />
          <YAxis tickLine={false} axisLine={false} width={32} allowDecimals={false} />
          <ChartTooltip cursor={false} content={<ChartTooltipContent />} />
          {/* Recharts omits the bar element entirely for a zero value — with
              no mark there's no hover/tooltip hit target for that bucket
              (`references/interaction.md`'s "the mark is the hit target"
              rule). `minPointSize` keeps a thin sliver so every bucket stays
              hoverable, a zero-commit month included. */}
          <Bar dataKey="commits" fill="var(--color-commits)" radius={[4, 4, 0, 0]} maxBarSize={24} minPointSize={2} />
        </BarChart>
      </ChartContainer>

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
          </TableBody>
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
