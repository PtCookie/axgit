import { useEffect, useState } from "react";

import { ApiError } from "@/lib/api/client";
import { archiveUrl, feedUrl, getRepo } from "@/lib/api/repos";
import type { RepoSummary as RepoSummaryData } from "@/lib/api/schemas";
import { formatAbsoluteTime, formatRelativeTime } from "@/lib/format/time";
import { repoFromPathname } from "@/lib/repo-param";
import { Skeleton } from "@/components/ui/skeleton";

type State =
  { status: "loading" } | { status: "error"; error: ApiError } | { status: "data"; summary: RepoSummaryData };

interface RepoSummaryProps {
  /**
   * Omitted by the prerendered `/{repo}` shell, which is built under a
   * placeholder param (`lib/shell.ts`) — the real name is read from the URL
   * in the browser. `client:only` guarantees this default is only ever
   * evaluated there.
   */
  repo?: string;
}

/** Also rendered statically into the page shell as the island's
 *  `slot="fallback"`, so the prerendered HTML is not blank. */
export function RepoSummarySkeleton() {
  return (
    <div className="space-y-2" aria-busy="true">
      <Skeleton className="h-5 w-2/3" />
      <Skeleton className="h-24 w-full" />
    </div>
  );
}

export default function RepoSummary({ repo }: RepoSummaryProps) {
  const resolvedRepo = repo ?? repoFromPathname(window.location.pathname);
  const [state, setState] = useState<State>({ status: "loading" });

  useEffect(() => {
    let cancelled = false;

    getRepo(resolvedRepo)
      .then((summary) => {
        if (!cancelled) {
          setState({ status: "data", summary });
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
  }, [resolvedRepo]);

  if (state.status === "loading") {
    return <RepoSummarySkeleton />;
  }

  if (state.status === "error") {
    const message = state.error.status === 404 ? "Repository not found." : state.error.message;
    return (
      <p role="alert" className="text-destructive text-sm">
        Failed to load repository: {message}
      </p>
    );
  }

  const { summary } = state;

  return (
    <div className="space-y-6">
      {summary.description && <p className="text-foreground">{summary.description}</p>}

      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm">
        {summary.section && (
          <>
            <dt className="text-muted-foreground">Section</dt>
            <dd>{summary.section}</dd>
          </>
        )}
        {summary.owner && (
          <>
            <dt className="text-muted-foreground">Owner</dt>
            <dd>{summary.owner}</dd>
          </>
        )}
        <dt className="text-muted-foreground">Default branch</dt>
        <dd>{summary.default_branch ?? "—"}</dd>
        <dt className="text-muted-foreground">Last activity</dt>
        <dd>
          {summary.last_modified ? (
            <span title={formatAbsoluteTime(summary.last_modified)}>{formatRelativeTime(summary.last_modified)}</span>
          ) : (
            "—"
          )}
        </dd>
        <dt className="text-muted-foreground">HEAD</dt>
        <dd className="font-mono">{summary.head ?? "—"}</dd>
        <dt className="text-muted-foreground">Branches</dt>
        <dd>{summary.branch_count}</dd>
        <dt className="text-muted-foreground">Tags</dt>
        <dd>{summary.tag_count}</dd>
        {summary.clone_url && (
          <>
            <dt className="text-muted-foreground">Clone</dt>
            <dd className="font-mono break-all">{summary.clone_url}</dd>
          </>
        )}
        {summary.head !== null && (
          <>
            <dt className="text-muted-foreground">Download</dt>
            <dd className="space-x-3">
              <a
                href={archiveUrl(summary.name, undefined, "tar.gz")}
                className="text-primary underline underline-offset-2"
              >
                tar.gz
              </a>
              <a
                href={archiveUrl(summary.name, undefined, "zip")}
                className="text-primary underline underline-offset-2"
              >
                zip
              </a>
            </dd>
            <dt className="text-muted-foreground">Feed</dt>
            <dd>
              <a href={feedUrl(summary.name)} className="text-primary underline underline-offset-2">
                Atom
              </a>
            </dd>
          </>
        )}
      </dl>

      {summary.head === null && <p className="text-muted-foreground text-sm">No commits yet.</p>}
    </div>
  );
}
