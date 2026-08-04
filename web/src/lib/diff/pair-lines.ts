import type { Line } from "@/lib/api/schemas";

/**
 * One row of a side-by-side diff. `"context"` rows always have both sides
 * (the same line, since it didn't change); `"change"` rows may have either
 * side `null` when one side's run is longer than the other's.
 */
export type SplitRow =
  { kind: "context"; old: Line; new: Line } | { kind: "change"; old: Line | null; new: Line | null };

/**
 * Pairs a hunk's flat `Line[]` into side-by-side rows for `SplitHunk`. Single
 * pass, same algorithm as cgit's `ui-ssdiff.c`: consecutive deletions and
 * additions are collected separately and paired index-for-index once the
 * run ends, so a 3-deletion/1-addition block becomes one paired row plus two
 * `{ new: null }` rows — never assuming the two runs are the same length.
 *
 * Hunks are cut at hunk boundaries, never mid-hunk (`repo/diff.rs`), so every
 * hunk handed to this function is structurally complete.
 */
export function pairHunkLines(lines: Line[]): SplitRow[] {
  const rows: SplitRow[] = [];
  let dels: Line[] = [];
  let adds: Line[] = [];

  const flush = () => {
    const count = Math.max(dels.length, adds.length);
    for (let index = 0; index < count; index++) {
      rows.push({ kind: "change", old: dels[index] ?? null, new: adds[index] ?? null });
    }
    dels = [];
    adds = [];
  };

  for (const line of lines) {
    if (line.origin === " ") {
      flush();
      rows.push({ kind: "context", old: line, new: line });
    } else if (line.origin === "-") {
      // A deletion following some already-collected additions starts a new
      // change block (git never interleaves +/- this way within one run,
      // but nothing here assumes it can't happen).
      if (adds.length > 0) {
        flush();
      }
      dels.push(line);
    } else if (line.origin === "+") {
      adds.push(line);
    }
  }
  flush();

  return rows;
}
