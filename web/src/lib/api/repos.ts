import { apiFetch, apiUrl } from "./client";
import { encodePath, encodeSegment } from "./path";
import type {
  BlameInfo,
  BlobInfo,
  CommitDetail,
  CommitDiff,
  CommitsPage,
  ReadmeInfo,
  RefsInfo,
  ReposResponse,
  RepoSummary,
  SearchKind,
  SearchResults,
  StatsPeriod,
  StatsResults,
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

export interface ListCommitsParams {
  ref?: string;
  path?: string;
  cursor?: string;
  limit?: number;
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

export function getTree(name: string, ref: string | undefined, path: string): Promise<TreeListing> {
  return apiFetch<TreeListing>(`/repos/${encodeSegment(name)}/tree/${refPathSegment(ref, path)}`);
}

export function getBlob(name: string, ref: string | undefined, path: string): Promise<BlobInfo> {
  return apiFetch<BlobInfo>(`/repos/${encodeSegment(name)}/blob/${refPathSegment(ref, path)}`);
}

export function getBlame(name: string, ref: string | undefined, path: string): Promise<BlameInfo> {
  return apiFetch<BlameInfo>(`/repos/${encodeSegment(name)}/blame/${refPathSegment(ref, path)}`);
}

/** Link-only (never `fetch`ed by the client) — the raw content is streamed
 *  straight from the api, so `BlobView` renders it as an `<a>` href. */
export function rawUrl(name: string, ref: string | undefined, path: string): string {
  return apiUrl(`/repos/${encodeSegment(name)}/raw/${refPathSegment(ref, path)}`);
}

export function getReadme(name: string, ref?: string): Promise<ReadmeInfo> {
  const query = buildQuery({ ref });
  return apiFetch<ReadmeInfo>(`/repos/${encodeSegment(name)}/readme${query}`);
}

export type ArchiveFormat = "tar.gz" | "zip";

/** Link-only (never `fetch`ed by the client), same pattern as `rawUrl` —
 *  the archive is streamed straight from the api as a download. `ref`
 *  defaults to `HEAD` like the other `{ref}/{path...}` helpers. */
export function archiveUrl(name: string, ref: string | undefined, format: ArchiveFormat): string {
  return apiUrl(`/repos/${encodeSegment(name)}/archive/${encodePath(ref || DEFAULT_REF)}.${format}`);
}

/** Link-only — an Atom feed URL for the repository's default branch. */
export function feedUrl(name: string): string {
  return apiUrl(`/repos/${encodeSegment(name)}/feed.atom`);
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
  limit?: number;
}

export function getStats(name: string, params: StatsParams = {}): Promise<StatsResults> {
  const query = buildQuery(params);
  return apiFetch<StatsResults>(`/repos/${encodeSegment(name)}/stats${query}`);
}
