import { useEffect, useState } from "react";

import { ApiError } from "@/lib/api/client";
import { commitPatchUrl, commitRawDiffUrl, getCommit, getCommitDiff } from "@/lib/api/repos";
import type { CommitAuthor, CommitDetail, CommitDiff } from "@/lib/api/schemas";
import { useCommitRefs } from "@/lib/commit-refs";
import { diffApiParams, parseDiffOptions } from "@/lib/diff-options";
import { linkify } from "@/lib/format/linkify";
import { formatAbsoluteTime, formatRelativeTime } from "@/lib/format/time";
import { commitHref, treeHref } from "@/lib/repo-href";
import { commitShaFromPathname, repoFromPathname } from "@/lib/repo-param";
import AuthorAvatar from "@/components/repo/AuthorAvatar";
import RefBadges from "@/components/repo/RefBadges";
import DiffFileList from "@/components/repo/diff/DiffFileList";
import DiffOptionsBar from "@/components/repo/diff/DiffOptionsBar";
import DiffStatTable from "@/components/repo/diff/DiffStatTable";
import { Skeleton } from "@/components/ui/skeleton";

type State =
  | { status: "loading" }
  | { status: "error"; error: ApiError }
  | { status: "data"; detail: CommitDetail; diff: CommitDiff };

interface CommitViewProps {
  /**
   * Omitted by the prerendered `/{repo}/commit` shell, which is built under
   * a placeholder param (`lib/shell.ts`) — the real repo/sha are read from
   * the URL in the browser. `client:only` guarantees this default is only
   * ever evaluated there.
   */
  repo?: string;
  sha?: string;
  /** `context`/`ignorews` follow the same "prop overrides, `location` is the
   *  default source" pattern as `repo`/`sha` — see `SearchView`. */
  context?: number;
  ignorews?: boolean;
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

export default function CommitView({ repo, sha, context: contextProp, ignorews: ignorewsProp }: CommitViewProps) {
  const resolvedRepo = repo ?? repoFromPathname(window.location.pathname);
  const resolvedSha = sha ?? commitShaFromPathname(window.location.pathname);
  const urlOptions = parseDiffOptions(window.location.search);
  const resolvedContext = contextProp ?? urlOptions.context;
  const resolvedIgnorews = ignorewsProp ?? urlOptions.ignorews;
  const options = { view: "unified" as const, context: resolvedContext, ignorews: resolvedIgnorews };
  const [state, setState] = useState<State>({ status: "loading" });
  const refsBySha = useCommitRefs(resolvedRepo);

  useEffect(() => {
    let cancelled = false;

    Promise.all([
      getCommit(resolvedRepo, resolvedSha),
      getCommitDiff(resolvedRepo, resolvedSha, diffApiParams(options)),
    ])
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
  }, [resolvedRepo, resolvedSha, resolvedContext, resolvedIgnorews]);

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
              <dd className="space-x-2 font-mono">
                {detail.parents.map((parent) => (
                  <a
                    key={parent}
                    className="underline"
                    href={commitHref(resolvedRepo, parent, { context: resolvedContext, ignorews: resolvedIgnorews })}
                  >
                    {parent.slice(0, 12)}
                  </a>
                ))}
              </dd>
            </>
          )}
        </dl>
        {detail.parents.length > 1 && (
          <p className="text-muted-foreground text-sm">
            This is a merge commit — the diff below is shown against the first parent only.
          </p>
        )}
        <div className="text-muted-foreground flex flex-wrap gap-x-4 gap-y-1 text-sm">
          <a className="hover:text-foreground hover:underline" href={treeHref(resolvedRepo, "", detail.sha)}>
            Tree
          </a>
          <a
            className="hover:text-foreground hover:underline"
            href={commitRawDiffUrl(resolvedRepo, detail.sha, diffApiParams(options))}
          >
            Raw diff
          </a>
          <a className="hover:text-foreground hover:underline" href={commitPatchUrl(resolvedRepo, detail.sha)}>
            Patch
          </a>
        </div>
      </div>

      {detail.message && (
        <pre className="border-border bg-muted/30 overflow-x-auto rounded-md border p-3 text-sm whitespace-pre-wrap">
          {linkify(detail.message, { repo: resolvedRepo })}
        </pre>
      )}

      <DiffStatTable stat={detail.diffstat} />

      <DiffOptionsBar options={options} />

      <DiffFileList truncated={diff.truncated} files={diff.files} />
    </div>
  );
}
