import { useEffect, useRef, useState } from "react";

import { ApiError } from "@/lib/api/client";
import { comparePatchUrl, compareRawDiffUrl, getDiff, getRefs } from "@/lib/api/repos";
import type { RefsInfo, RevDiff } from "@/lib/api/schemas";
import { useDefaultBranch } from "@/lib/default-branch";
import { diffApiParams, type DiffViewMode, parseDiffOptions } from "@/lib/diff-options";
import { compareHref } from "@/lib/repo-href";
import { paramFromSearch, repoFromPathname } from "@/lib/repo-param";
import DiffFileList from "@/components/repo/diff/DiffFileList";
import DiffOptionsBar from "@/components/repo/diff/DiffOptionsBar";
import DiffStatTable from "@/components/repo/diff/DiffStatTable";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";

const NO_REFS: RefsInfo = { branches: [], tags: [] };
const REVISIONS_LIST_ID = "axgit-diff-revisions";

// No comparison requested yet: the picker is shown but nothing was fetched.
type State =
  { status: "idle" } | { status: "loading" } | { status: "error"; error: ApiError } | { status: "data"; diff: RevDiff };

interface DiffViewProps {
  /**
   * Omitted by the prerendered `/{repo}/diff` shell, which is built under a
   * placeholder param (`lib/shell.ts`) — the real name is read from the URL
   * in the browser. `client:only` guarantees this default is only ever
   * evaluated there.
   */
  repo?: string;
  /** `from`/`to`/`path`/`view`/`context`/`ignorews` follow the same "prop
   *  overrides, `location` is the default source" pattern as `repo` — see
   *  `CommitView`/`SearchView`. */
  from?: string;
  to?: string;
  path?: string;
  view?: DiffViewMode;
  context?: number;
  ignorews?: boolean;
}

/** Also rendered statically into the page shell as the island's
 *  `slot="fallback"`, so the prerendered HTML is not blank. */
export function DiffViewSkeleton() {
  return (
    <div className="space-y-4" aria-busy="true">
      <Skeleton className="h-9 w-full" />
      <Skeleton className="h-24 w-full" />
      <Skeleton className="h-48 w-full" />
    </div>
  );
}

