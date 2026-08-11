import { useEffect, useState } from "react";

import { ApiError } from "@/lib/api/client";
import {
  ARCHIVE_FORMATS,
  archiveUrl,
  commitPatchUrl,
  commitRawDiffUrl,
  getCommit,
  getCommitDiff,
} from "@/lib/api/repos";
import type { CommitAuthor, CommitDetail, CommitDiff } from "@/lib/api/schemas";
import { useCommitRefs } from "@/lib/commit-refs";
import { diffApiParams, type DiffViewMode, parseDiffOptions } from "@/lib/diff-options";
import { linkify } from "@/lib/format/linkify";
import { formatAbsoluteTime, formatRelativeTime } from "@/lib/format/time";
import { commitHref, compareHref, treeHref } from "@/lib/repo-href";
import { commitShaFromPathname, paramFromSearch, repoFromPathname } from "@/lib/repo-param";
import AuthorAvatar from "@/components/repo/AuthorAvatar";
import RefBadges from "@/components/repo/RefBadges";
import DiffFileList from "@/components/repo/diff/DiffFileList";
import DiffOptionsBar from "@/components/repo/diff/DiffOptionsBar";
import DiffStatTable from "@/components/repo/diff/DiffStatTable";
import { Skeleton } from "@/components/ui/skeleton";

type State =
  | { status: "loading" }
  | { status: "error"; error: ApiError }
  // `diff` is `null` in stat-only mode (`view=stat`): `detail.diffstat` is
  // already the full, uncapped file list, so the hunk-bearing diff response
  // is never fetched at all — see the effect below.
  | { status: "data"; detail: CommitDetail; diff: CommitDiff | null };

interface CommitViewProps {
  /**
   * Omitted by the prerendered `/{repo}/commit` shell, which is built under
   * a placeholder param (`lib/shell.ts`) — the real repo/sha are read from
   * the URL in the browser. `client:only` guarantees this default is only
   * ever evaluated there.
   */
  repo?: string;
  sha?: string;
  /** `view`/`context`/`ignorews`/`path` follow the same "prop overrides,
   *  `location` is the default source" pattern as `repo`/`sha` — see
   *  `SearchView`. `path` restricts the diff to one file — set when a
   *  stat-view row links into a single-file diff. */
  view?: DiffViewMode;
  context?: number;
  ignorews?: boolean;
  path?: string;
}

/** Also rendered statically into the page shell as the island's
 *  `slot="fallback"`, so the prerendered HTML is not blank. */
export function CommitViewSkeleton() {
  return (
    <div className="space-y-2" aria-busy="true">
      <Skeleton className="h-6 w-2/3" />
      <Skeleton className="h-24 w-full" />
      <Skeleton className="h-48 w-full" />
    </div>
  );
}

function AuthorLine({ label, author, at }: { label: string; author: CommitAuthor; at: string | null }) {
  return (
    <div className="flex items-center gap-2">
      <AuthorAvatar author={author} className="size-6 shrink-0 rounded-full" />
      <span>
        <span className="text-muted-foreground">{label} </span>
        <span className="font-medium">{author.name}</span>
        {at && (
          <>
            {" "}
            <span className="text-muted-foreground" title={formatAbsoluteTime(at)}>
              {formatRelativeTime(at)}
            </span>
          </>
        )}
      </span>
    </div>
  );
}

