import type { RepoInfo } from "@/lib/api/schemas";

/**
 * Client-side mirror of `api/src/repo/sort.rs::RepoOrder` — lets
 * `RepoList.tsx` re-sort an already-fetched page in memory when the user
 * clicks a column header, without a refetch (docs/DECISIONS.md #65). Every
 * rule here (nulls-last regardless of direction, `idle`'s reversed default,
 * `name`-ascending tiebreak, comparing parsed instants rather than the
 * formatted `last_modified` string) is the exact rule the API applies, so a
 * client-side sort and a server-side one of the same order never disagree.
 */

export type RepoSortKey = "name" | "desc" | "owner" | "idle" | "section";

export interface RepoOrder {
  key: RepoSortKey;
  reverse: boolean;
}

const KEYS: readonly RepoSortKey[] = ["name", "desc", "owner", "idle", "section"];

function isRepoSortKey(value: string): value is RepoSortKey {
  return (KEYS as readonly string[]).includes(value);
}

/** A key's un-prefixed direction: ascending for every key except `idle`,
 *  which defaults descending (most recently active first) — cgit's own
 *  `idle` sort. */
function defaultReverse(key: RepoSortKey): boolean {
  return key === "idle";
}

/** Parses a `?sort=` value. `undefined` for anything unrecognized — an
 *  invalid or absent client-side `?sort=` means "use whatever order the API
 *  actually returned" (`ReposResponse.sort`), not a client-side error. */
export function parseOrder(raw: string): RepoOrder | undefined {
  const negated = raw.startsWith("-");
  const rest = negated ? raw.slice(1) : raw;
  if (!isRepoSortKey(rest)) {
    return undefined;
  }
  return { key: rest, reverse: defaultReverse(rest) !== negated };
}

/** Inverse of `parseOrder` — the `?sort=` spelling for `order`. A leading
 *  `-` appears only when `reverse` differs from the key's own default. */
export function orderToParam(order: RepoOrder): string {
  return order.reverse !== defaultReverse(order.key) ? `-${order.key}` : order.key;
}

/** `key`'s own default order — what clicking a column that isn't currently
 *  active should switch to, rather than inheriting whatever direction was
 *  active on the previously-sorted column. */
export function defaultOrder(key: RepoSortKey): RepoOrder {
  return { key, reverse: defaultReverse(key) };
}

type StringKey = Exclude<RepoSortKey, "idle">;

function fieldValue(key: StringKey, repo: RepoInfo): string | null {
  switch (key) {
    case "name":
      return repo.name;
    case "desc":
      return repo.description;
    case "owner":
      return repo.owner;
    case "section":
      return repo.section;
  }
}

/** `null` always sorts last: `reverse` only flips the comparison between two
 *  present values, never the `null`-vs-present branches. */
function compareOptStr(a: string | null, b: string | null, reverse: boolean): number {
  if (a !== null && b !== null) {
    const cmp = a < b ? -1 : a > b ? 1 : 0;
    return reverse ? -cmp : cmp;
  }
  if (a !== null) return -1;
  if (b !== null) return 1;
  return 0;
}

/** Compares parsed instants, not the formatted `last_modified` string —
 *  matches `api/src/repo/sort.rs::compare_idle`'s reasoning: two repositories
 *  recorded under different UTC offsets would otherwise compare wrong. An
 *  unparseable value (shouldn't happen — the API only ever emits RFC 3339 or
 *  `null`) is treated the same as `null`. */
function compareIdle(a: RepoInfo, b: RepoInfo, reverse: boolean): number {
  const parse = (repo: RepoInfo): number | null => {
    if (repo.last_modified === null) return null;
    const ms = Date.parse(repo.last_modified);
    return Number.isNaN(ms) ? null : ms;
  };
  const aTime = parse(a);
  const bTime = parse(b);
  if (aTime !== null && bTime !== null) {
    const cmp = aTime < bTime ? -1 : aTime > bTime ? 1 : 0;
    return reverse ? -cmp : cmp;
  }
  if (aTime !== null) return -1;
  if (bTime !== null) return 1;
  return 0;
}

/** Sorts `repos` by `order`, returning a new array (never mutates `repos` —
 *  callers hold it in component state/props). Ties, including two
 *  repositories that both lack the sorted field, break by `name` ascending. */
export function sortRepos(repos: RepoInfo[], order: RepoOrder): RepoInfo[] {
  return [...repos].sort((a, b) => {
    const cmp =
      order.key === "idle"
        ? compareIdle(a, b, order.reverse)
        : compareOptStr(fieldValue(order.key, a), fieldValue(order.key, b), order.reverse);
    if (cmp !== 0) return cmp;
    return a.name < b.name ? -1 : a.name > b.name ? 1 : 0;
  });
}
