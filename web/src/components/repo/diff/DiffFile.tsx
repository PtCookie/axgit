import type { FileDiff } from "@/lib/api/schemas";
import type { DiffViewMode } from "@/lib/diff-options";
import { renamePathLabel, STATUS_CLASS, STATUS_LABEL } from "@/lib/format/diff-status";
import SplitHunk from "@/components/repo/diff/SplitHunk";
import UnifiedHunk from "@/components/repo/diff/UnifiedHunk";

interface DiffFileProps {
  file: FileDiff;
  /** Position within the current file list (0-based) — used for the `#diff-N`
   *  anchor cgit-compat-redirected changeset links land on (DECISIONS.md #35). */
  index: number;
  view: DiffViewMode;
}

/** One file's diff: a collapsible summary line plus its hunks (or a binary/notice). */
export default function DiffFile({ file, index, view }: DiffFileProps) {
  // A whole added/deleted file has nothing on one side either way — an
  // entire empty split column is pure waste, so it always renders unified
  // regardless of the page's chosen view (cgit does the same).
  const effectiveView: DiffViewMode = file.status === "added" || file.status === "deleted" ? "unified" : view;

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
                <SplitHunk key={hunk.header} hunk={hunk} />
              ) : (
                <UnifiedHunk key={hunk.header} hunk={hunk} />
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