export default function CommitView({
  repo,
  sha,
  view: viewProp,
  context: contextProp,
  ignorews: ignorewsProp,
  path: pathProp,
}: CommitViewProps) {
  const resolvedRepo = repo ?? repoFromPathname(window.location.pathname);
  const resolvedSha = sha ?? commitShaFromPathname(window.location.pathname);
  const urlOptions = parseDiffOptions(window.location.search);
  const resolvedView = viewProp ?? urlOptions.view;
  const resolvedContext = contextProp ?? urlOptions.context;
  const resolvedIgnorews = ignorewsProp ?? urlOptions.ignorews;
  const resolvedPath = pathProp ?? paramFromSearch("path", window.location.search);
  const options = { view: resolvedView, context: resolvedContext, ignorews: resolvedIgnorews };
  const isStatOnly = resolvedView === "stat";
  const [state, setState] = useState<State>({ status: "loading" });
  const refsBySha = useCommitRefs(resolvedRepo);

  useEffect(() => {
    let cancelled = false;

    // Stat-only mode never fetches the hunk-bearing diff response —
    // `detail.diffstat` is already the full, uncapped file list, so there's
    // nothing the diff response would add.
    const diffPromise: Promise<CommitDiff | null> = isStatOnly
      ? Promise.resolve(null)
      : getCommitDiff(resolvedRepo, resolvedSha, { path: resolvedPath, ...diffApiParams(options) });

    Promise.all([getCommit(resolvedRepo, resolvedSha), diffPromise])
      .then(([detail, diff]) => {
        if (!cancelled) {
          setState({ status: "data", detail, diff });
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
  }, [resolvedRepo, resolvedSha, resolvedContext, resolvedIgnorews, resolvedPath, isStatOnly]);

  if (state.status === "loading") {
    return <CommitViewSkeleton />;
  }

  if (state.status === "error") {
    const message = state.error.status === 404 ? "Commit not found." : state.error.message;
    return (
      <p role="alert" className="text-destructive text-sm">
        Failed to load commit: {message}
      </p>
    );
  }

  const { detail, diff } = state;

  return (
    <div className="space-y-6">
      <div className="space-y-3">
        <div className="flex flex-wrap items-center gap-2">
          <h2 className="text-lg font-medium break-words">{detail.summary ?? "(no commit message)"}</h2>
          <RefBadges repo={resolvedRepo} refs={refsBySha.get(detail.sha) ?? []} />
        </div>
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm">
          <dt className="text-muted-foreground">Commit</dt>
          <dd className="font-mono break-all">{detail.sha}</dd>
          <dt className="text-muted-foreground">Author</dt>
          <dd>
            <AuthorLine label="" author={detail.author} at={detail.authored_at} />
          </dd>
          <dt className="text-muted-foreground">Committer</dt>
          <dd>
            <AuthorLine label="" author={detail.committer} at={detail.committed_at} />
          </dd>
          {detail.parents.length > 0 && (
            <>
              <dt className="text-muted-foreground">Parents</dt>
              <dd className="space-y-1 font-mono">
                {detail.parents.map((parent) => (
                  <div key={parent} className="space-x-2">
                    <a
                      className="underline"
                      href={commitHref(resolvedRepo, parent, {
                        context: resolvedContext,
                        ignorews: resolvedIgnorews,
                      })}
                    >
                      {parent.slice(0, 12)}
                    </a>
                    <a
                      className="text-muted-foreground hover:text-foreground text-xs"
                      href={compareHref(resolvedRepo, {
                        from: parent,
                        to: detail.sha,
                        context: resolvedContext,
                        ignorews: resolvedIgnorews,
                      })}
                      aria-label={`Diff against parent ${parent.slice(0, 12)}`}
                    >
                      (diff)
                    </a>
                  </div>
                ))}
              </dd>
            </>
          )}
        </dl>
        {detail.parents.length > 1 && (
          <p className="text-muted-foreground text-sm">
            This is a merge commit — the diff below is shown against the first parent only. Use the (diff) links above
            to compare against another parent.
          </p>
        )}
        <div className="text-muted-foreground flex flex-wrap gap-x-4 gap-y-1 text-sm">
          <a
            className="hover:text-foreground hover:underline"
            href={treeHref(resolvedRepo, "", detail.sha)}
            aria-label="Browse the tree at this commit"
          >
            Tree
          </a>
          <a
            className="hover:text-foreground hover:underline"
            href={commitRawDiffUrl(resolvedRepo, detail.sha, { path: resolvedPath, ...diffApiParams(options) })}
          >
            Raw diff
          </a>
          <a className="hover:text-foreground hover:underline" href={commitPatchUrl(resolvedRepo, detail.sha)}>
            Patch
          </a>
          {ARCHIVE_FORMATS.map((format) => (
            <a
              key={format}
              className="hover:text-foreground hover:underline"
              href={archiveUrl(resolvedRepo, detail.sha, format)}
            >
              {format}
            </a>
          ))}
        </div>
      </div>

      {detail.message && (
        <pre className="border-border bg-muted/30 overflow-x-auto rounded-md border p-3 text-sm whitespace-pre-wrap">
          {linkify(detail.message, { repo: resolvedRepo })}
        </pre>
      )}

      {detail.note && (
        <section className="space-y-1">
          <h3 className="text-muted-foreground text-sm font-medium">Notes</h3>
          <pre className="border-border bg-muted/30 border-l-primary overflow-x-auto rounded-md border border-l-4 p-3 text-sm whitespace-pre-wrap">
            {linkify(detail.note, { repo: resolvedRepo })}
          </pre>
        </section>
      )}

      {resolvedPath && (
        <p className="text-muted-foreground text-sm">
          Showing only <span className="text-foreground font-mono">{resolvedPath}</span> —{" "}
          <a className="hover:text-foreground underline" href={commitHref(resolvedRepo, detail.sha, options)}>
            Show all files
          </a>
        </p>
      )}

      <DiffStatTable
        stat={detail.diffstat}
        hrefFor={(file) => commitHref(resolvedRepo, detail.sha, { ...options, view: "unified", path: file.path })}
      />

      <DiffOptionsBar
        options={options}
        extraParams={resolvedPath ? { path: resolvedPath } : undefined}
        hrefFor={(patch) => commitHref(resolvedRepo, detail.sha, { ...options, path: resolvedPath, ...patch })}
      />

      {resolvedView !== "stat" && diff && (
        <DiffFileList truncated={diff.truncated} files={diff.files} view={resolvedView} />
      )}
    </div>
  );
}
