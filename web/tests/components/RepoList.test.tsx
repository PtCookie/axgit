import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "vitest-browser-react";
import { page } from "vitest/browser";

import RepoList from "@/components/RepoList";
import { ApiError } from "@/lib/api/client";
import { listRepos } from "@/lib/api/repos";

import fixture from "../fixtures/repos.json";

vi.mock("@/lib/api/repos", () => ({
  listRepos: vi.fn(),
}));

const mockedListRepos = vi.mocked(listRepos);

describe("RepoList", () => {
  beforeEach(() => {
    mockedListRepos.mockReset();
  });

  afterEach(() => {
    cleanup();
  });

  it("groups repositories by section, moving the unsectioned group last", async () => {
    mockedListRepos.mockResolvedValue(fixture);
    render(<RepoList />);

    const headings = page.getByRole("heading", { level: 2 });
    await expect.element(headings.first()).toHaveTextContent("infra");
    await expect.element(headings.last()).toHaveTextContent("Other");
    const link = page.getByRole("link", { name: "git-compose" });
    await expect.element(link).toBeVisible();
    await expect.element(link).toHaveAttribute("href", "/git-compose");
  });

  it("shows an empty-state message when there are no repositories", async () => {
    mockedListRepos.mockResolvedValue({ repos: [] });
    render(<RepoList />);

    await expect.element(page.getByText("No repositories found.")).toBeVisible();
  });

  it("shows an error message when the request fails", async () => {
    mockedListRepos.mockRejectedValue(new ApiError("internal", "boom", 500));
    render(<RepoList />);

    await expect.element(page.getByRole("alert")).toHaveTextContent("boom");
  });
});
