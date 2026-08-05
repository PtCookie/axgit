import { useEffect, useState } from "react";

import { ApiError } from "@/lib/api/client";
import { encodeSegment } from "@/lib/api/path";
import { searchRepo } from "@/lib/api/repos";
import type { SearchKind, SearchResults } from "@/lib/api/schemas";
import { formatAbsoluteTime, formatRelativeTime } from "@/lib/format/time";
import { paramFromSearch, repoFromPathname } from "@/lib/repo-param";
import { blobHref, blobLineHref } from "@/lib/repo-href";
import AuthorAvatar from "@/components/repo/AuthorAvatar";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";

// No query yet: the form is shown but nothing has been searched.
type State =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "error"; error: ApiError }
  | { status: "data"; results: SearchResults };

interface SearchViewProps {
  /**
   * Omitted by the prerendered `/{repo}/search` shell, which is built under
   * a placeholder param (`lib/shell.ts`) — the real name is read from the
   * URL in the browser. `client:only` guarantees this default is only ever
   * evaluated there.
   */
  repo?: string;
  /** `q`/`type`/`ref` follow the same "prop overrides, `location` is the
   *  default source" pattern as `repo` — see `CommitLog`/`TreeView`. */
  q?: string;
  type?: string;
  ref?: string;
}

const TYPE_OPTIONS: { value: SearchKind; label: string }[] = [
  { value: "content", label: "Content" },
  { value: "path", label: "File path" },
  { value: "message", label: "Commit message" },
  { value: "author", label: "Author" },
  { value: "committer", label: "Committer" },
  { value: "range", label: "Revision range" },
];

/** Every valid `type` value, derived from `TYPE_OPTIONS` so the list lives
 *  in one place. */
const SEARCH_KINDS: readonly SearchKind[] = TYPE_OPTIONS.map((option) => option.value);

/** `type`s whose results are commits (`SearchResults.commits`), rendered as
 *  commit rows — everything else (`content`/`path`) renders as file rows. */
const COMMIT_KINDS: readonly SearchKind[] = ["message", "author", "committer", "range"];

function isSearchKind(value: string | undefined): value is SearchKind {
  return value !== undefined && (SEARCH_KINDS as readonly string[]).includes(value);
}

/** Also rendered statically into the page shell as the island's
 *  `slot="fallback"`, so the prerendered HTML is not blank. */
export function SearchViewSkeleton() {
  return (
    <div className="space-y-4" aria-busy="true">
      <Skeleton className="h-9 w-full" />
      <Skeleton className="h-8 w-full" />
      <Skeleton className="h-8 w-full" />
    </div>
  );
}

