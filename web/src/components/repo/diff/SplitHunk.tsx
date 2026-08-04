import { useMemo } from "react";

import type { Hunk, Line } from "@/lib/api/schemas";
import { intralineSegments, type Segment } from "@/lib/diff/intraline";
import { mergeTokensWithSegments } from "@/lib/diff/merge-tokens";
import { pairHunkLines, type SplitRow } from "@/lib/diff/pair-lines";
import type { HighlightedLine } from "@/lib/format/highlight";
import TokenSpans from "@/components/repo/diff/TokenSpans";
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

/** Shiki foreground color (`style`) plus intra-line background (`changed`)
 *  in one pass of spans — see `lib/diff/merge-tokens.ts`. */
function MergedSpans({
  tokens,
  segments,
  tint,
}: {
  tokens: HighlightedLine;
  segments: Segment[];
  tint: "red" | "green";
}) {
  const changedClass = tint === "red" ? "bg-red-500/30 dark:bg-red-500/40" : "bg-green-500/30 dark:bg-green-500/40";
  const fragments = mergeTokensWithSegments(tokens, segments);
  return fragments.map((fragment, index) => (
    <span
      // eslint-disable-next-line @eslint-react/no-array-index-key -- fragments have no stable identity
      key={index}
      style={fragment.style}
      className={fragment.changed ? cn("rounded-[2px]", changedClass) : undefined}
    >
      {fragment.text}
    </span>
  ));
}

const LINENO_CLASS = "text-muted-foreground w-10 shrink-0 px-2 text-right font-mono select-none";
const CONTENT_CLASS = "font-mono whitespace-pre-wrap break-words";

interface SplitRowViewProps {
  row: SplitRow;
  highlights?: Map<Line, HighlightedLine> | null;
}

function SplitRowView({ row, highlights }: SplitRowViewProps) {
  // Computed unconditionally (rules-of-hooks) — `null` for context rows and
  // for a change row where one side is missing (nothing to compare).
  const segments = useMemo(() => {
    if (row.kind === "change" && row.old && row.new) {
      return intralineSegments(row.old.content, row.new.content);
    }
    return null;
  }, [row]);

  if (row.kind === "context") {
    // Context rows share one `Line` object for both sides (`pair-lines.ts`),
    // so there's only ever one lookup here regardless of which side asks.
    const tokens = highlights?.get(row.old);
    return (
      <tr>
        <td className={LINENO_CLASS}>{row.old.old_lineno}</td>
        <td className={CONTENT_CLASS}>{tokens ? <TokenSpans tokens={tokens} /> : row.old.content}</td>
        <td className={LINENO_CLASS}>{row.new.new_lineno}</td>
        <td className={CONTENT_CLASS}>{tokens ? <TokenSpans tokens={tokens} /> : row.new.content}</td>
      </tr>
    );
  }

  // A `null` side gets a filler tint so "nothing here" reads differently
  // from "unchanged" (which has no tint at all).
  const oldBg = row.old ? "bg-red-500/10 dark:bg-red-500/20" : "bg-muted/30";
  const newBg = row.new ? "bg-green-500/10 dark:bg-green-500/20" : "bg-muted/30";
  const oldTokens = row.old ? highlights?.get(row.old) : undefined;
  const newTokens = row.new ? highlights?.get(row.new) : undefined;

  return (
    <tr>
      <td className={cn(LINENO_CLASS, oldBg)}>{row.old?.old_lineno ?? ""}</td>
      <td className={cn(CONTENT_CLASS, oldBg)}>
        {segments && oldTokens ? (
          <MergedSpans tokens={oldTokens} segments={segments[0]} tint="red" />
        ) : segments ? (
          <SegmentSpans segments={segments[0]} tint="red" />
        ) : oldTokens ? (
          <TokenSpans tokens={oldTokens} />
        ) : (
          row.old?.content
        )}
      </td>
      <td className={cn(LINENO_CLASS, newBg)}>{row.new?.new_lineno ?? ""}</td>
      <td className={cn(CONTENT_CLASS, newBg)}>
        {segments && newTokens ? (
          <MergedSpans tokens={newTokens} segments={segments[1]} tint="green" />
        ) : segments ? (
          <SegmentSpans segments={segments[1]} tint="green" />
        ) : newTokens ? (
          <TokenSpans tokens={newTokens} />
        ) : (
          row.new?.content
        )}
      </td>
    </tr>
  );
}

interface SplitHunkProps {
  hunk: Hunk;
  /** `Line` object identity → Shiki tokens, from `highlightFileDiff` — a
   *  missing entry (unsupported language, still loading, or highlighting
   *  skipped for an over-threshold file) falls back to plain text/intra-line
   *  segments only, same as `UnifiedHunk`. */
  highlights?: Map<Line, HighlightedLine> | null;
}

/**
 * One hunk rendered as a side-by-side (old | new) diff table. Both sides of
 * a row share one `<tr>` so their heights sync for free — a `null` side gets
 * a muted filler cell rather than an empty one. Narrow viewports scroll
 * horizontally (`min-w-[52rem]` on the table, `overflow-x-auto` on the
 * wrapper in `DiffFile`) rather than crushing the two columns unreadably.
 */
export default function SplitHunk({ hunk, highlights }: SplitHunkProps) {
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
          <SplitRowView key={index} row={row} highlights={highlights} />
        ))}
      </tbody>
    </table>
  );
}
