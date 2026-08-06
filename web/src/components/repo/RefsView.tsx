import { useEffect, useState } from "react";

import { ApiError } from "@/lib/api/client";
import { archiveUrl, getRefs, type ArchiveFormat } from "@/lib/api/repos";
import type { RefsInfo } from "@/lib/api/schemas";
import { useDefaultBranch } from "@/lib/default-branch";
import { formatAbsoluteTime, formatRelativeTime } from "@/lib/format/time";
import { compareHref } from "@/lib/repo-href";
import { repoFromPathname } from "@/lib/repo-param";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

type State = { status: "loading" } | { status: "error"; error: ApiError } | { status: "data"; refs: RefsInfo };

// cgit's tags-only Download column (`print_tag_downloads()`) — a branch
// archive's filename (`{repo}-{branch}.{format}`) names a moving target that
// changes meaning on every push, while a tag's is reproducible. `archiveUrl`
// itself accepts any ref (branches included); this is a scope choice, not a
// capability gap (docs/DECISIONS.md #51).
const ARCHIVE_FORMATS: readonly ArchiveFormat[] = ["tar.gz", "zip"];

/** Per-tag tar.gz/zip download links. Plain visible text with an overriding
 *  `aria-label`, not `IconLink` — the format name *is* the link's entire
 *  information content, so two identical download glyphs side by side would
 *  be indistinguishable without a hover (unlike `IconLink`'s distinct-icon
 *  rows in `TreeView.tsx`). Matches this table's own `Compare` column
 *  convention (visible text + a per-row `aria-label` override) rather than
 *  #48's icon convention. */
function ArchiveLinks({ repo, tagName }: { repo: string; tagName: string }) {
  return (
    <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
      {ARCHIVE_FORMATS.map((format) => (
        <a
          key={format}
          className="hover:underline"
          aria-label={`Download ${tagName} as ${format}`}
          href={archiveUrl(repo, tagName, format)}
        >
          {format}
        </a>
      ))}
    </span>
  );
}

interface RefsViewProps {
  /**
   * Omitted by the prerendered `/{repo}/refs` shell, which is built under a
   * placeholder param (`lib/shell.ts`) — the real name is read from the URL
   * in the browser. `client:only` guarantees this default is only ever
   * evaluated there.
   */
  repo?: string;
}

/** Also rendered statically into the page shell as the island's
 *  `slot="fallback"`, so the prerendered HTML is not blank. */
export function RefsViewSkeleton() {
  return (
    <div className="space-y-2" aria-busy="true">
      <Skeleton className="h-8 w-full" />
      <Skeleton className="h-8 w-full" />
    </div>
  );
}

export default function RefsView({ repo }: RefsViewProps) {
  const resolvedRepo = repo ?? repoFromPathname(window.location.pathname);
  const [state, setState] = useState<State>({ status: "loading" });
  // Powers each row's "Compare" link only — `RefsInfo` itself carries no
  // default-branch field, so it's fetched separately here (see the hook's
  // own doc comment for the "decoration, not required data" rule).
  const defaultBranch = useDefaultBranch(resolvedRepo);

  useEffect(() => {
    let cancelled = false;

    getRefs(resolvedRepo)
      .then((refs) => {
        if (!cancelled) {
          setState({ status: "data", refs });
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
    return <RefsViewSkeleton />;
  }

  if (state.status === "error") {
    const message = state.error.status === 404 ? "Repository not found." : state.error.message;
    return (
      <p role="alert" className="text-destructive text-sm">
        Failed to load refs: {message}
      </p>
    );
  }

  const { refs } = state;

  return (
    <div className="space-y-8">
      <section>
        <h2 className="text-muted-foreground mb-2 text-sm font-medium">Branches</h2>
        {refs.branches.length === 0 ? (
          <p className="text-muted-foreground text-sm">No branches.</p>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Commit</TableHead>
                <TableHead>Committed</TableHead>
                <TableHead>Compare</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {refs.branches.map((branch) => (
                <TableRow key={branch.name}>
                  <TableCell className="font-medium">{branch.name}</TableCell>
                  <TableCell className="text-muted-foreground font-mono">{branch.target.slice(0, 12)}</TableCell>
                  <TableCell className="text-muted-foreground">
                    {branch.committed_at ? (
                      <span title={formatAbsoluteTime(branch.committed_at)}>
                        {formatRelativeTime(branch.committed_at)}
                      </span>
                    ) : (
                      "—"
                    )}
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {defaultBranch === null ? null : branch.name === defaultBranch ? (
                      "—"
                    ) : (
                      <a
                        className="hover:underline"
                        aria-label={`Compare ${defaultBranch} with ${branch.name}`}
                        href={compareHref(resolvedRepo, { from: defaultBranch, to: branch.name })}
                      >
                        Compare
                      </a>
                    )}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </section>

      <section>
        <h2 className="text-muted-foreground mb-2 text-sm font-medium">Tags</h2>
        {refs.tags.length === 0 ? (
          <p className="text-muted-foreground text-sm">No tags.</p>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Commit</TableHead>
                <TableHead>Message</TableHead>
                <TableHead>Tagged</TableHead>
                <TableHead>Compare</TableHead>
                <TableHead>Download</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {refs.tags.map((tag) => (
                <TableRow key={tag.name}>
                  <TableCell className="font-medium">{tag.name}</TableCell>
                  <TableCell className="text-muted-foreground font-mono">{tag.target.slice(0, 12)}</TableCell>
                  <TableCell className="text-muted-foreground">{tag.annotation ?? "—"}</TableCell>
                  <TableCell className="text-muted-foreground">
                    {tag.tagged_at ? (
                      <span title={formatAbsoluteTime(tag.tagged_at)}>{formatRelativeTime(tag.tagged_at)}</span>
                    ) : (
                      "—"
                    )}
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {defaultBranch === null ? null : (
                      <a
                        className="hover:underline"
                        aria-label={`Compare ${tag.name} with ${defaultBranch}`}
                        href={compareHref(resolvedRepo, { from: tag.name, to: defaultBranch })}
                      >
                        Compare
                      </a>
                    )}
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    <ArchiveLinks repo={resolvedRepo} tagName={tag.name} />
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </section>
    </div>
  );
}
