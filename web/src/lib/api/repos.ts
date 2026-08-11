import { ApiError, apiFetch, apiUrl } from "./client";
import { encodePath, encodeSegment } from "./path";
import type {
  BlameInfo,
  BlobInfo,
  CommitDetail,
  CommitDiff,
  CommitsPage,
  ObjectDetail,
  ReadmeInfo,
  RefsInfo,
  ReposResponse,
  RepoSummary,
  RevDiff,
  SearchKind,
  SearchResults,
  StatsPeriod,
  StatsResults,
  TagDetail,
  TreeListing,
} from "./schemas";

/** Defaulted when `?ref=` is absent from the page URL — `resolve_ref_path`
 *  on the api side falls back to `HEAD` for an unrecognized/missing ref
 *  segment too, so passing it explicitly here doesn't need a refs lookup. */
const DEFAULT_REF = "HEAD";

/** Builds the `{ref}/{path...}` wildcard segment shared by tree/blob/raw —
 *  each part is percent-encoded independently (`encodePath` splits on `/`),
 *  since a literal `/` inside `ref` (a branch name) or `path` must survive
 *  for the api's refs longest-match to see it. */
function refPathSegment(ref: string | undefined, path: string): string {
  const encodedRef = encodePath(ref || DEFAULT_REF);
  return path ? `${encodedRef}/${encodePath(path)}` : encodedRef;
}

export function listRepos(): Promise<ReposResponse> {
  return apiFetch<ReposResponse>("/repos");
}

export function getRepo(name: string): Promise<RepoSummary> {
  return apiFetch<RepoSummary>(`/repos/${encodeSegment(name)}`);
}

export function getRefs(name: string): Promise<RefsInfo> {
  return apiFetch<RefsInfo>(`/repos/${encodeSegment(name)}/refs`);
}

/** `encodePath`, not `encodeSegment` — a tag name may itself contain `/`
 *  (`release/1.0`), same reasoning as `refPathSegment`. */
export function getTag(name: string, tagName: string): Promise<TagDetail> {
  return apiFetch<TagDetail>(`/repos/${encodeSegment(name)}/tags/${encodePath(tagName)}`);
}

/** `encodeSegment`, not `encodePath` — an oid never contains `/`. */
export function getObject(name: string, oid: string): Promise<ObjectDetail> {
  return apiFetch<ObjectDetail>(`/repos/${encodeSegment(name)}/objects/${encodeSegment(oid)}`);
}

/** The by-oid analogue of `rawUrl` — same dual use (plain `<a>` href, or
 *  `fetchRawBytes`ed for a binary blob's hex dump). */
export function objectRawUrl(name: string, oid: string): string {
  return apiUrl(`/repos/${encodeSegment(name)}/objects/${encodeSegment(oid)}/raw`);
}

export interface ListCommitsParams {
  ref?: string;
  path?: string;
  cursor?: string;
  limit?: number;
  /** `1` to have each entry carry its message `body` (`msg=1`). */
  msg?: number;
  /** `1` to follow the `path` filter across whole-file renames (`follow=1`).
   *  Ignored when `path` is unset. */
  follow?: number;
  /** `1` to have each entry carry first-parent file/line counts (`stat=1`). */
  stat?: number;
}

/** Builds a query string from the given params, omitting any that are unset
 *  (an empty `?` would needlessly perturb the response cache key). */
function buildQuery<T extends object>(params: T): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params) as [string, string | number | undefined][]) {
    if (value !== undefined && value !== "") {
      search.set(key, String(value));
    }
  }
  const query = search.toString();
  return query ? `?${query}` : "";
}

export function listCommits(name: string, params: ListCommitsParams = {}): Promise<CommitsPage> {
  const query = buildQuery(params);
  return apiFetch<CommitsPage>(`/repos/${encodeSegment(name)}/commits${query}`);
}

export function getCommit(name: string, sha: string): Promise<CommitDetail> {
  return apiFetch<CommitDetail>(`/repos/${encodeSegment(name)}/commits/${encodeSegment(sha)}`);
}

export interface DiffQueryParams {
  path?: string;
  context?: number;
  ignorews?: number;
}

export function getCommitDiff(name: string, sha: string, params: DiffQueryParams = {}): Promise<CommitDiff> {
  const query = buildQuery(params);
  return apiFetch<CommitDiff>(`/repos/${encodeSegment(name)}/commits/${encodeSegment(sha)}/diff${query}`);
}

/** Link-only (never `fetch`ed by the client), same pattern as `rawUrl` — a
 *  `git format-patch`-style series for this commit alone (`from` omitted),
 *  for `git am`. Doesn't take `context`/`ignorews`: the api ignores them on
 *  `/patch`, since a patch meant to be applied has no display options. */
export function commitPatchUrl(name: string, sha: string): string {
  return apiUrl(`/repos/${encodeSegment(name)}/patch?to=${encodeSegment(sha)}`);
}

/** Link-only (never `fetch`ed by the client) — plain unified diff against the
 *  commit's first parent, honoring the same display options as the
 *  structured diff above. */
export function commitRawDiffUrl(name: string, sha: string, params: DiffQueryParams = {}): string {
  const query = buildQuery({ ...params, to: sha });
  return apiUrl(`/repos/${encodeSegment(name)}/rawdiff${query}`);
}

export interface RevDiffParams {
  from?: string;
  to?: string;
  path?: string;
  context?: number;
  ignorews?: number;
  /** `GET /diff` only — skips hunk rendering, returning just the uncapped
   *  `diffstat` (cgit's `dt=2`). Meaningless on `/rawdiff`/`/patch`; no
   *  caller passes it through `compareRawDiffUrl`/`comparePatchUrl`, which
   *  share this same param shape. */
  stat?: number;
}

