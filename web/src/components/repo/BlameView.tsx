import { useEffect, useState } from "react";

import { ApiError } from "@/lib/api/client";
import { encodeSegment } from "@/lib/api/path";
import { getBlame, getBlob, rawUrl } from "@/lib/api/repos";
import type { BlameInfo, BlameRange, BlobInfo } from "@/lib/api/schemas";
import { formatAbsoluteTime, formatRelativeTime } from "@/lib/format/time";
import { filePathFromPathname, paramFromSearch, repoFromPathname } from "@/lib/repo-param";
import { blameHref, blobHref, logHref } from "@/lib/repo-href";
import CodeBlock, { type GutterCell } from "@/components/repo/CodeBlock";
import PathBreadcrumbs from "@/components/repo/PathBreadcrumbs";
import { Skeleton } from "@/components/ui/skeleton";

type State =
  { status: "loading" } | { status: "error"; error: ApiError } | { status: "data"; blame: BlameInfo; blob: BlobInfo };

interface BlameViewProps {
  /**
   * Omitted by the prerendered `/{repo}/blame` shell, which is built under a
   * placeholder param (`lib/shell.ts`) — the real repo/path are read from
   * the URL in the browser. `client:only` guarantees this default is only
   * ever evaluated there.
   */
  repo?: string;
  path?: string;
  ref?: string;
}

/** Also rendered statically into the page shell as the island's
 *  `slot="fallback"`, so the prerendered HTML is not blank. */
export function BlameViewSkeleton() {
  return (
    <div className="space-y-2" aria-busy="true">
      <Skeleton className="h-5 w-1/2" />
      <Skeleton className="h-64 w-full" />
    </div>
  );
}

/** One gutter cell's content — short sha link, relative time, author name
 *  (cgit's compact blame gutter). `authored_at`/`summary` can be `null` for
 *  an unrepresentable timestamp or non-UTF-8 commit message (docs/API.md). */
function RangeCell({ repo, range }: { repo: string; range: BlameRange }) {
  const commitHref = `/${encodeSegment(repo)}/commit/${encodeSegment(range.sha)}`;
  const title = [
    range.summary ?? "(no commit message)",
    range.authored_at ? formatAbsoluteTime(range.authored_at) : null,
  ]
    .filter(Boolean)
    .join("\n");

  return (
    <div className="flex flex-col text-xs" title={title}>
      <span className="flex items-center gap-1">
        <a className="hover:text-foreground font-mono underline" href={commitHref}>
          {range.sha.slice(0, 7)}
        </a>
        {/* Whole-file renames are tracked (docs/API.md); this marker links
         *  back to the file's blame under its earlier path, at the commit
         *  that still had it. */}
        {range.orig_path && (
          <a
            className="text-muted-foreground hover:text-foreground"
            href={blameHref(repo, range.orig_path, range.sha)}
            title={`Renamed from ${range.orig_path}`}
            aria-label={`Renamed from ${range.orig_path}`}
          >
            ↩
          </a>
        )}
      </span>
      <span>{range.authored_at ? formatRelativeTime(range.authored_at) : "unknown time"}</span>
      <span className="truncate">{range.author.name}</span>
    </div>
  );
}

/** Expands `ranges` (1-based, gap-free, covering `lines` lines) into a
 *  0-based per-display-line gutter for `CodeBlock` — one `GutterCell` at
 *  each range's first line (`rowSpan` merges it over the rest), `null` for
 *  the lines it covers. Any display lines past `lines` (e.g. the empty
 *  trailing entry `content.split("\n")` produces for a final newline) get a
 *  blank one-row cell so the column stays aligned. */
function buildGutter(
  repo: string,
  ranges: BlameRange[],
  lines: number,
  displayLineCount: number,
): (GutterCell | null)[] {
  const cells: (GutterCell | null)[] = new Array(displayLineCount).fill(null);
  for (const range of ranges) {
    const start = range.start_line - 1;
    cells[start] = { node: <RangeCell repo={repo} range={range} />, rowSpan: range.line_count };
    for (let i = start + 1; i < start + range.line_count; i++) {
      cells[i] = null;
    }
  }
  for (let i = lines; i < displayLineCount; i++) {
    cells[i] = { node: null, rowSpan: 1 };
  }
  return cells;
}

export default function BlameView({ repo, path, ref: refParam }: BlameViewProps) {
  const resolvedRepo = repo ?? repoFromPathname(window.location.pathname);
  const resolvedPath = path ?? filePathFromPathname(window.location.pathname);
  const resolvedRef = refParam ?? paramFromSearch("ref", window.location.search);
  const [state, setState] = useState<State>({ status: "loading" });

  useEffect(() => {
    let cancelled = false;

    // The blame response carries no file content (docs/API.md), so the blob
    // is fetched alongside it — same parallel-fetch shape as `CommitView`'s
    // detail + diff.
    Promise.all([getBlame(resolvedRepo, resolvedRef, resolvedPath), getBlob(resolvedRepo, resolvedRef, resolvedPath)])
      .then(([blame, blob]) => {
        if (!cancelled) {
          setState({ status: "data", blame, blob });
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
  }, [resolvedRepo, resolvedRef, resolvedPath]);

  if (state.status === "loading") {
    return <BlameViewSkeleton />;
  }

  if (state.status === "error") {
    const message =
      state.error.code === "path_not_found"
        ? "Path not found."
        : state.error.code === "repo_not_found"
          ? "Repository not found."
          : state.error.message;
    return (
      <p role="alert" className="text-destructive text-sm">
        Failed to load blame: {message}
      </p>
    );
  }

  const { blame, blob } = state;
  const raw = rawUrl(resolvedRepo, resolvedRef, resolvedPath);
  const historyHref = logHref(resolvedRepo, { path: blame.path, ref: resolvedRef });

  return (
    <div className="space-y-4">
      <PathBreadcrumbs repo={resolvedRepo} path={resolvedPath} ref={resolvedRef} />

      <div className="text-muted-foreground flex flex-wrap items-center gap-x-4 gap-y-1 text-sm">
        <a className="hover:text-foreground hover:underline" href={blobHref(resolvedRepo, resolvedPath, resolvedRef)}>
          View file
        </a>
        <a className="hover:text-foreground hover:underline" href={raw}>
          Raw
        </a>
        <a className="hover:text-foreground hover:underline" href={historyHref}>
          History
        </a>
      </div>

      {blame.binary ? (
        <p className="text-muted-foreground text-sm">Binary file — blame not shown.</p>
      ) : blame.too_large ? (
        <p className="text-muted-foreground text-sm">File too large — blame not shown.</p>
      ) : blame.ranges.length === 0 ? (
        <p className="text-muted-foreground text-sm">This file is empty.</p>
      ) : (
        <CodeBlock
          content={blob.content ?? ""}
          path={blob.path}
          gutter={buildGutter(resolvedRepo, blame.ranges, blame.lines, (blob.content ?? "").split("\n").length)}
        />
      )}
    </div>
  );
}
