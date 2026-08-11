import { DownloadSimpleIcon } from "@phosphor-icons/react/dist/ssr/DownloadSimple";
import { GitBranchIcon } from "@phosphor-icons/react/dist/ssr/GitBranch";
import { RssIcon } from "@phosphor-icons/react/dist/ssr/Rss";
import { TagIcon } from "@phosphor-icons/react/dist/ssr/Tag";
import { useEffect, useState, type ReactNode } from "react";

import { ApiError } from "@/lib/api/client";
import { ARCHIVE_FORMATS, archiveUrl, feedUrl, getRepo } from "@/lib/api/repos";
import type { RepoSummary as RepoSummaryData } from "@/lib/api/schemas";
import { formatAbsoluteTime, formatRelativeTime } from "@/lib/format/time";
import { refsHref } from "@/lib/repo-href";
import { repoFromPathname } from "@/lib/repo-param";
import { cn } from "@/lib/utils";
import { Skeleton } from "@/components/ui/skeleton";

type State =
  { status: "loading" } | { status: "error"; error: ApiError } | { status: "data"; summary: RepoSummaryData };

interface RepoSummaryProps {
  /**
   * Omitted by the prerendered `/{repo}` shell, which is built under a
   * placeholder param (`lib/shell.ts`) — the real name is read from the URL
   * in the browser. `client:only` guarantees this default is only ever
   * evaluated there.
   */
  repo?: string;
}

/** One label/value pair, stacked (label above value) to fit the narrow
 *  sidebar (`pages/[repo]/index.astro`). Still a `<dl>` row — only the
 *  presentation moved from the old `grid-cols-[auto_1fr]` layout. */
function MetaItem({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div>
      <dt className="text-muted-foreground text-xs font-medium tracking-wide uppercase">{label}</dt>
      <dd className="mt-1 text-sm break-words">{children}</dd>
    </div>
  );
}

const LINK_CLASS = "text-primary underline underline-offset-2";

/** An icon + label link row. `aria-hidden` on the icon is load-bearing: the
 *  accessible name must stay exactly the label text ("tar.gz", "Atom", …),
 *  which is what both the component tests and the e2e spec look these up
 *  by. Same `size-4` / `aria-hidden` idiom as `components/theme.tsx`. */
function MetaLink({ href, Icon, children }: { href: string; Icon: typeof GitBranchIcon; children: ReactNode }) {
  return (
    <a href={href} className={cn(LINK_CLASS, "inline-flex items-center gap-1.5")}>
      <Icon className="size-4" aria-hidden="true" />
      {children}
    </a>
  );
}

/** One `MetaItem`-shaped placeholder — label bar above value bar, same
 *  rhythm as the real `dl` so nothing shifts when data lands. */
function SkeletonMetaItem() {
  return (
    <div className="space-y-1">
      <Skeleton className="h-3 w-16" />
      <Skeleton className="h-4 w-28" />
    </div>
  );
}

/** Also rendered statically into the page shell as the island's
 *  `slot="fallback"`, so the prerendered HTML is not blank. Sized for the
 *  18rem sidebar it sits in (`pages/[repo]/index.astro`), not a full-width
 *  block. */
export function RepoSummarySkeleton() {
  return (
    <div className="space-y-4" aria-busy="true">
      <Skeleton className="h-4 w-full" />
      <Skeleton className="h-4 w-3/4" />
      <div className="border-border space-y-4 border-t pt-4">
        <SkeletonMetaItem />
        <SkeletonMetaItem />
        <SkeletonMetaItem />
        <SkeletonMetaItem />
      </div>
    </div>
  );
}

export default function RepoSummary({ repo }: RepoSummaryProps) {
  const resolvedRepo = repo ?? repoFromPathname(window.location.pathname);
  const [state, setState] = useState<State>({ status: "loading" });

  useEffect(() => {
    let cancelled = false;

    getRepo(resolvedRepo)
      .then((summary) => {
        if (!cancelled) {
          setState({ status: "data", summary });
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
  }, [resolvedRepo]);

  if (state.status === "loading") {
    return <RepoSummarySkeleton />;
  }

  if (state.status === "error") {
    const message = state.error.status === 404 ? "Repository not found." : state.error.message;
    return (
      <p role="alert" className="text-destructive text-sm">
        Failed to load repository: {message}
      </p>
    );
  }

  const { summary } = state;

  return (
    <div className="space-y-4">
      {summary.description && <p className="text-foreground text-sm">{summary.description}</p>}

      <dl className={cn("space-y-4", summary.description && "border-border border-t pt-4")}>
        {summary.section && <MetaItem label="Section">{summary.section}</MetaItem>}
        {summary.owner && <MetaItem label="Owner">{summary.owner}</MetaItem>}
        <MetaItem label="Default branch">{summary.default_branch ?? "—"}</MetaItem>
        <MetaItem label="Last activity">
          {summary.last_modified ? (
            <span title={formatAbsoluteTime(summary.last_modified)}>{formatRelativeTime(summary.last_modified)}</span>
          ) : (
            "—"
          )}
        </MetaItem>
        <MetaItem label="HEAD">
          <span className="font-mono break-all">{summary.head ?? "—"}</span>
        </MetaItem>
        {/* Counts are folded into the link text rather than a bare
            `<a>{count}</a>` — a link whose accessible name is just "3" is
            useless out of context (WCAG 2.4.4). Both point at the refs
            page; a `#branches`/`#tags` fragment wouldn't scroll, since
            `RefsView` is `client:only` and hasn't mounted yet. */}
        <MetaItem label="Refs">
          <span className="flex flex-wrap items-center gap-x-4 gap-y-1">
            <MetaLink href={refsHref(summary.name)} Icon={GitBranchIcon}>
              {summary.branch_count} {summary.branch_count === 1 ? "branch" : "branches"}
            </MetaLink>
            <MetaLink href={refsHref(summary.name)} Icon={TagIcon}>
              {summary.tag_count} {summary.tag_count === 1 ? "tag" : "tags"}
            </MetaLink>
          </span>
        </MetaItem>
        {summary.clone_url && (
          <MetaItem label="Clone">
            <code className="border-border bg-muted/50 block rounded-md border px-2 py-1.5 font-mono text-xs break-all">
              {summary.clone_url}
            </code>
          </MetaItem>
        )}
        {summary.head !== null && (
          <>
            <MetaItem label="Download">
              <span className="flex flex-wrap items-center gap-x-4 gap-y-1">
                {ARCHIVE_FORMATS.map((format) => (
                  <MetaLink key={format} href={archiveUrl(summary.name, undefined, format)} Icon={DownloadSimpleIcon}>
                    {format}
                  </MetaLink>
                ))}
              </span>
            </MetaItem>
            <MetaItem label="Feed">
              <span className="flex flex-wrap items-center gap-x-4 gap-y-1">
                <MetaLink href={feedUrl(summary.name)} Icon={RssIcon}>
                  Atom
                </MetaLink>
                <MetaLink href={feedUrl(summary.name, { all: 1 })} Icon={RssIcon}>
                  All refs
                </MetaLink>
              </span>
            </MetaItem>
          </>
        )}
      </dl>

      {summary.head === null && <p className="text-muted-foreground text-sm">No commits yet.</p>}
    </div>
  );
}
