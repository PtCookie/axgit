import { apiFetch } from "./client";
import { encodeSegment } from "./path";
import type { CommitDetail, CommitDiff, CommitsPage, RefsInfo, ReposResponse, RepoSummary } from "./schemas";

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

export function getCommitDiff(name: string, sha: string, path?: string): Promise<CommitDiff> {
  const query = buildQuery({ path });
  return apiFetch<CommitDiff>(`/repos/${encodeSegment(name)}/commits/${encodeSegment(sha)}/diff${query}`);
}
