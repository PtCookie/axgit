import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "vitest-browser-react";
import { page } from "vitest/browser";

import RepoSummary from "@/components/repo/RepoSummary";
import { ApiError } from "@/lib/api/client";
import { ARCHIVE_FORMATS, getRepo } from "@/lib/api/repos";
import type { RepoSummary as RepoSummaryData } from "@/lib/api/schemas";

vi.mock("@/lib/api/repos", async (importOriginal) => {
  // `archiveUrl`/`feedUrl` are pure link builders (no network) — kept real
  // via `importOriginal` so the href assertions below exercise the actual
  // implementation, only `getRepo` needs mocking.
  const actual = await importOriginal<typeof import("@/lib/api/repos")>();
  return { ...actual, getRepo: vi.fn() };
});

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

    await expect.element(page.getByText("No commits yet.")).toBeVisible();
  });

  it("links to every archive download format and the Atom feed", async () => {
    mockedGetRepo.mockResolvedValue(SUMMARY);
    render(<RepoSummary repo="git-compose" />);

    for (const format of ARCHIVE_FORMATS) {
      const link = page.getByRole("link", { name: format });
      await expect.element(link).toBeVisible();
      await expect.element(link).toHaveAttribute("href", `/api/v1/repos/git-compose/archive/HEAD.${format}`);
    }

    const feed = page.getByRole("link", { name: "Atom" });
    await expect.element(feed).toHaveAttribute("href", "/api/v1/repos/git-compose/feed.atom");
  });

  it("omits archive/feed links for an empty repository", async () => {
    mockedGetRepo.mockResolvedValue({ ...SUMMARY, head: null, default_branch: null });
    render(<RepoSummary repo="scratch" />);

    await expect.element(page.getByText("No commits yet.")).toBeVisible();
    expect(page.getByRole("link", { name: "tar.gz" }).elements().length).toBe(0);
    expect(page.getByRole("link", { name: "Atom" }).elements().length).toBe(0);
  });

  it("shows an error message when the request fails", async () => {
    mockedGetRepo.mockRejectedValue(new ApiError("internal", "boom", 500));
    render(<RepoSummary repo="git-compose" />);

    await expect.element(page.getByRole("alert")).toHaveTextContent("boom");
  });

  it("links branch and tag counts to the refs page", async () => {
    mockedGetRepo.mockResolvedValue(SUMMARY);
    render(<RepoSummary repo="git-compose" />);

    const branches = page.getByRole("link", { name: "3 branches" });
    await expect.element(branches).toHaveAttribute("href", "/git-compose/refs");

    const tags = page.getByRole("link", { name: "1 tag" });
    await expect.element(tags).toHaveAttribute("href", "/git-compose/refs");
  });

  it("singularizes the branch count", async () => {
    mockedGetRepo.mockResolvedValue({ ...SUMMARY, branch_count: 1 });
    render(<RepoSummary repo="git-compose" />);

    await expect.element(page.getByRole("link", { name: "1 branch" })).toBeVisible();
  });

  it("shows the clone URL", async () => {
    mockedGetRepo.mockResolvedValue(SUMMARY);
    render(<RepoSummary repo="git-compose" />);

    await expect.element(page.getByText("git@git.ptcookie.net:git-compose.git")).toBeVisible();
  });
});
