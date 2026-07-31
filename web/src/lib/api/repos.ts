import { apiFetch } from "./client";
import type { ReposResponse } from "./schemas";

export function listRepos(): Promise<ReposResponse> {
  return apiFetch<ReposResponse>("/repos");
}