export default function DiffView({
  repo,
  from: fromProp,
  to: toProp,
  path: pathProp,
  view: viewProp,
  context: contextProp,
  ignorews: ignorewsProp,
}: DiffViewProps) {
  const resolvedRepo = repo ?? repoFromPathname(window.location.pathname);
  const resolvedFrom = fromProp ?? paramFromSearch("from", window.location.search);
  const resolvedTo = toProp ?? paramFromSearch("to", window.location.search);
  const resolvedPath = pathProp ?? paramFromSearch("path", window.location.search);
  const urlOptions = parseDiffOptions(window.location.search);
  const resolvedView = viewProp ?? urlOptions.view;
  const resolvedContext = contextProp ?? urlOptions.context;
  const resolvedIgnorews = ignorewsProp ?? urlOptions.ignorews;
  const options = { view: resolvedView, context: resolvedContext, ignorews: resolvedIgnorews };
  const isStatOnly = resolvedView === "stat";

  // Neither side given: nothing to compare yet, same "idle" shape as
  // SearchView before a query is entered.
  const hasComparison = resolvedFrom !== undefined || resolvedTo !== undefined;
  const [state, setState] = useState<State>(() => (hasComparison ? { status: "loading" } : { status: "idle" }));
  const [refs, setRefs] = useState<RefsInfo>(NO_REFS);

  // Prefills the idle picker's `to` field only — never fetched once a
  // comparison is already in progress. Decoration, not required data: see
  // `useDefaultBranch`'s own doc comment.
  const defaultBranch = useDefaultBranch(resolvedRepo, !hasComparison);
  const toInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    // Imperative, not `defaultValue`/`key`: the form is deliberately
    // uncontrolled (plain `method="get"`, works without JS — #39), and React
    // ignores a changed `defaultValue` on re-render, so an async value would
    // never reach the input that way. A `key` remount would show it, but
    // would also wipe out anything the visitor already typed while the fetch
    // was in flight and steal focus. Only fill an untouched field — checked
    // against the installed `@base-ui/react` Input source: it forwards this
    // ref straight onto the native `<input>` and keeps no React state for an
    // uncontrolled value, so reading/writing `.value` here is safe.
    if (defaultBranch && toInputRef.current && toInputRef.current.value === "") {
      toInputRef.current.value = defaultBranch;
    }
  }, [defaultBranch]);

  useEffect(() => {
    // Populates the revision datalist only — decoration, never blocks the
    // diff itself, same "silent on failure" rule as `useCommitRefs`.
    let cancelled = false;
    getRefs(resolvedRepo)
      .then((info) => {
        if (!cancelled) setRefs(info);
      })
      .catch(() => {
        /* decoration only */
      });
    return () => {
      cancelled = true;
    };
  }, [resolvedRepo]);

  useEffect(() => {
    if (!hasComparison) {
      return;
    }
    // `hasComparison`/`resolvedFrom`/etc. never actually change on an
    // already-mounted instance in production — the island remounts fresh on
    // every navigation (DECISIONS.md #24) — so there's no separate
    // "loading" transition to fire here; the lazy `useState` initializer
    // above already covers the real (first-render) case. This only matters
    // for tests that change props on a live instance, where staying on the
    // previous status until the new one resolves beats a loading flash
    // (same convention as `CommitLog`/`SearchView`).
    let cancelled = false;

    // Stat-only mode (`?stat=1`) skips hunk rendering server-side, so
    // `context`/`ignorews` are dropped from the request entirely — neither
    // affects `diffstat`, and omitting them normalizes every stat-only
    // comparison onto one cache entry regardless of what unified/split had
    // last been set to.
    getDiff(resolvedRepo, {
      from: resolvedFrom,
      to: resolvedTo,
      path: resolvedPath,
      ...(isStatOnly ? { stat: 1 } : diffApiParams(options)),
    })
      .then((diff) => {
        if (!cancelled) {
          setState({ status: "data", diff });
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
  }, [resolvedRepo, resolvedFrom, resolvedTo, resolvedPath, resolvedContext, resolvedIgnorews, isStatOnly]);

  const revisionNames = [...refs.branches.map((branch) => branch.name), ...refs.tags.map((tag) => tag.name)];

  const rawDiffParams = { from: resolvedFrom, to: resolvedTo, path: resolvedPath, ...diffApiParams(options) };
  const patchParams = { from: resolvedFrom, to: resolvedTo, path: resolvedPath };
  // Carried as hidden inputs so the options-bar "Apply" button doesn't drop
  // the current comparison.
  const extraFormParams: Record<string, string> = {};
  if (resolvedFrom) extraFormParams.from = resolvedFrom;
  if (resolvedTo) extraFormParams.to = resolvedTo;
  if (resolvedPath) extraFormParams.path = resolvedPath;

  return (
    <div className="space-y-6">
      {/* Plain `method="get"` form: `<ClientRouter />` (DECISIONS.md #24)
          intercepts same-origin GET submits for a client-side transition,
          and a full page load works identically if it doesn't. */}
      <form method="get" className="flex flex-wrap items-end gap-2">
        <label className="flex flex-col gap-1 text-sm">
          <span className="text-muted-foreground">From</span>
          <Input
            name="from"
            list={REVISIONS_LIST_ID}
            defaultValue={resolvedFrom ?? ""}
            placeholder="e.g. main"
            aria-label="Compare from revision"
            className="w-44"
          />
        </label>
        <label className="flex flex-col gap-1 text-sm">
          <span className="text-muted-foreground">To</span>
          <Input
            ref={toInputRef}
            name="to"
            list={REVISIONS_LIST_ID}
            defaultValue={resolvedTo ?? ""}
            placeholder="e.g. HEAD"
            aria-label="Compare to revision"
            className="w-44"
          />
        </label>
        <datalist id={REVISIONS_LIST_ID}>
          {revisionNames.map((name) => (
            <option key={name} value={name} />
          ))}
        </datalist>
        {resolvedPath && <input type="hidden" name="path" value={resolvedPath} />}
        <button
          type="submit"
          className="border-input bg-input/50 hover:bg-accent h-9 rounded-3xl border px-4 text-sm font-medium"
        >
          Compare
        </button>
        {hasComparison && (
          <a
            href={compareHref(resolvedRepo, {
              from: resolvedTo,
              to: resolvedFrom,
              path: resolvedPath,
              ...options,
            })}
            aria-label="Swap revisions"
            className="border-input bg-input/50 hover:bg-accent flex h-9 items-center rounded-3xl border px-4 text-sm font-medium"
          >
            Swap
          </a>
        )}
      </form>

      {state.status === "idle" && (
        <p className="text-muted-foreground text-sm">
          {defaultBranch ? `Pick a revision to compare against ${defaultBranch}.` : "Pick two revisions to compare."}
        </p>
      )}

      {state.status === "loading" && <DiffViewSkeleton />}

      {state.status === "error" && (
        <p role="alert" className="text-destructive text-sm">
          Failed to load diff: {state.error.status === 404 ? "Revision not found." : state.error.message}
        </p>
      )}

      {state.status === "data" && (
        <>
          <h2 className="text-lg font-medium break-all">
            Comparing{" "}
            <span className="font-mono text-base">
              {state.diff.from ? state.diff.from.slice(0, 12) : "(empty tree)"}
            </span>{" "}
            → <span className="font-mono text-base">{state.diff.to.slice(0, 12)}</span>
          </h2>

          <div className="text-muted-foreground flex flex-wrap gap-x-4 gap-y-1 text-sm">
            <a className="hover:text-foreground hover:underline" href={compareRawDiffUrl(resolvedRepo, rawDiffParams)}>
              Raw diff
            </a>
            <a className="hover:text-foreground hover:underline" href={comparePatchUrl(resolvedRepo, patchParams)}>
              Patch
            </a>
          </div>

          {resolvedPath && (
            <p className="text-muted-foreground text-sm">
              Showing only <span className="text-foreground font-mono">{resolvedPath}</span> —{" "}
              <a
                className="hover:text-foreground underline"
                href={compareHref(resolvedRepo, { from: resolvedFrom, to: resolvedTo, ...options })}
              >
                Show all files
              </a>
            </p>
          )}

          <DiffStatTable
            stat={state.diff.diffstat}
            hrefFor={(file) =>
              compareHref(resolvedRepo, {
                from: resolvedFrom,
                to: resolvedTo,
                path: file.path,
                ...options,
                view: "unified",
              })
            }
          />

          <DiffOptionsBar
            options={options}
            extraParams={extraFormParams}
            hrefFor={(patch) =>
              compareHref(resolvedRepo, {
                from: resolvedFrom,
                to: resolvedTo,
                path: resolvedPath,
                ...options,
                ...patch,
              })
            }
          />

          {resolvedView !== "stat" && (
            <DiffFileList truncated={state.diff.truncated} files={state.diff.files} view={resolvedView} />
          )}
        </>
      )}
    </div>
  );
}
