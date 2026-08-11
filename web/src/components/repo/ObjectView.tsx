import { useEffect, useState } from "react";

import { ApiError } from "@/lib/api/client";
import { getObject, objectRawUrl } from "@/lib/api/repos";
import type { ObjectDetail } from "@/lib/api/schemas";
import { formatMode } from "@/lib/format/mode";
import { formatSize } from "@/lib/format/size";
import { linkify } from "@/lib/format/linkify";
import { formatAbsoluteTime, formatRelativeTime } from "@/lib/format/time";
import { commitHref, objectHref } from "@/lib/repo-href";
import { objectOidFromPathname, repoFromPathname } from "@/lib/repo-param";
import AuthorAvatar from "@/components/repo/AuthorAvatar";
import CodeBlock from "@/components/repo/CodeBlock";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

type State = { status: "loading" } | { status: "error"; error: ApiError } | { status: "data"; detail: ObjectDetail };

interface ObjectViewProps {
  /**
   * Omitted by the prerendered `/{repo}/object` shell, which is built under a
   * placeholder param (`lib/shell.ts`) — the real repo/oid are read from the
   * URL in the browser. `client:only` guarantees this default is only ever
   * evaluated there.
   */
  repo?: string;
  oid?: string;
}

/** Also rendered statically into the page shell as the island's
 *  `slot="fallback"`, so the prerendered HTML is not blank. */
export function ObjectViewSkeleton() {
  return (
    <div className="space-y-2" aria-busy="true">
      <Skeleton className="h-6 w-2/3" />
      <Skeleton className="h-24 w-full" />
    </div>
  );
}

/** Links a dereferenced object's sha onward: `commitHref` for a commit,
 *  `objectHref` (this page, one hop further) for everything else — shared
 *  by the tag payload's `object` row here and by `RefsView`/`TagView`'s
 *  non-commit tag targets (docs/DECISIONS.md #53). */
function dereferenceHref(repo: string, sha: string, type: string): string {
  return type === "commit" ? commitHref(repo, sha) : objectHref(repo, sha);
}

export default function ObjectView({ repo, oid }: ObjectViewProps) {
  const resolvedRepo = repo ?? repoFromPathname(window.location.pathname);
  const resolvedOid = oid ?? objectOidFromPathname(window.location.pathname);
  const [state, setState] = useState<State>({ status: "loading" });

  useEffect(() => {
    let cancelled = false;

    getObject(resolvedRepo, resolvedOid)
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
  }, [resolvedRepo, resolvedOid]);

  if (state.status === "loading") {
    return <ObjectViewSkeleton />;
  }

  if (state.status === "error") {
    const message =
      state.error.code === "object_not_found"
        ? "Object not found."
        : state.error.code === "repo_not_found"
          ? "Repository not found."
          : state.error.message;
    return (
      <p role="alert" className="text-destructive text-sm">
        Failed to load object: {message}
      </p>
    );
  }

  const { detail } = state;
  const raw = objectRawUrl(resolvedRepo, detail.sha);

  return (
    <div className="space-y-4">
      <h2 className="font-mono text-lg break-all">
        {detail.sha}
        <span className="text-muted-foreground ml-2 font-sans text-sm not-italic">({detail.type})</span>
      </h2>

      {detail.type === "commit" && (
        <p className="text-sm">
          <a className="underline" href={commitHref(resolvedRepo, detail.sha)}>
            View commit
          </a>
        </p>
      )}

      {detail.tree && (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="w-px">Mode</TableHead>
              <TableHead>Name</TableHead>
              <TableHead>Size</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {detail.tree.entries.length === 0 ? (
              <TableRow>
                <TableCell colSpan={3} className="text-muted-foreground">
                  This tree is empty.
                </TableCell>
              </TableRow>
            ) : (
              detail.tree.entries.map((entry) => (
                <TableRow key={entry.name}>
                  <TableCell className="text-muted-foreground w-px font-mono" title={entry.mode}>
                    {formatMode(entry.mode)}
                  </TableCell>
                  <TableCell className="font-medium">
                    {/* A gitlink entry's sha is a commit in another
                        repository — nothing in this one to link to, same as
                        `TreeView.tsx`'s submodule row. */}
                    {entry.type === "commit" ? (
                      <span title="submodule">{entry.name}</span>
                    ) : (
                      <a className="hover:underline" href={objectHref(resolvedRepo, entry.sha)}>
                        {entry.type === "tree" ? `${entry.name}/` : entry.name}
                      </a>
                    )}
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {entry.size === null ? "—" : formatSize(entry.size)}
                  </TableCell>
                </TableRow>
              ))
            )}
          </TableBody>
        </Table>
      )}

      {detail.blob && (
        <div className="space-y-4">
          <div className="text-muted-foreground flex flex-wrap items-center gap-x-4 gap-y-1 text-sm">
            <span>{formatSize(detail.blob.size)}</span>
            <a className="hover:text-foreground hover:underline" href={raw}>
              Raw
            </a>
          </div>
          {detail.blob.binary ? (
            <p className="text-muted-foreground text-sm">
              Binary file not shown —{" "}
              <a className="underline" href={raw}>
                view raw
              </a>
              .
            </p>
          ) : detail.blob.too_large ? (
            <p className="text-muted-foreground text-sm">
              File too large to display —{" "}
              <a className="underline" href={raw}>
                view raw
              </a>
              .
            </p>
          ) : (
            // No filename behind an oid, so `path=""` — Shiki has nothing to
            // pick a language from and renders plain, same as an
            // unrecognized extension elsewhere.
            <CodeBlock content={detail.blob.content ?? ""} path="" />
          )}
        </div>
      )}

      {detail.tag && (
        <div className="space-y-6">
          <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm">
            {detail.tag.tagger !== null && (
              <>
                <dt className="text-muted-foreground">Tagger</dt>
                <dd className="flex items-center gap-2">
                  <AuthorAvatar author={detail.tag.tagger} className="size-6 shrink-0 rounded-full" />
                  <span className="font-medium">{detail.tag.tagger.name}</span>
                  {detail.tag.tagged_at && (
                    <span className="text-muted-foreground" title={formatAbsoluteTime(detail.tag.tagged_at)}>
                      {formatRelativeTime(detail.tag.tagged_at)}
                    </span>
                  )}
                </dd>
              </>
            )}
            <dt className="text-muted-foreground">Object</dt>
            <dd className="font-mono break-all">
              <a
                className="underline"
                href={dereferenceHref(resolvedRepo, detail.tag.object.sha, detail.tag.object.type)}
              >
                {detail.tag.object.sha}
              </a>
              <span className="text-muted-foreground not-italic"> ({detail.tag.object.type})</span>
            </dd>
            {detail.tag.target !== null && detail.tag.target !== detail.tag.object.sha && (
              <>
                <dt className="text-muted-foreground">Commit</dt>
                <dd className="font-mono break-all">
                  <a className="underline" href={commitHref(resolvedRepo, detail.tag.target)}>
                    {detail.tag.target}
                  </a>
                </dd>
              </>
            )}
          </dl>
          {detail.tag.message && (
            <pre className="border-border bg-muted/30 overflow-x-auto rounded-md border p-3 text-sm whitespace-pre-wrap">
              {linkify(detail.tag.message, { repo: resolvedRepo })}
            </pre>
          )}
        </div>
      )}
    </div>
  );
}
