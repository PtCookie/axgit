import { useEffect, useState } from "react";

import type { FileDiff, Line } from "@/lib/api/schemas";
import type { HunkViewMode } from "@/lib/diff-options";
import { highlightFileDiff } from "@/lib/diff/file-highlights";
import { renamePathLabel, STATUS_CLASS, STATUS_LABEL } from "@/lib/format/diff-status";
import type { HighlightedLine } from "@/lib/format/highlight";
import SplitHunk from "@/components/repo/diff/SplitHunk";
import UnifiedHunk from "@/components/repo/diff/UnifiedHunk";

interface DiffFileProps {
  file: FileDiff;
  /** Position within the current file list (0-based) — used for the `#diff-N`
   *  anchor cgit-compat-redirected changeset links land on (DECISIONS.md #35). */
  index: number;
  view: HunkViewMode;
}

/** One file's diff: a collapsible summary line plus its hunks (or a binary/notice). */
export default function DiffFile({ file, index, view }: DiffFileProps) {
  // A whole added/deleted file has nothing on one side either way — an
  // entire empty split column is pure waste, so it always renders unified
  // regardless of the page's chosen view (cgit does the same).
  const effectiveView: HunkViewMode = file.status === "added" || file.status === "deleted" ? "unified" : view;

  // `null` until highlighting resolves (or if the language isn't supported)
  // — every hunk renders plain text in the meantime, then swaps in tokens,
  // same "no loading flash" pattern as `CodeBlock`.
  const [highlights, setHighlights] = useState<Map<Line, HighlightedLine> | null>(null);

  useEffect(() => {
    let cancelled = false;
    highlightFileDiff(file).then((result) => {
      if (!cancelled) {
        setHighlights(result);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [file]);

  return (
    <details open id={`diff-${index + 1}`} className="border-border rounded-md border">
      <summary className="bg-muted/50 cursor-pointer px-3 py-2 font-mono text-sm">
        <span className={STATUS_CLASS[file.status]}>{STATUS_LABEL[file.status]}</span> {renamePathLabel(file)}{" "}
        <span className="text-muted-foreground">
          +{file.additions} −{file.deletions}
        </span>
      </summary>
      <div className="overflow-x-auto">
        {file.binary ? (
          <p className="text-muted-foreground px-3 py-2 text-sm">Binary file not shown.</p>
        ) : (
          <>
            {file.hunks.map((hunk) =>
              effectiveView === "split" ? (
                <SplitHunk key={hunk.header} hunk={hunk} highlights={highlights} />
              ) : (
                <UnifiedHunk key={hunk.header} hunk={hunk} highlights={highlights} />
              ),
            )}
            {file.truncated && (
              <p className="text-muted-foreground px-3 py-2 text-sm">Diff truncated (1000 lines max).</p>
            )}
          </>
        )}
      </div>
    </details>
  );
}
