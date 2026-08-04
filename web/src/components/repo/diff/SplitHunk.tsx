import { useMemo } from "react";

import type { Hunk } from "@/lib/api/schemas";
import type { Segment } from "@/lib/diff/intraline";
import { intralineSegments } from "@/lib/diff/intraline";
import { pairHunkLines, type SplitRow } from "@/lib/diff/pair-lines";
import { cn } from "@/lib/utils";

function SegmentSpans({ segments, tint }: { segments: Segment[]; tint: "red" | "green" }) {
  const changedClass = tint === "red" ? "bg-red-500/30 dark:bg-red-500/40" : "bg-green-500/30 dark:bg-green-500/40";
  return segments.map((segment, index) =>
    segment.changed ? (
      // eslint-disable-next-line @eslint-react/no-array-index-key -- segments have no stable identity
      <span key={index} className={cn("rounded-[2px]", changedClass)}>
        {segment.text}
      </span>
    ) : (
      // eslint-disable-next-line @eslint-react/no-array-index-key -- segments have no stable identity
      <span key={index}>{segment.text}</span>
    ),
  );
}

const LINENO_CLASS = "text-muted-foreground w-10 shrink-0 px-2 text-right font-mono select-none";
const CONTENT_CLASS = "font-mono whitespace-pre-wrap break-words";

function SplitRowView({ row }: { row: SplitRow }) {
  // Computed unconditionally (rules-of-hooks) — `null` for context rows and
  // for a change row where one side is missing (nothing to compare).
  const segments = useMemo(() => {
    if (row.kind === "change" && row.old && row.new) {
      return intralineSegments(row.old.content, row.new.content);
    }
    return null;
  }, [row]);

  if (row.kind === "context") {
    return (
      <tr>
        <td className={LINENO_CLASS}>{row.old.old_lineno}</td>
        <td className={CONTENT_CLASS}>{row.old.content}</td>
        <td className={LINENO_CLASS}>{row.new.new_lineno}</td>
        <td className={CONTENT_CLASS}>{row.new.content}</td>
      </tr>
    );
  }

  // A `null` side gets a filler tint so "nothing here" reads differently
  // from "unchanged" (which has no tint at all).
  const oldBg = row.old ? "bg-red-500/10 dark:bg-red-500/20" : "bg-muted/30";
  const newBg = row.new ? "bg-green-500/10 dark:bg-green-500/20" : "bg-muted/30";

  return (
    <tr>
      <td className={cn(LINENO_CLASS, oldBg)}>{row.old?.old_lineno ?? ""}</td>
      <td className={cn(CONTENT_CLASS, oldBg)}>
        {segments ? <SegmentSpans segments={segments[0]} tint="red" /> : row.old?.content}
      </td>
      <td className={cn(LINENO_CLASS, newBg)}>{row.new?.new_lineno ?? ""}</td>
      <td className={cn(CONTENT_CLASS, newBg)}>
        {segments ? <SegmentSpans segments={segments[1]} tint="green" /> : row.new?.content}
      </td>
    </tr>
  );
}

/**
 * One hunk rendered as a side-by-side (old | new) diff table. Both sides of
 * a row share one `<tr>` so their heights sync for free — a `null` side gets
 * a muted filler cell rather than an empty one. Narrow viewports scroll
 * horizontally (`min-w-[52rem]` on the table, `overflow-x-auto` on the
 * wrapper in `DiffFile`) rather than crushing the two columns unreadably.
 */
export default function SplitHunk({ hunk }: { hunk: Hunk }) {
  const rows = useMemo(() => pairHunkLines(hunk.lines), [hunk]);

  return (
    <table className="w-full min-w-[52rem] table-fixed border-collapse text-sm">
      <colgroup>
        <col className="w-10" />
        <col />
        <col className="w-10" />
        <col />
      </colgroup>
      <tbody>
        <tr>
          <td colSpan={4} className="text-muted-foreground bg-muted/30 px-2 py-1 font-mono">
            {hunk.header}
          </td>
        </tr>
        {rows.map((row, index) => (
          // eslint-disable-next-line @eslint-react/no-array-index-key -- rows have no stable identity
          <SplitRowView key={index} row={row} />
        ))}
      </tbody>
    </table>
  );
}