export default function SearchView({ repo, q: qParam, type: typeParam, ref: refParam }: SearchViewProps) {
  const resolvedRepo = repo ?? repoFromPathname(window.location.pathname);
  const resolvedQuery = qParam ?? paramFromSearch("q", window.location.search) ?? "";
  const rawType = typeParam ?? paramFromSearch("type", window.location.search);
  const resolvedType: SearchKind = isSearchKind(rawType) ? rawType : "content";
  const resolvedRef = refParam ?? paramFromSearch("ref", window.location.search);
  // Lazily derived from the initial query so a fresh mount never flashes
  // "idle" before "loading" — matches `CommitLog`'s note that this effect's
  // dependency array only matters for tests changing props on a live
  // instance; production always remounts fresh on a new URL (DECISIONS.md
  // #24), so staying on the previous status until the new one resolves is
  // preferable here too.
  const [state, setState] = useState<State>(() => (resolvedQuery ? { status: "loading" } : { status: "idle" }));

  useEffect(() => {
    if (!resolvedQuery) {
      return;
    }
    let cancelled = false;

    searchRepo(resolvedRepo, { q: resolvedQuery, type: resolvedType, ref: resolvedRef })
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
  }, [resolvedRepo, resolvedQuery, resolvedType, resolvedRef]);

  return (
    <div className="space-y-4">
      {/* Plain `method="get"` form: `<ClientRouter />` (DECISIONS.md #24)
          intercepts same-origin GET submits for a client-side transition,
          and a full page load works identically if it doesn't — no
          client-side router state is kept (DECISIONS.md #18). */}
      <form method="get" className="flex flex-wrap items-center gap-2">
        <Input
          type="search"
          name="q"
          defaultValue={resolvedQuery}
          placeholder="Search this repository…"
          aria-label="Search query"
          className="min-w-48 flex-1"
        />
        <select
          name="type"
          defaultValue={resolvedType}
          aria-label="Search type"
          className="border-input bg-input/50 h-9 rounded-3xl border px-3 text-sm"
        >
          {TYPE_OPTIONS.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
        {resolvedRef && <input type="hidden" name="ref" value={resolvedRef} />}
        <button
          type="submit"
          className="border-input bg-input/50 hover:bg-accent h-9 rounded-3xl border px-4 text-sm font-medium"
        >
          Search
        </button>
      </form>

      {resolvedType === "range" && (
        <p className="text-muted-foreground text-sm">
          Revision range: a rev-list expression, not a text query — e.g. <code>v1.0..main</code> or{" "}
          <code>main ^next</code>.
        </p>
      )}

      {state.status === "idle" && <p className="text-muted-foreground text-sm">Enter a search query above.</p>}

      {state.status === "loading" && (
        <div className="space-y-2" aria-busy="true">
          <Skeleton className="h-8 w-full" />
          <Skeleton className="h-8 w-full" />
        </div>
      )}

      {state.status === "error" && (
        <p role="alert" className="text-destructive text-sm">
          Failed to search: {state.error.status === 404 ? "Repository not found." : state.error.message}
        </p>
      )}

      {state.status === "data" && <SearchResultsList repo={resolvedRepo} ref={resolvedRef} results={state.results} />}
    </div>
  );
}

interface SearchResultsListProps {
  repo: string;
  ref: string | undefined;
  results: SearchResults;
}

function SearchResultsList({ repo, ref, results }: SearchResultsListProps) {
  const hasResults = results.files.length > 0 || results.commits.length > 0;

  return (
    <div className="space-y-4">
      {results.truncated && (
        <p role="status" className="text-muted-foreground text-sm">
          Showing a partial result — narrow the query for a complete match set.
        </p>
      )}
      {!hasResults ? (
        <p className="text-muted-foreground text-sm">No matches found.</p>
      ) : COMMIT_KINDS.includes(results.type) ? (
        <ul className="divide-border divide-y">
          {results.commits.map((commit) => (
            <li key={commit.sha} className="flex flex-wrap items-center gap-2 py-2">
              <AuthorAvatar author={commit.author} className="size-6 shrink-0 rounded-full" />
              <span className="text-muted-foreground text-sm">{commit.author.name}</span>
              <a
                className="font-medium hover:underline"
                href={`/${encodeSegment(repo)}/commit/${encodeSegment(commit.sha)}`}
              >
                {commit.summary ?? "(no commit message)"}
              </a>
              <span className="text-muted-foreground font-mono text-sm">{commit.sha.slice(0, 12)}</span>
              {commit.authored_at && (
                <span className="text-muted-foreground text-sm" title={formatAbsoluteTime(commit.authored_at)}>
                  {formatRelativeTime(commit.authored_at)}
                </span>
              )}
            </li>
          ))}
        </ul>
      ) : (
        <ul className="space-y-4">
          {results.files.map((file) => (
            <li key={file.path}>
              <a
                className="font-mono text-sm font-medium hover:underline"
                href={
                  file.lines.length > 0
                    ? blobLineHref(repo, file.path, ref, file.lines[0].line)
                    : blobHref(repo, file.path, ref)
                }
              >
                {file.path}
              </a>
              {file.lines.length > 0 && (
                <ul className="mt-1 space-y-1">
                  {file.lines.map((line) => (
                    <li key={line.line}>
                      <a
                        className="text-muted-foreground hover:text-foreground block truncate font-mono text-xs"
                        href={blobLineHref(repo, file.path, ref, line.line)}
                      >
                        <span className="mr-2 select-none">{line.line}:</span>
                        {line.text}
                      </a>
                    </li>
                  ))}
                </ul>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
