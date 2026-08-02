import { lazy, Suspense, useEffect, useState } from "react";

import { ApiError } from "@/lib/api/client";
import { getReadme } from "@/lib/api/repos";
import type { ReadmeInfo } from "@/lib/api/schemas";
import { repoFromPathname } from "@/lib/repo-param";
import { Skeleton } from "@/components/ui/skeleton";

// Module scope, not inside the component — otherwise every render would mint
// a new lazy type and remount the markdown tree. Deliberately *not*
// pre-warmed alongside the `getReadme` fetch below (contrast `StatsView`'s
// `importStatsChart`): the whole point of this split is that a repository
// with no README, or a `rst`/`plain` one, never downloads react-markdown at
// all — pre-warming would defeat that before `format` is even known.
const ReadmeMarkdown = lazy(() => import("./ReadmeMarkdown"));

type State =
  | { status: "loading" }
  | { status: "error"; error: ApiError }
  /** No README candidate found (`path_not_found`) or an unborn HEAD
   *  (`ref_not_found`) — both are a normal "nothing to show" outcome, not
   *  an error (docs/API.md's readme section). */
  | { status: "empty" }
  | { status: "data"; readme: ReadmeInfo };

interface ReadmeViewProps {
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
export function ReadmeViewSkeleton() {
  return (
    <div className="space-y-2" aria-busy="true">
      <Skeleton className="h-5 w-1/3" />
      <Skeleton className="h-40 w-full" />
    </div>
  );
}

export default function ReadmeView({ repo }: ReadmeViewProps) {
  const resolvedRepo = repo ?? repoFromPathname(window.location.pathname);
  const [state, setState] = useState<State>({ status: "loading" });

  useEffect(() => {
    let cancelled = false;

    getReadme(resolvedRepo)
      .then((readme) => {
        if (!cancelled) {
          setState({ status: "data", readme });
        }
      })
      .catch((error: unknown) => {
        if (cancelled) {
          return;
        }
        if (error instanceof ApiError && error.status === 404) {
          setState({ status: "empty" });
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
    return <ReadmeViewSkeleton />;
  }

  if (state.status === "empty") {
    return null;
  }

  if (state.status === "error") {
    return (
      <p role="alert" className="text-destructive text-sm">
        Failed to load README: {state.error.message}
      </p>
    );
  }

  const { readme } = state;

  return (
    <div className="space-y-3">
      <h2 className="text-muted-foreground font-mono text-xs tracking-wide uppercase">{readme.path}</h2>
      {readme.format === "markdown" ? (
        <Suspense fallback={<Skeleton className="h-40 w-full" />}>
          <ReadmeMarkdown repo={resolvedRepo} content={readme.content} />
        </Suspense>
      ) : (
        <pre className="border-border overflow-x-auto rounded-md border p-3 font-mono text-sm whitespace-pre-wrap">
          {readme.content}
        </pre>
      )}
    </div>
  );
}