/** Arbitrary two-revision diff (`GET /diff`) — `?to=X` alone (no `from`) is a
 *  strict superset of `getCommitDiff(name, X, ...)`. */
export function getDiff(name: string, params: RevDiffParams = {}): Promise<RevDiff> {
  const query = buildQuery(params);
  return apiFetch<RevDiff>(`/repos/${encodeSegment(name)}/diff${query}`);
}

/** Link-only (never `fetch`ed by the client) — plain unified diff between
 *  `from` and `to`, same rules as `commitRawDiffUrl`. */
export function compareRawDiffUrl(name: string, params: RevDiffParams = {}): string {
  const query = buildQuery(params);
  return apiUrl(`/repos/${encodeSegment(name)}/rawdiff${query}`);
}

/** Link-only (never `fetch`ed by the client) — `git format-patch`-style
 *  series for the commit range `(from, to]`, for `git am`. No `context`/
 *  `ignorews`: the api ignores them on `/patch` (docs/API.md). */
export function comparePatchUrl(name: string, params: { from?: string; to?: string; path?: string } = {}): string {
  const query = buildQuery(params);
  return apiUrl(`/repos/${encodeSegment(name)}/patch${query}`);
}

export function getTree(name: string, ref: string | undefined, path: string): Promise<TreeListing> {
  return apiFetch<TreeListing>(`/repos/${encodeSegment(name)}/tree/${refPathSegment(ref, path)}`);
}

export function getBlob(name: string, ref: string | undefined, path: string): Promise<BlobInfo> {
  return apiFetch<BlobInfo>(`/repos/${encodeSegment(name)}/blob/${refPathSegment(ref, path)}`);
}

export function getBlame(name: string, ref: string | undefined, path: string): Promise<BlameInfo> {
  return apiFetch<BlameInfo>(`/repos/${encodeSegment(name)}/blame/${refPathSegment(ref, path)}`);
}

/** The raw content is streamed straight from the api. `BlobView` renders it
 *  as an `<a>` href directly; `HexDump` also `fetchRawBytes`es it for a
 *  binary blob (docs/DECISIONS.md #58) — both uses share this one URL. */
export function rawUrl(name: string, ref: string | undefined, path: string): string {
  return apiUrl(`/repos/${encodeSegment(name)}/raw/${refPathSegment(ref, path)}`);
}

/** Fetches raw bytes from a `rawUrl`/`objectRawUrl` link for the hex dump
 *  view (docs/DECISIONS.md #58) — the only client-side consumer of the raw
 *  endpoint's bytes rather than just its `href`. Throws `ApiError` on a
 *  non-`ok` response or a network failure, same contract as `apiFetch`. */
export async function fetchRawBytes(url: string): Promise<Uint8Array> {
  let response: Response;
  try {
    response = await fetch(url);
  } catch {
    throw new ApiError("internal", "network request failed", 0);
  }
  if (!response.ok) {
    throw new ApiError("internal", response.statusText || "request failed", response.status);
  }
  return new Uint8Array(await response.arrayBuffer());
}

export function getReadme(name: string, ref?: string): Promise<ReadmeInfo> {
  const query = buildQuery({ ref });
  return apiFetch<ReadmeInfo>(`/repos/${encodeSegment(name)}/readme${query}`);
}

/** Every format `GET /archive/{ref}.{format}` serves, in the order the UI
 *  lists them (tar variants grouped, zip last). Mirrors `FORMATS` in
 *  `api/src/handlers/archive.rs` (docs/DECISIONS.md #54). Exported because
 *  `RepoSummary`, `RefsView`, and `TagView` all render the same list — the
 *  type is derived from it so a format can't be added to one but not the
 *  other. */
export const ARCHIVE_FORMATS = ["tar.gz", "tar.bz2", "tar.xz", "tar.zst", "zip"] as const;

export type ArchiveFormat = (typeof ARCHIVE_FORMATS)[number];

/** Link-only (never `fetch`ed by the client), same pattern as `rawUrl` —
 *  the archive is streamed straight from the api as a download. `ref`
 *  defaults to `HEAD` like the other `{ref}/{path...}` helpers. */
export function archiveUrl(name: string, ref: string | undefined, format: ArchiveFormat): string {
  return apiUrl(`/repos/${encodeSegment(name)}/archive/${encodePath(ref || DEFAULT_REF)}.${format}`);
}

export interface FeedParams {
  ref?: string;
  path?: string;
  /** `1` to walk every branch and tag instead of just `ref` (`all=1`). */
  all?: number;
  limit?: number;
}

/** Link-only — an Atom feed URL. Defaults to the repository's default branch
 *  and 20 entries; `ref`/`path` scope it the way `logHref` scopes the log
 *  page. Called with no params, this is byte-identical to the pre-#61 URL. */
export function feedUrl(name: string, params: FeedParams = {}): string {
  return apiUrl(`/repos/${encodeSegment(name)}/feed.atom${buildQuery(params)}`);
}

export interface SearchParams {
  q: string;
  type?: SearchKind;
  ref?: string;
  limit?: number;
}

export function searchRepo(name: string, params: SearchParams): Promise<SearchResults> {
  const query = buildQuery(params);
  return apiFetch<SearchResults>(`/repos/${encodeSegment(name)}/search${query}`);
}

export interface StatsParams {
  period?: StatsPeriod;
  ref?: string;
  path?: string;
  limit?: number;
}

export function getStats(name: string, params: StatsParams = {}): Promise<StatsResults> {
  const query = buildQuery(params);
  return apiFetch<StatsResults>(`/repos/${encodeSegment(name)}/stats${query}`);
}
