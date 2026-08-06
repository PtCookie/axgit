import { expect, test } from "@playwright/test";

import fixture from "../tests/fixtures/repos.json" with { type: "json" };

test("shows repositories grouped by section", async ({ page }) => {
  await page.route("**/api/v1/repos", async (route) => {
    await route.fulfill({ json: fixture });
  });

  await page.goto("/");

  await expect(page.getByRole("heading", { name: "infra", level: 2 })).toBeVisible();
  await expect(page.getByText("git-compose")).toBeVisible();
  await expect(page.getByRole("heading", { name: "Other", level: 2 })).toBeVisible();
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
});
