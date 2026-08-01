import { useEffect, useState } from "react";

import { ApiError } from "@/lib/api/client";
import { encodeSegment } from "@/lib/api/path";
import { listCommits } from "@/lib/api/repos";
import type { CommitsPage } from "@/lib/api/schemas";
import { formatAbsoluteTime, formatRelativeTime } from "@/lib/format/time";
import { paramFromSearch, repoFromPathname } from "@/lib/repo-param";
import AuthorAvatar from "@/components/repo/AuthorAvatar";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

type State = { status: "loading" } | { status: "error"; error: ApiError } | { status: "data"; page: CommitsPage };

interface CommitLogProps {
  /**
   * Omitted by the prerendered `/{repo}/log` shell, which is built under a
   * placeholder param (`lib/shell.ts`) — the real name is read from the URL
   * in the browser. `client:only` guarantees this default is only ever
   * evaluated there.
   */
  repo?: string;
  /**
   * `ref`/`path`/`cursor` follow the same "prop overrides, `location` is the
   * default source" pattern as `repo`. Navigating between log pages (e.g.
   * "Older →") is a client-side transition to a new `/{repo}/log?...` URL
   * (DECISIONS.md #24) — `<main>` isn't persisted across it (`Layout.astro`),
   * so this component always remounts fresh against the new query string
   * rather than receiving these as changed props on a live instance. They
   * still live in the URL's query string rather than in any router state.
   */
  ref?: string;
  path?: string;
  cursor?: string;
}

/** Also rendered statically into the page shell as the island's
 *  `slot="fallback"`, so the prerendered HTML is not blank. */
export function CommitLogSkeleton() {
  return (
    <div className="space-y-2" aria-busy="true">
      <Skeleton className="h-8 w-full" />
      <Skeleton className="h-8 w-full" />
      <Skeleton className="h-8 w-full" />
    </div>
  );
}

/** Builds a `/{repo}/log` href, keeping the given params and clearing any
 *  key set to `undefined` in `overrides`. */
function logHref(
  repo: string,
  current: { ref?: string; path?: string },
  overrides: Record<string, string | undefined>,
) {
  const params = new URLSearchParams();
  const merged = { ...current, ...overrides };
  for (const [key, value] of Object.entries(merged)) {
    if (value) params.set(key, value);
  }
  const query = params.toString();
  return `/${encodeSegment(repo)}/log${query ? `?${query}` : ""}`;
}

export default function CommitLog({ repo, ref: refParam, path: pathParam, cursor: cursorParam }: CommitLogProps) {
  const resolvedRepo = repo ?? repoFromPathname(window.location.pathname);
  const resolvedRef = refParam ?? paramFromSearch("ref", window.location.search);
  const resolvedPath = pathParam ?? paramFromSearch("path", window.location.search);
  const resolvedCursor = cursorParam ?? paramFromSearch("cursor", window.location.search);
  const [state, setState] = useState<State>({ status: "loading" });

  useEffect(() => {
    let cancelled = false;

    // `resolvedRef`/`resolvedPath`/`resolvedCursor` never actually change on
    // an already-mounted instance in production — `<main>` isn't persisted
    // across navigations (DECISIONS.md #24), so a new URL always remounts
    // this component fresh instead of updating its props in place. This
    // effect's dependency array only matters for tests, which render the
    // component directly and change props on a live instance; staying on the
    // previous result until the new one arrives is preferable to a loading
    // flash there too.
    listCommits(resolvedRepo, { ref: resolvedRef, path: resolvedPath, cursor: resolvedCursor })
      .then((page) => {
        if (!cancelled) {
          setState({ status: "data", page });
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
  }, [resolvedRepo, resolvedRef, resolvedPath, resolvedCursor]);

  if (state.status === "loading") {
    return <CommitLogSkeleton />;
  }

  if (state.status === "error") {
    const message = state.error.status === 404 ? "Repository not found." : state.error.message;
    return (
      <p role="alert" className="text-destructive text-sm">
        Failed to load commits: {message}
      </p>
    );
  }

  const { page } = state;
  const current = { ref: resolvedRef, path: resolvedPath };

  return (
    <div className="space-y-4">
      {resolvedPath && (
        <p className="text-muted-foreground text-sm">
          Filtered by path <code className="text-foreground">{resolvedPath}</code> —{" "}
          <a className="underline" href={logHref(resolvedRepo, current, { path: undefined, cursor: undefined })}>
            clear filter
          </a>
        </p>
      )}

      {page.commits.length === 0 ? (
        <p className="text-muted-foreground text-sm">No commits yet.</p>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Author</TableHead>
              <TableHead>Summary</TableHead>
              <TableHead>Commit</TableHead>
              <TableHead>Committed</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {page.commits.map((commit) => (
              <TableRow key={commit.sha}>
                <TableCell>
                  <div className="flex items-center gap-2">
                    <AuthorAvatar author={commit.author} className="size-6 shrink-0 rounded-full" />
                    <span className="text-muted-foreground">{commit.author.name}</span>
                  </div>
                </TableCell>
                <TableCell className="font-medium">
                  <a
                    className="hover:underline"
                    href={`/${encodeSegment(resolvedRepo)}/commit/${encodeSegment(commit.sha)}`}
                  >
                    {commit.summary ?? "(no commit message)"}
                  </a>
                </TableCell>
                <TableCell className="text-muted-foreground font-mono">{commit.sha.slice(0, 12)}</TableCell>
                <TableCell className="text-muted-foreground">
                  {commit.authored_at ? (
                    <span title={formatAbsoluteTime(commit.authored_at)}>{formatRelativeTime(commit.authored_at)}</span>
                  ) : (
                    "—"
                  )}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}

      {page.next_cursor && (
        <a
          className="text-muted-foreground hover:text-foreground text-sm underline"
          href={logHref(resolvedRepo, current, { cursor: page.next_cursor })}
        >
          Older →
        </a>
      )}
    </div>
  );
}
