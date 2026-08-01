import { useEffect, useState } from "react";

import { ApiError } from "@/lib/api/client";
import { encodeSegment } from "@/lib/api/path";
import { getCommit, getCommitDiff } from "@/lib/api/repos";
import type { CommitAuthor, CommitDetail, CommitDiff, DiffStatus, FileDiff, Line } from "@/lib/api/schemas";
import { linkify } from "@/lib/format/linkify";
import { formatAbsoluteTime, formatRelativeTime } from "@/lib/format/time";
import { commitShaFromPathname, repoFromPathname } from "@/lib/repo-param";
import AuthorAvatar from "@/components/repo/AuthorAvatar";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

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

const STATUS_LABEL: Record<DiffStatus, string> = {
  added: "added",
  deleted: "deleted",
  modified: "modified",
  renamed: "renamed",
  copied: "copied",
  typechange: "typechange",
};

const STATUS_CLASS: Record<DiffStatus, string> = {
  added: "text-green-600 dark:text-green-400",
  deleted: "text-destructive",
  modified: "text-amber-600 dark:text-amber-400",
  renamed: "text-muted-foreground",
  copied: "text-muted-foreground",
  typechange: "text-muted-foreground",
};

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

function DiffLineRow({ line }: { line: Line }) {
  const background = line.origin === "+" ? "bg-green-500/10" : line.origin === "-" ? "bg-red-500/10" : undefined;
  return (
    <tr className={background}>
      <td className="text-muted-foreground w-10 shrink-0 px-2 text-right font-mono select-none">
        {line.old_lineno ?? ""}
      </td>
      <td className="text-muted-foreground w-10 shrink-0 px-2 text-right font-mono select-none">
        {line.new_lineno ?? ""}
      </td>
      <td className="w-4 shrink-0 text-center font-mono select-none">{line.origin}</td>
      <td className="font-mono whitespace-pre">{line.content}</td>
    </tr>
  );
}

function FileDiffView({ file }: { file: FileDiff }) {
  const pathLabel = file.old_path && file.old_path !== file.path ? `${file.old_path} → ${file.path}` : file.path;

  return (
    <details open className="border-border rounded-md border">
      <summary className="bg-muted/50 cursor-pointer px-3 py-2 font-mono text-sm">
        <span className={STATUS_CLASS[file.status]}>{STATUS_LABEL[file.status]}</span> {pathLabel}{" "}
        <span className="text-muted-foreground">
          +{file.additions} −{file.deletions}
        </span>
      </summary>
      <div className="overflow-x-auto">
        {file.binary ? (
          <p className="text-muted-foreground px-3 py-2 text-sm">Binary file not shown.</p>
        ) : (
          <>
            {file.hunks.map((hunk) => (
              <table key={hunk.header} className="w-full border-collapse text-sm">
                <tbody>
                  <tr>
                    <td colSpan={4} className="text-muted-foreground bg-muted/30 px-2 py-1 font-mono">
                      {hunk.header}
                    </td>
                  </tr>
                  {hunk.lines.map((line, index) => (
                    // eslint-disable-next-line @eslint-react/no-array-index-key -- lines have no stable identity
                    <DiffLineRow key={index} line={line} />
                  ))}
                </tbody>
              </table>
            ))}
            {file.truncated && (
              <p className="text-muted-foreground px-3 py-2 text-sm">Diff truncated (1000 lines max).</p>
            )}
          </>
        )}
      </div>
    </details>
  );
}

export default function CommitView({ repo, sha }: CommitViewProps) {
  const resolvedRepo = repo ?? repoFromPathname(window.location.pathname);
  const resolvedSha = sha ?? commitShaFromPathname(window.location.pathname);
  const [state, setState] = useState<State>({ status: "loading" });

  useEffect(() => {
    let cancelled = false;

    Promise.all([getCommit(resolvedRepo, resolvedSha), getCommitDiff(resolvedRepo, resolvedSha)])
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
  }, [resolvedRepo, resolvedSha]);

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
  const commitHref = (parentSha: string) => `/${encodeSegment(resolvedRepo)}/commit/${encodeSegment(parentSha)}`;

  return (
    <div className="space-y-6">
      <div className="space-y-3">
        <h2 className="text-lg font-medium break-words">{detail.summary ?? "(no commit message)"}</h2>
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
                  <a key={parent} className="underline" href={commitHref(parent)}>
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
      </div>

      {detail.message && (
        <pre className="border-border bg-muted/30 overflow-x-auto rounded-md border p-3 text-sm whitespace-pre-wrap">
          {linkify(detail.message, { repo: resolvedRepo })}
        </pre>
      )}

      <div className="space-y-2">
        <h3 className="text-sm font-medium">
          {detail.diffstat.files_changed} file{detail.diffstat.files_changed === 1 ? "" : "s"} changed, +
          {detail.diffstat.total_additions} −{detail.diffstat.total_deletions}
        </h3>
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Path</TableHead>
              <TableHead>Status</TableHead>
              <TableHead>Changes</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {detail.diffstat.files.map((file) => (
              <TableRow key={file.path}>
                <TableCell className="font-mono">
                  {file.old_path && file.old_path !== file.path ? `${file.old_path} → ${file.path}` : file.path}
                </TableCell>
                <TableCell className={STATUS_CLASS[file.status]}>{STATUS_LABEL[file.status]}</TableCell>
                <TableCell className="text-muted-foreground font-mono">
                  {file.binary ? "binary" : `+${file.additions} −${file.deletions}`}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>

      <div className="space-y-3">
        {diff.truncated && (
          <p className="text-muted-foreground text-sm">
            Some files were omitted (300 files max) — see the table above for the full file list.
          </p>
        )}
        {diff.files.map((file) => (
          <FileDiffView key={file.path} file={file} />
        ))}
      </div>
    </div>
  );
}
