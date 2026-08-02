import { GitBranchIcon } from "@phosphor-icons/react/dist/ssr/GitBranch";
import { TagIcon } from "@phosphor-icons/react/dist/ssr/Tag";

import type { CommitRef } from "@/lib/commit-refs";
import { logHref, refsHref } from "@/lib/repo-href";
import { Badge } from "@/components/ui/badge";

interface RefBadgesProps {
  repo: string;
  refs: CommitRef[];
  /** Caps how many badges render before an overflow link takes over —
   *  unset (the commit detail header) shows every ref; the log table passes
   *  a small cap so a heavily-tagged commit can't blow out its row height
   *  (docs/DECISIONS.md #34). */
  max?: number;
}

/** Branch/tag decoration for a commit — one badge per ref whose tip is that
 *  commit, linking to the log filtered by that ref (`?ref=`, DECISIONS.md
 *  #18). Kind is encoded by icon, not colour: `--chart-2..5` are still
 *  unvalidated shadcn boilerplate (#29), so nothing new gets colour ahead
 *  of the dataviz validator. Renders nothing for an empty list. */
export default function RefBadges({ repo, refs, max }: RefBadgesProps) {
  if (refs.length === 0) {
    return null;
  }

  const shown = max ? refs.slice(0, max) : refs;
  const overflow = max ? refs.length - shown.length : 0;

  return (
    <span className="inline-flex flex-wrap items-center gap-1">
      {shown.map((ref) => (
        <Badge
          key={`${ref.kind}-${ref.name}`}
          variant={ref.kind === "branch" ? "secondary" : "outline"}
          render={<a href={logHref(repo, { ref: ref.name })} title={`${ref.kind}: ${ref.name}`} />}
        >
          {ref.kind === "branch" ? <GitBranchIcon aria-hidden weight="bold" /> : <TagIcon aria-hidden weight="bold" />}
          {ref.name}
        </Badge>
      ))}
      {overflow > 0 && (
        <a href={refsHref(repo)} className="text-muted-foreground hover:text-foreground text-xs underline">
          +{overflow}
        </a>
      )}
    </span>
  );
}
