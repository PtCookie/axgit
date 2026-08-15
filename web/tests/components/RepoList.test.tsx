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

/** Repository name links, in document order across every section's table —
 *  `IconLink`s render in the same cell but carry no text content, so a plain
 *  text filter isolates the name column without needing to scope per
 *  section. Reads from `render()`'s own `container`, not the global
 *  `document` — browser mode runs each test in its own frame. */
function visibleRepoOrder(container: HTMLElement): string[] {
  return [...container.querySelectorAll("table tbody a")]
    .map((a) => a.textContent?.trim() ?? "")
    .filter((text) => text !== "");
}

describe("RepoList", () => {
  beforeEach(() => {
    mockedListRepos.mockReset();
  });

  afterEach(async () => {
    await cleanup();
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
    const link = page.getByRole("link", { name: "git-compose", exact: true });
    await expect.element(link).toBeVisible();
    await expect.element(link).toHaveAttribute("href", "/git-compose");
  });

  it("gives each row a Log and Tree quick link (enable-index-links parity)", async () => {
    mockedListRepos.mockResolvedValue(fixture);
    render(<RepoList />);

    await expect
      .element(page.getByRole("link", { name: "Log for git-compose" }))
      .toHaveAttribute("href", "/git-compose/log");
    await expect
      .element(page.getByRole("link", { name: "Tree for git-compose" }))
      .toHaveAttribute("href", "/git-compose/tree");
  });

  it("gives a row a Homepage quick link only when the repo has one configured", async () => {
    mockedListRepos.mockResolvedValue(fixture);
    render(<RepoList />);

    const homepage = page.getByRole("link", { name: "Homepage for git-compose" });
    await expect.element(homepage).toHaveAttribute("href", "https://git.ptcookie.net/git-compose");
    await expect.element(homepage).toHaveAttribute("target", "_blank");
    await expect.element(homepage).toHaveAttribute("rel", "noopener noreferrer");

    // axgit has no homepage in the fixture.
    expect(page.getByRole("link", { name: "Homepage for axgit" }).elements().length).toBe(0);
  });

  it("shows an empty-state message when there are no repositories", async () => {
    mockedListRepos.mockResolvedValue({ repos: [], sort: "name" });
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

    await expect.element(page.getByRole("link", { name: "dotfiles", exact: true })).toBeVisible();
    await expect.element(page.getByRole("link", { name: "git-compose", exact: true })).not.toBeInTheDocument();
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
    await expect.element(page.getByRole("link", { name: "git-compose", exact: true })).not.toBeInTheDocument();
  });

  it("marks the active column via aria-sort, matching the API's own order", async () => {
    mockedListRepos.mockResolvedValue(fixture);
    await render(<RepoList />);

    await expect
      .element(page.getByRole("columnheader", { name: "Name" }).first())
      .toHaveAttribute("aria-sort", "ascending");
    await expect
      .element(page.getByRole("columnheader", { name: "Owner" }).first())
      .toHaveAttribute("aria-sort", "none");
  });

  it("clicking a column header re-sorts client-side and writes ?sort= to the URL", async () => {
    mockedListRepos.mockResolvedValue(fixture);
    const { container } = await render(<RepoList />);
    await expect.element(page.getByText("git-compose")).toBeVisible();

    expect(visibleRepoOrder(container)).toEqual(["git-compose", "axgit", "dotfiles", "scratch"]);

    await userEvent.click(page.getByRole("button", { name: "Owner" }).first());

    expect(new URLSearchParams(window.location.search).get("sort")).toBe("owner");
    // Owner ascending: both "PtCookie" repos (tie broken by name: axgit
    // before git-compose), then both `null`-owner repos (dotfiles, scratch),
    // `null` always last regardless of direction.
    expect(visibleRepoOrder(container)).toEqual(["axgit", "git-compose", "dotfiles", "scratch"]);
    await expect
      .element(page.getByRole("columnheader", { name: "Owner" }).first())
      .toHaveAttribute("aria-sort", "ascending");
  });

  it("deep-links a sorted list from an existing ?sort=", async () => {
    window.history.replaceState(null, "", "?sort=-idle");
    mockedListRepos.mockResolvedValue(fixture);
    const { container } = await render(<RepoList />);
    await expect.element(page.getByText("git-compose")).toBeVisible();

    // "-idle" flips idle's own descending default to ascending (oldest
    // last_modified first); `null` (scratch) still sorts last.
    expect(visibleRepoOrder(container)).toEqual(["dotfiles", "git-compose", "axgit", "scratch"]);
    await expect
      .element(page.getByRole("columnheader", { name: "Last activity" }).first())
      .toHaveAttribute("aria-sort", "ascending");
  });
});
