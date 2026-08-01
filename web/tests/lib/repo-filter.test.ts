import { describe, expect, it } from "vitest";

import { filterRepos } from "@/lib/repo-filter";

import fixture from "../fixtures/repos.json";

const repos = fixture.repos;

describe("filterRepos", () => {
  it("returns the same array reference for an empty or whitespace-only query", () => {
    expect(filterRepos(repos, "")).toBe(repos);
    expect(filterRepos(repos, "   ")).toBe(repos);
  });

  it("matches case-insensitively on name", () => {
    expect(filterRepos(repos, "AXGIT").map((repo) => repo.name)).toEqual(["axgit"]);
  });

  it("matches on description", () => {
    expect(filterRepos(repos, "scratch repository").map((repo) => repo.name)).toEqual(["scratch"]);
  });

  it("matches on owner", () => {
    expect(filterRepos(repos, "PtCookie").map((repo) => repo.name)).toEqual(["git-compose", "axgit"]);
  });

  it("matches on section", () => {
    expect(filterRepos(repos, "tools").map((repo) => repo.name)).toEqual(["dotfiles"]);
  });

  it("requires every whitespace-separated term to match (AND), possibly different fields", () => {
    // "infra" only matches section, "compose" only matches the git-compose name.
    expect(filterRepos(repos, "infra compose").map((repo) => repo.name)).toEqual(["git-compose"]);
  });

  it("skips null fields instead of matching or throwing", () => {
    expect(filterRepos(repos, "dotfiles").map((repo) => repo.name)).toEqual(["dotfiles"]);
    expect(filterRepos(repos, "null")).toEqual([]);
  });

  it("returns an empty array when nothing matches", () => {
    expect(filterRepos(repos, "does-not-exist")).toEqual([]);
  });
});
