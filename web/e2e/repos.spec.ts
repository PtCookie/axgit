import { expect, test } from "@playwright/test";

import fixture from "../tests/fixtures/repos.json" with { type: "json" };

test("shows repositories grouped by section, unsectioned first then most recently active", async ({ page }) => {
  await page.route("**/api/v1/repos", async (route) => {
    await route.fulfill({ json: fixture });
  });

  await page.goto("/");

  await expect(page.getByRole("heading", { name: "infra", level: 2 })).toBeVisible();
  await expect(page.getByText("git-compose")).toBeVisible();
  // The unsectioned group (scratch) has no visible label — its `sr-only` heading is present for
  // screen readers only — but it's still the first section landmark on the page.
  const headings = page.getByRole("heading", { level: 2 });
  await expect(headings.first()).toHaveText("Uncategorized");
  await expect(headings.first()).toHaveClass(/sr-only/);
});

test("filters the repository list and syncs the query into the URL", async ({ page }) => {
  await page.route("**/api/v1/repos", async (route) => {
    await route.fulfill({ json: fixture });
  });

  await page.goto("/");
  await expect(page.getByRole("link", { name: "dotfiles", exact: true })).toBeVisible();

  await page.getByRole("searchbox", { name: "Filter repositories" }).fill("dotfiles");

  await expect(page.getByRole("link", { name: "dotfiles", exact: true })).toBeVisible();
  await expect(page.getByRole("link", { name: "git-compose", exact: true })).not.toBeVisible();
  await expect(page).toHaveURL(/\?q=dotfiles$/);
});

test("deep-links a filtered list from ?q=", async ({ page }) => {
  await page.route("**/api/v1/repos", async (route) => {
    await route.fulfill({ json: fixture });
  });

  await page.goto("/?q=axgit");

  await expect(page.getByRole("searchbox", { name: "Filter repositories" })).toHaveValue("axgit");
  await expect(page.getByRole("link", { name: "axgit", exact: true })).toBeVisible();
  await expect(page.getByRole("link", { name: "git-compose", exact: true })).not.toBeVisible();
});

test("follows a row's Tree quick link into the repository's tree page", async ({ page }) => {
  await page.route("**/api/v1/repos", async (route) => {
    await route.fulfill({ json: fixture });
  });
  await page.route("**/api/v1/repos/git-compose/tree/HEAD", async (route) => {
    await route.fulfill({ json: { sha: "abc123def456abc123def456abc123def456abc", path: "", entries: [] } });
  });

  await page.goto("/");
  await page.getByRole("link", { name: "Tree for git-compose" }).click();

  await expect(page).toHaveURL("/git-compose/tree");
});

test("gives a row with a configured homepage a Homepage quick link", async ({ page }) => {
  await page.route("**/api/v1/repos", async (route) => {
    await route.fulfill({ json: fixture });
  });

  await page.goto("/");

  const link = page.getByRole("link", { name: "Homepage for git-compose" });
  await expect(link).toHaveAttribute("href", "https://git.ptcookie.net/git-compose");
  await expect(link).toHaveAttribute("target", "_blank");
  await expect(link).toHaveAttribute("rel", "noopener noreferrer");
  // axgit has no homepage in the fixture.
  await expect(page.getByRole("link", { name: "Homepage for axgit" })).not.toBeAttached();
});

test("clicking a column header re-sorts and writes ?sort= to the URL", async ({ page }) => {
  await page.route("**/api/v1/repos", async (route) => {
    await route.fulfill({ json: fixture });
  });

  await page.goto("/");
  await expect(page.getByRole("link", { name: "git-compose", exact: true })).toBeVisible();

  // Duplicate "Owner" headers exist, one per section table — any of them
  // drives the same shared sort state, so `.first()` is enough.
  await page.getByRole("button", { name: "Owner" }).first().click();

  await expect(page).toHaveURL(/\?sort=owner$/);
  await expect(page.getByRole("columnheader", { name: "Owner" }).first()).toHaveAttribute("aria-sort", "ascending");
});

test("deep-links a sorted list from ?sort=", async ({ page }) => {
  await page.route("**/api/v1/repos", async (route) => {
    await route.fulfill({ json: fixture });
  });

  await page.goto("/?sort=-idle");

  await expect(page.getByRole("columnheader", { name: "Last activity" }).first()).toHaveAttribute(
    "aria-sort",
    "ascending",
  );
  // Group order is fixed (unsectioned first, then most recently active first): scratch leads,
  // then tools (dotfiles, 2026-08-05) ahead of infra (axgit, 2026-07-30), regardless of the active
  // sort. "-idle" flips idle's descending default to ascending within each group — visible here
  // in infra, where git-compose (2026-07-24) now precedes axgit (2026-07-30).
  const names = await page.locator("table tbody a").allTextContents();
  expect(names.filter((name) => name.trim() !== "")).toEqual(["scratch", "dotfiles", "git-compose", "axgit"]);
});

// robots.txt is a real static file (`web/public/robots.txt`), served ahead of
// the page-shell fallback (`api/src/routes.rs`) — steers crawlers away from
// the expensive, scan-budgeted endpoints (search/stats/blame/diff, DECISIONS
// #38's `X-Robots-Tag` precedent generalized here).
test("serves a robots.txt disallowing the expensive endpoints", async ({ page }) => {
  const response = await page.request.get("/robots.txt");
  expect(response.status()).toBe(200);

  const body = await response.text();
  expect(body).toContain("Disallow: /api/v1/");
  expect(body).toContain("Disallow: /*/search");
  // The object graph is walkable link-by-link — cheap per request, but not
  // something to hand a crawler (docs/DECISIONS.md #53).
  expect(body).toContain("Disallow: /*/object/");
});
