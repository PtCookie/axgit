import { lazy, Suspense } from "react";

import type { ReadmeFormat } from "@/lib/api/schemas";
import { Skeleton } from "@/components/ui/skeleton";

// Module scope, not inside the component — otherwise every render would mint
// a new lazy type and remount the markdown tree. Deliberately *not*
// pre-warmed alongside either caller's fetch (contrast `StatsView`'s
// `importStatsChart`): the whole point of this split is that a `rst`/`plain`
// readme never downloads react-markdown at all — pre-warming would defeat
// that before `format` is even known.
const ReadmeMarkdown = lazy(() => import("./ReadmeMarkdown"));

interface ReadmeBodyProps {
  format: ReadmeFormat;
  content: string;
  /** Passed straight through to `ReadmeMarkdown` — omitted by `SiteIntro.tsx`
   *  for the site-level readme, which has no single repository to resolve
   *  relative links/images against. See `ReadmeMarkdown`'s own doc comment. */
  repo?: string;
}

/** The render-only half shared by `ReadmeView.tsx` (a repository's own
 *  README) and `SiteIntro.tsx` (the site-level readme) — both already have
 *  a `ReadmeInfo`/`SiteReadme` in hand by the time this renders; fetching
 *  and the "no readme" empty state are each caller's own concern. */
export default function ReadmeBody({ format, content, repo }: ReadmeBodyProps) {
  if (format !== "markdown") {
    return (
      <pre className="border-border overflow-x-auto rounded-md border p-3 font-mono text-sm whitespace-pre-wrap">
        {content}
      </pre>
    );
  }

  return (
    <Suspense fallback={<Skeleton className="h-40 w-full" />}>
      <ReadmeMarkdown repo={repo} content={content} />
    </Suspense>
  );
}
