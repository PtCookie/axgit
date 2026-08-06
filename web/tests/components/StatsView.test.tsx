import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "vitest-browser-react";
import { page } from "vitest/browser";

import StatsView from "@/components/repo/StatsView";
import { ApiError } from "@/lib/api/client";
import { getStats } from "@/lib/api/repos";
import type { StatsResults } from "@/lib/api/schemas";

vi.mock("@/lib/api/repos", () => ({
  getStats: vi.fn(),
}));

const mockedGetStats = vi.mocked(getStats);

/** 12 ascending monthly buckets ending 2026-07-01, matching the shape the
 *  API always returns (fixed `BUCKET_COUNT`, docs/DECISIONS.md #28). */
function monthlyBuckets(commitsByMonth: Record<string, number> = {}) {
  return Array.from({ length: 12 }, (_, index) => {
    const month = index + 8; // Aug 2025 (index 0) .. Jul 2026 (index 11)
    const year = 2025 + Math.floor(month / 12);
    const normalizedMonth = (month % 12) + 1;
    const start = `${year}-${String(normalizedMonth).padStart(2, "0")}-01T00:00:00+00:00`;
    return { start, commits: commitsByMonth[start] ?? 0 };
  });
}

const JULY_START = "2026-07-01T00:00:00+00:00";

const RESULTS: StatsResults = {
  sha: "abc123def456abc123def456abc123def456abc",
  period: "month",
  truncated: false,
  author_count: 2,
  buckets: monthlyBuckets({ [JULY_START]: 3 }),
  authors: [
    {
      author: { name: "Alice", email_hash: "aaaa" },
      commits: 2,
      buckets: monthlyBuckets({ [JULY_START]: 2 }).map((bucket) => bucket.commits),
    },
    {
      author: { name: "Bob", email_hash: "bbbb" },
      commits: 1,
      buckets: monthlyBuckets({ [JULY_START]: 1 }).map((bucket) => bucket.commits),
    },
  ],
  others: null,
};

const EMPTY_REPO_RESULTS: StatsResults = {
  sha: null,
  period: "month",
  truncated: false,
  author_count: 0,
  buckets: [],
  authors: [],
  others: null,
};

describe("StatsView", () => {
  beforeEach(() => {
    mockedGetStats.mockReset();
  });

  afterEach(() => {
    cleanup();
  });

  it("fetches with the resolved period and ref", async () => {
    mockedGetStats.mockResolvedValue(RESULTS);
    render(<StatsView repo="git-compose" period="month" ref="main" />);

    await expect.element(page.getByText("Alice")).toBeVisible();
    expect(mockedGetStats).toHaveBeenCalledWith("git-compose", { period: "month", ref: "main" });
  });

  it("defaults to period=month when omitted", async () => {
    mockedGetStats.mockResolvedValue(RESULTS);
    render(<StatsView repo="git-compose" />);

    await expect.element(page.getByText("Alice")).toBeVisible();
    expect(mockedGetStats).toHaveBeenCalledWith("git-compose", { period: "month", ref: undefined });
  });

  it("marks the current period link and builds hrefs for the others", async () => {
    mockedGetStats.mockResolvedValue(RESULTS);
    render(<StatsView repo="git-compose" period="quarter" ref="main" />);

    await expect.element(page.getByText("Alice")).toBeVisible();
    const current = page.getByRole("link", { name: "Quarter" });
    await expect.element(current).toHaveAttribute("aria-current", "page");

    const week = page.getByRole("link", { name: "Week" });
    await expect.element(week).toHaveAttribute("href", "/git-compose/stats?period=week&ref=main");
    await expect.element(week).not.toHaveAttribute("aria-current");
  });

  it("renders the author breakdown with per-bucket columns and a total row", async () => {
    mockedGetStats.mockResolvedValue(RESULTS);
    render(<StatsView repo="git-compose" period="month" />);

    await expect.element(page.getByText("Alice")).toBeVisible();
    await expect.element(page.getByText("Bob")).toBeVisible();
    // Alice's total-commits cell.
    await expect.element(page.getByRole("cell", { name: "2", exact: true }).first()).toBeVisible();
    await expect.element(page.getByText("2 authors.")).toBeVisible();
    // The footer's per-bucket total for July matches the sum of both authors.
    await expect.element(page.getByRole("row", { name: /Total/ })).toBeVisible();
  });

  it("shows a truncation notice when the response is truncated", async () => {
    mockedGetStats.mockResolvedValue({ ...RESULTS, truncated: true });
    render(<StatsView repo="git-compose" period="month" />);

    await expect.element(page.getByRole("status")).toHaveTextContent("partial result");
  });

  it("shows a top-N note when the author list is cut by limit", async () => {
    mockedGetStats.mockResolvedValue({ ...RESULTS, author_count: 5, authors: [RESULTS.authors[0]] });
    render(<StatsView repo="git-compose" period="month" />);

    await expect.element(page.getByText("Showing top 1 of 5 authors.")).toBeVisible();
  });

  it("renders an Others row that reconciles the visible authors with the totals", async () => {
    // Alice alone is shown; Bob plus two more are folded into `others`, so the
    // July column reads 2 (Alice) + 3 (Others) = 5 (Total).
    mockedGetStats.mockResolvedValue({
      ...RESULTS,
      truncated: true,
      author_count: 4,
      buckets: monthlyBuckets({ [JULY_START]: 5 }),
      authors: [RESULTS.authors[0]],
      others: {
        count: 3,
        commits: 3,
        buckets: monthlyBuckets({ [JULY_START]: 3 }).map((bucket) => bucket.commits),
      },
    });
    render(<StatsView repo="git-compose" period="month" />);

    await expect.element(page.getByRole("row", { name: /Others \(3\)/ })).toBeVisible();
    // The aggregate is deliberately not an author row — no avatar, no name.
    await expect.element(page.getByText("Bob")).not.toBeInTheDocument();
    await expect.element(page.getByText("Showing top 1 of 4 authors.")).toBeVisible();
  });

  it("omits the Others row when the limit cut nothing", async () => {
    mockedGetStats.mockResolvedValue(RESULTS);
    render(<StatsView repo="git-compose" period="month" />);

    await expect.element(page.getByText("Alice")).toBeVisible();
    expect(page.getByRole("row", { name: /Others/ }).elements().length).toBe(0);
  });

  it("shows the empty state for a repository with no commits", async () => {
    mockedGetStats.mockResolvedValue(EMPTY_REPO_RESULTS);
    render(<StatsView repo="empty" period="month" />);

    await expect.element(page.getByText("No commits yet.")).toBeVisible();
  });

  it("shows an error message when the request fails", async () => {
    mockedGetStats.mockRejectedValue(new ApiError("internal", "boom", 500));
    render(<StatsView repo="git-compose" period="month" />);

    await expect.element(page.getByRole("alert")).toHaveTextContent("boom");
  });

  it("shows a not-found message for a 404", async () => {
    mockedGetStats.mockRejectedValue(new ApiError("repo_not_found", "nope", 404));
    render(<StatsView repo="nope" period="month" />);

    await expect.element(page.getByRole("alert")).toHaveTextContent("Repository not found.");
  });
});
