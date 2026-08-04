/** One contiguous run of characters within an intra-line highlight, tagged
 *  with whether it changed between the old and new side. */
export interface Segment {
  text: string;
  changed: boolean;
}

/**
 * Above this (old-middle length × new-middle length, in characters) the
 * word-level diff is skipped and the whole trimmed middle is reported as one
 * changed segment instead. Bounds the O(n·m) DP table to roughly 1 MiB
 * (a `Uint32Array` cell per token pair) regardless of how long a single
 * diff line gets.
 */
const WORD_DIFF_BUDGET = 250_000;

/** Splits text into words, runs of whitespace, and individual punctuation
 *  characters — the token granularity a word-level diff highlights at. */
const TOKEN_PATTERN = /(\w+|\s+|[^\w\s])/g;

function tokenize(text: string): string[] {
  return text.match(TOKEN_PATTERN) ?? [];
}

function mergeAdjacent(segments: Segment[]): Segment[] {
  const merged: Segment[] = [];
  for (const segment of segments) {
    const last = merged[merged.length - 1];
    if (last && last.changed === segment.changed) {
      last.text += segment.text;
    } else {
      merged.push({ ...segment });
    }
  }
  return merged;
}

/** Word-level LCS between two already-trimmed strings — an O(n·m) DP over a
 *  flat `Uint32Array`, backtracked into alternating changed/unchanged runs. */
function wordDiff(oldText: string, newText: string): [Segment[], Segment[]] {
  const oldTokens = tokenize(oldText);
  const newTokens = tokenize(newText);
  const n = oldTokens.length;
  const m = newTokens.length;
  const width = m + 1;
  const dp = new Uint32Array((n + 1) * width);
  const at = (i: number, j: number) => i * width + j;

  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[at(i, j)] =
        oldTokens[i] === newTokens[j] ? dp[at(i + 1, j + 1)] + 1 : Math.max(dp[at(i + 1, j)], dp[at(i, j + 1)]);
    }
  }

  const oldOps: Segment[] = [];
  const newOps: Segment[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (oldTokens[i] === newTokens[j]) {
      oldOps.push({ text: oldTokens[i], changed: false });
      newOps.push({ text: newTokens[j], changed: false });
      i++;
      j++;
    } else if (dp[at(i + 1, j)] >= dp[at(i, j + 1)]) {
      oldOps.push({ text: oldTokens[i], changed: true });
      i++;
    } else {
      newOps.push({ text: newTokens[j], changed: true });
      j++;
    }
  }
  while (i < n) {
    oldOps.push({ text: oldTokens[i], changed: true });
    i++;
  }
  while (j < m) {
    newOps.push({ text: newTokens[j], changed: true });
    j++;
  }

  return [mergeAdjacent(oldOps), mergeAdjacent(newOps)];
}

/**
 * Computes intra-line highlight segments for a paired old/new line in the
 * side-by-side diff view. Three tiers, cheapest first:
 *
 * 1. Common prefix + common suffix trimmed off (unchanged) — covers the
 *    overwhelming majority of real edits (one identifier, one literal, one
 *    added argument) at O(n).
 * 2. If both trimmed middles are non-empty and within [`WORD_DIFF_BUDGET`],
 *    a word-level LCS diff of the middles.
 * 3. Otherwise the whole middle on each side is reported as one changed
 *    segment.
 *
 * Operates on Unicode code points (via `Array.from`, not UTF-16 code units)
 * so a surrogate pair is never split across a prefix/suffix boundary.
 */
export function intralineSegments(oldText: string, newText: string): [Segment[], Segment[]] {
  const oldChars = Array.from(oldText);
  const newChars = Array.from(newText);

  let prefix = 0;
  const maxPrefix = Math.min(oldChars.length, newChars.length);
  while (prefix < maxPrefix && oldChars[prefix] === newChars[prefix]) {
    prefix++;
  }

  let suffix = 0;
  const maxSuffix = Math.min(oldChars.length, newChars.length) - prefix;
  while (suffix < maxSuffix && oldChars[oldChars.length - 1 - suffix] === newChars[newChars.length - 1 - suffix]) {
    suffix++;
  }

  const oldPrefixText = oldChars.slice(0, prefix).join("");
  const newPrefixText = newChars.slice(0, prefix).join("");
  const oldSuffixText = suffix > 0 ? oldChars.slice(oldChars.length - suffix).join("") : "";
  const newSuffixText = suffix > 0 ? newChars.slice(newChars.length - suffix).join("") : "";

  const oldMid = oldChars.slice(prefix, oldChars.length - suffix);
  const newMid = newChars.slice(prefix, newChars.length - suffix);

  const oldSegments: Segment[] = [];
  const newSegments: Segment[] = [];
  if (oldPrefixText) oldSegments.push({ text: oldPrefixText, changed: false });
  if (newPrefixText) newSegments.push({ text: newPrefixText, changed: false });

  if (oldMid.length > 0 && newMid.length > 0 && oldMid.length * newMid.length <= WORD_DIFF_BUDGET) {
    const [oldWordSegments, newWordSegments] = wordDiff(oldMid.join(""), newMid.join(""));
    oldSegments.push(...oldWordSegments);
    newSegments.push(...newWordSegments);
  } else {
    if (oldMid.length > 0) oldSegments.push({ text: oldMid.join(""), changed: true });
    if (newMid.length > 0) newSegments.push({ text: newMid.join(""), changed: true });
  }

  if (oldSuffixText) oldSegments.push({ text: oldSuffixText, changed: false });
  if (newSuffixText) newSegments.push({ text: newSuffixText, changed: false });

  return [oldSegments, newSegments];
}
