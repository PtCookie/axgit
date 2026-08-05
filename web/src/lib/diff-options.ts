/**
 * Diff display options (`view`/`context`/`ignorews`), shared by the commit
 * page and the upcoming two-revision compare page. All state lives in the
 * URL — data islands remount fresh on every navigation (DECISIONS.md #24),
 * so there's nowhere else for it to live.
 */

export type DiffViewMode = "unified" | "split" | "stat";

/** `DiffViewMode` minus `"stat"` — the two modes that actually render hunks.
 *  `stat` never reaches `DiffFileList`/`DiffFile` (the caller doesn't render
 *  them at all in that mode), so their `view` prop is narrowed to this type
 *  rather than having to handle a third, meaningless case internally. */
export type HunkViewMode = Exclude<DiffViewMode, "stat">;

export interface DiffOptions {
  view: DiffViewMode;
  context: number;
  ignorews: boolean;
}

/** Matches the api's `context=` contract (`api/src/repo/diff.rs`):
 *  `0..=100`, defaulting to libgit2's own default of 3. This is a shorter,
 *  UI-facing subset of that range, not the api's validation — any value the
 *  api accepts can still be reached by hand-editing the URL. */
export const ALLOWED_CONTEXT = [1, 3, 5, 10, 20, 40] as const;
export const DEFAULT_CONTEXT = 3;
const DEFAULT_VIEW: DiffViewMode = "unified";

/** Parses `view=`/`context=`/`ignorews=` from a page URL's query string. An
 *  unrecognized `view` or a `context` outside [`ALLOWED_CONTEXT`] silently
 *  falls back to the default, rather than forwarding a bad value to the api. */
export function parseDiffOptions(search: string): DiffOptions {
  const params = new URLSearchParams(search);

  const rawView = params.get("view");
  const view: DiffViewMode = rawView === "split" || rawView === "stat" ? rawView : DEFAULT_VIEW;

  const rawContext = Number(params.get("context"));
  const context = (ALLOWED_CONTEXT as readonly number[]).includes(rawContext) ? rawContext : DEFAULT_CONTEXT;

  const ignorews = params.get("ignorews") === "1";

  return { view, context, ignorews };
}

/** `{ view, context, ignorews }` with anything at its default omitted — an
 *  unperturbed URL for the common case, and a stable response-cache key on
 *  the api side (`context=3` and no `context` at all must collide there). */
export function diffOptionsQuery(options: DiffOptions): Record<string, string> {
  const query: Record<string, string> = {};
  if (options.view !== DEFAULT_VIEW) query.view = options.view;
  if (options.context !== DEFAULT_CONTEXT) query.context = String(options.context);
  if (options.ignorews) query.ignorews = "1";
  return query;
}

/** The subset of `DiffOptions` the api's diff endpoints actually accept —
 *  `view` is a display-only choice made entirely client-side. */
export function diffApiParams(options: DiffOptions): { context?: number; ignorews?: number } {
  const params: { context?: number; ignorews?: number } = {};
  if (options.context !== DEFAULT_CONTEXT) params.context = options.context;
  if (options.ignorews) params.ignorews = 1;
  return params;
}
