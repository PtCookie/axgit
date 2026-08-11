import { useEffect, useState } from "react";

import { ApiError } from "@/lib/api/client";
import { ARCHIVE_FORMATS, archiveUrl, getTag } from "@/lib/api/repos";
import type { TagDetail } from "@/lib/api/schemas";
import { linkify } from "@/lib/format/linkify";
import { formatAbsoluteTime, formatRelativeTime } from "@/lib/format/time";
import { commitHref, logHref, objectHref, treeHref } from "@/lib/repo-href";
import { repoFromPathname, tagNameFromPathname } from "@/lib/repo-param";
import AuthorAvatar from "@/components/repo/AuthorAvatar";
import { Skeleton } from "@/components/ui/skeleton";

type State = { status: "loading" } | { status: "error"; error: ApiError } | { status: "data"; detail: TagDetail };

interface TagViewProps {
  /**
   * Omitted by the prerendered `/{repo}/tag` shell, which is built under a
   * placeholder param (`lib/shell.ts`) — the real repo/name are read from
   * the URL in the browser. `client:only` guarantees this default is only
   * ever evaluated there.
   */
  repo?: string;
  name?: string;
}

/** Also rendered statically into the page shell as the island's
 *  `slot="fallback"`, so the prerendered HTML is not blank. */
export function TagViewSkeleton() {
  return (
    <div className="space-y-2" aria-busy="true">
      <Skeleton className="h-6 w-2/3" />
      <Skeleton className="h-24 w-full" />
    </div>
  );
}

export default function TagView({ repo, name }: TagViewProps) {
  const resolvedRepo = repo ?? repoFromPathname(window.location.pathname);
  const resolvedName = name ?? tagNameFromPathname(window.location.pathname);
  const [state, setState] = useState<State>({ status: "loading" });

  useEffect(() => {
    let cancelled = false;

    getTag(resolvedRepo, resolvedName)
      .then((detail) => {
        if (!cancelled) {
          setState({ status: "data", detail });
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
  }, [resolvedRepo, resolvedName]);

  if (state.status === "loading") {
    return <TagViewSkeleton />;
  }

  if (state.status === "error") {
    const message = state.error.status === 404 ? "Tag not found." : state.error.message;
    return (
      <p role="alert" className="text-destructive text-sm">
        Failed to load tag: {message}
      </p>
    );
  }

  const { detail } = state;
  // Only when the tag's one-level dereference is itself a commit does its
  // sha equal `target` — showing a separate "Commit" row then would just
  // repeat the "Object" row above it. A nested tag or a tag on a tree/blob
  // is exactly when the two differ (or `target` is null), so the row earns
  // its place.
  const showCommitRow = detail.target !== null && detail.target !== detail.object.sha;
  // A tag that never reaches a commit can't be archived (`git archive`
  // needs a treeish) and its Tree/Log links would 404 the same way — gate
  // all three on `target`, not just the archive links.
  const canBrowse = detail.target !== null;

  return (
    <div className="space-y-6">
      <div className="space-y-3">
        <h2 className="text-lg font-medium break-all">{detail.name}</h2>
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm">
          {detail.tag_object !== null && (
            <>
              <dt className="text-muted-foreground">Tag object</dt>
              <dd className="font-mono break-all">{detail.tag_object}</dd>
            </>
          )}
          {detail.tagger !== null && (
            <>
              <dt className="text-muted-foreground">Tagger</dt>
              <dd className="flex items-center gap-2">
                <AuthorAvatar author={detail.tagger} className="size-6 shrink-0 rounded-full" />
                <span className="font-medium">{detail.tagger.name}</span>
                {detail.tagged_at && (
                  <span className="text-muted-foreground" title={formatAbsoluteTime(detail.tagged_at)}>
                    {formatRelativeTime(detail.tagged_at)}
                  </span>
                )}
              </dd>
            </>
          )}
          <dt className="text-muted-foreground">Object</dt>
          <dd className="font-mono break-all">
            {/* A commit target links to the commit page; anything else
                (tree/blob/nested tag) links to the by-oid object page —
                cgit's `cgit_object_link()` parity (docs/DECISIONS.md #53). */}
            <a
              className="underline"
              href={
                detail.object.type === "commit"
                  ? commitHref(resolvedRepo, detail.object.sha)
                  : objectHref(resolvedRepo, detail.object.sha)
              }
            >
              {detail.object.sha}
            </a>
            <span className="text-muted-foreground not-italic"> ({detail.object.type})</span>
          </dd>
          {showCommitRow && (
            <>
              <dt className="text-muted-foreground">Commit</dt>
              <dd className="font-mono break-all">
                <a className="underline" href={commitHref(resolvedRepo, detail.target as string)}>
                  {detail.target}
                </a>
              </dd>
            </>
          )}
        </dl>
        <div className="text-muted-foreground flex flex-wrap gap-x-4 gap-y-1 text-sm">
          {canBrowse && (
            <>
              <a
                className="hover:text-foreground hover:underline"
                href={treeHref(resolvedRepo, "", detail.name)}
                aria-label="Browse the tree at this tag"
              >
                Tree
              </a>
              <a className="hover:text-foreground hover:underline" href={logHref(resolvedRepo, { ref: detail.name })}>
                Log
              </a>
              {ARCHIVE_FORMATS.map((format) => (
                <a
                  key={format}
                  className="hover:text-foreground hover:underline"
                  href={archiveUrl(resolvedRepo, detail.name, format)}
                >
                  {format}
                </a>
              ))}
            </>
          )}
        </div>
      </div>

      {detail.message && (
        <pre className="border-border bg-muted/30 overflow-x-auto rounded-md border p-3 text-sm whitespace-pre-wrap">
          {linkify(detail.message, { repo: resolvedRepo })}
        </pre>
      )}
    </div>
  );
}
