import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "vitest-browser-react";
import { page, userEvent } from "vitest/browser";

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
    // Each mount can leave `?q=` behind via `history.replaceState` — reset it
    // so later tests (and other files sharing this page) get a clean URL.
    window.history.replaceState(null, "", window.location.pathname);
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

  it("filters rows by the typed query and mirrors it into the URL's ?q=", async () => {
    mockedListRepos.mockResolvedValue(fixture);
    render(<RepoList />);

    const input = page.getByRole("searchbox", { name: "Filter repositories" });
    await expect.element(input).toBeVisible();
    await userEvent.type(input, "dotfiles");

    await expect.element(page.getByRole("link", { name: "dotfiles" })).toBeVisible();
    await expect.element(page.getByRole("link", { name: "git-compose" })).not.toBeInTheDocument();
    await expect.element(page.getByRole("status")).toHaveTextContent("1 of 4 repositories");
    expect(new URLSearchParams(window.location.search).get("q")).toBe("dotfiles");
  });

  it("shows a no-match message instead of an empty table when nothing matches", async () => {
    mockedListRepos.mockResolvedValue(fixture);
    render(<RepoList />);

    const input = page.getByRole("searchbox", { name: "Filter repositories" });
    await userEvent.type(input, "does-not-exist");

    await expect.element(page.getByText('No repositories match "does-not-exist".')).toBeVisible();
  });

  it("prefills the query and the filtered list from an existing ?q=", async () => {
    window.history.replaceState(null, "", "?q=axgit");
    mockedListRepos.mockResolvedValue(fixture);
    render(<RepoList />);

    const input = page.getByRole("searchbox", { name: "Filter repositories" });
    await expect.element(input).toHaveValue("axgit");
    await expect.element(page.getByRole("link", { name: "axgit", exact: true })).toBeVisible();
    await expect.element(page.getByRole("link", { name: "git-compose" })).not.toBeInTheDocument();
  });
});
