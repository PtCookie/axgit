import { apiFetch } from "./client";
import { encodeSegment } from "./path";
import type { RefsInfo, ReposResponse, RepoSummary } from "./schemas";

export function listRepos(): Promise<ReposResponse> {
  return apiFetch<ReposResponse>("/repos");
}

export function getRepo(name: string): Promise<RepoSummary> {
  return apiFetch<RepoSummary>(`/repos/${encodeSegment(name)}`);
}

export function getRefs(name: string): Promise<RefsInfo> {
  return apiFetch<RefsInfo>(`/repos/${encodeSegment(name)}/refs`);
}
