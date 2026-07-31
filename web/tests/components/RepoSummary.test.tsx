import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "vitest-browser-react";
import { page } from "vitest/browser";

import RepoSummary from "@/components/repo/RepoSummary";
import { ApiError } from "@/lib/api/client";
import { getRepo } from "@/lib/api/repos";
import type { RepoSummary as RepoSummaryData } from "@/lib/api/schemas";

vi.mock("@/lib/api/repos", () => ({
  getRepo: vi.fn(),
}));

const mockedGetRepo = vi.mocked(getRepo);

const SUMMARY: RepoSummaryData = {
  name: "git-compose",
  section: "infra",
  owner: "PtCookie",
  description: "Compose project of Git server",
  default_branch: "main",
  last_modified: "2026-07-24T13:06:00+09:00",
  head: "abc123def456",
  branch_count: 3,
  tag_count: 1,
  clone_url: "git@git.ptcookie.net:git-compose.git",
};

describe("RepoSummary", () => {
  beforeEach(() => {
    mockedGetRepo.mockReset();
  });

  afterEach(() => {
    cleanup();
  });

  it("shows the repository summary fields", async () => {
    mockedGetRepo.mockResolvedValue(SUMMARY);
    render(<RepoSummary repo="git-compose" />);

    await expect.element(page.getByText("Compose project of Git server")).toBeVisible();
    await expect.element(page.getByText("PtCookie", { exact: true })).toBeVisible();
    await expect.element(page.getByText("main")).toBeVisible();
  });

  it("shows an empty-history message when there is no HEAD", async () => {
    mockedGetRepo.mockResolvedValue({ ...SUMMARY, head: null, default_branch: null });
    render(<RepoSummary repo="scratch" />);

    await expect.element(page.getByText("커밋이 없습니다.")).toBeVisible();
  });

  it("shows an error message when the request fails", async () => {
    mockedGetRepo.mockRejectedValue(new ApiError("internal", "boom", 500));
    render(<RepoSummary repo="git-compose" />);

    await expect.element(page.getByRole("alert")).toHaveTextContent("boom");
  });
});
