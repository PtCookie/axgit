import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "vitest-browser-react";
import { page } from "vitest/browser";

import RefsView from "@/components/repo/RefsView";
import { ApiError } from "@/lib/api/client";
import { getRefs, getRepo } from "@/lib/api/repos";
import type { RefsInfo, RepoSummary } from "@/lib/api/schemas";

vi.mock("@/lib/api/repos", async (importOriginal) => {
  // `archiveUrl` is a pure link builder (no network) — kept real via
  // `importOriginal` so the Download column's href assertions below exercise
  // the actual implementation, same reasoning as `RepoSummary.test.tsx`.
  const actual = await importOriginal<typeof import("@/lib/api/repos")>();
  return { ...actual, getRefs: vi.fn(), getRepo: vi.fn() };
});

const mockedGetRefs = vi.mocked(getRefs);
const mockedGetRepo = vi.mocked(getRepo);

const REFS: RefsInfo = {
  branches: [
    { name: "main", target: "abc123def456", committed_at: "2026-07-24T13:06:00+09:00" },
    { name: "feature-x", target: "789abc012def", committed_at: "2026-07-25T09:00:00+09:00" },
  ],
  tags: [
    { name: "v1.0.0", target: "def456abc123", annotation: "First release", tagged_at: "2026-01-01T00:00:00+09:00" },
  ],
};

const SUMMARY: RepoSummary = {
  name: "git-compose",
  section: "infra",
  owner: "PtCookie",
  description: "Compose project of Git server",
  default_branch: "main",
  last_modified: "2026-07-24T13:06:00+09:00",
  head: "abc123def456",
  branch_count: 2,
  tag_count: 1,
  clone_url: "git@git.ptcookie.net:git-compose.git",
};

describe("RefsView", () => {
  beforeEach(() => {
    mockedGetRefs.mockReset();
    mockedGetRepo.mockReset();
    // Compare links are decoration (default branch fetched separately from
    // refs) — most tests don't care about the value, so default it here and
    // override per test where it matters.
    mockedGetRepo.mockResolvedValue(SUMMARY);
  });

  afterEach(() => {
    cleanup();
  });

  it("lists branches and tags", async () => {
    mockedGetRefs.mockResolvedValue(REFS);
    render(<RefsView repo="git-compose" />);

    await expect.element(page.getByText("main")).toBeVisible();
    await expect.element(page.getByText("v1.0.0")).toBeVisible();
    await expect.element(page.getByText("First release")).toBeVisible();
  });

  it("shows empty-state messages when there are no branches or tags", async () => {
    mockedGetRefs.mockResolvedValue({ branches: [], tags: [] });
    render(<RefsView repo="scratch" />);

    await expect.element(page.getByText("No branches.")).toBeVisible();
    await expect.element(page.getByText("No tags.")).toBeVisible();
  });

  it("shows an error message when the request fails", async () => {
    mockedGetRefs.mockRejectedValue(new ApiError("internal", "boom", 500));
    render(<RefsView repo="git-compose" />);

    await expect.element(page.getByRole("alert")).toHaveTextContent("boom");
  });

  it("links a non-default branch's Compare cell to the default branch, but not the default branch's own row", async () => {
    mockedGetRefs.mockResolvedValue(REFS);
    render(<RefsView repo="git-compose" />);

    const compareLink = page.getByRole("link", { name: "Compare main with feature-x" });
    await expect.element(compareLink).toHaveAttribute("href", "/git-compose/diff?from=main&to=feature-x");

    // The default branch's own row has no Compare link — just an em dash,
    // same as the table's other missing-value cells.
    await expect.element(page.getByRole("link", { name: "Compare main with main" })).not.toBeInTheDocument();
  });

  it("links a tag's Compare cell from the tag to the default branch (reversed direction from branches)", async () => {
    mockedGetRefs.mockResolvedValue(REFS);
    render(<RefsView repo="git-compose" />);

    const compareLink = page.getByRole("link", { name: "Compare v1.0.0 with main" });
    await expect.element(compareLink).toHaveAttribute("href", "/git-compose/diff?from=v1.0.0&to=main");
  });

  it("renders the table without a Compare link when the default-branch fetch fails", async () => {
    mockedGetRefs.mockResolvedValue(REFS);
    mockedGetRepo.mockRejectedValue(new ApiError("internal", "boom", 500));
    render(<RefsView repo="git-compose" />);

    await expect.element(page.getByText("feature-x")).toBeVisible();
    await expect.element(page.getByRole("link", { name: "Compare main with feature-x" })).not.toBeInTheDocument();
  });

  it("offers tar.gz and zip downloads for a tag", async () => {
    mockedGetRefs.mockResolvedValue(REFS);
    render(<RefsView repo="git-compose" />);

    await expect
      .element(page.getByRole("link", { name: "Download v1.0.0 as tar.gz" }))
      .toHaveAttribute("href", "/api/v1/repos/git-compose/archive/v1.0.0.tar.gz");
    await expect
      .element(page.getByRole("link", { name: "Download v1.0.0 as zip" }))
      .toHaveAttribute("href", "/api/v1/repos/git-compose/archive/v1.0.0.zip");
  });

  it("keeps a tag name's slash intact in its download href", async () => {
    mockedGetRefs.mockResolvedValue({
      branches: REFS.branches,
      tags: [...REFS.tags, { name: "release/1.0", target: "abc123", annotation: null, tagged_at: null }],
    });
    render(<RefsView repo="git-compose" />);

    await expect
      .element(page.getByRole("link", { name: "Download release/1.0 as tar.gz" }))
      .toHaveAttribute("href", "/api/v1/repos/git-compose/archive/release/1.0.tar.gz");
  });

  it("does not offer downloads for branches", async () => {
    mockedGetRefs.mockResolvedValue(REFS);
    render(<RefsView repo="git-compose" />);

    await expect.element(page.getByRole("link", { name: "Download main as tar.gz" })).not.toBeInTheDocument();
  });

  it("links a tag name to its tag detail page", async () => {
    mockedGetRefs.mockResolvedValue(REFS);
    render(<RefsView repo="git-compose" />);

    await expect
      .element(page.getByRole("link", { name: "v1.0.0", exact: true }))
      .toHaveAttribute("href", "/git-compose/tag/v1.0.0");
  });

  it("keeps a tag name's slash intact in its tag detail href", async () => {
    mockedGetRefs.mockResolvedValue({
      branches: REFS.branches,
      tags: [...REFS.tags, { name: "release/1.0", target: "abc123", annotation: null, tagged_at: null }],
    });
    render(<RefsView repo="git-compose" />);

    await expect
      .element(page.getByRole("link", { name: "release/1.0", exact: true }))
      .toHaveAttribute("href", "/git-compose/tag/release/1.0");
  });
});
