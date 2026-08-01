import { expect, test } from "@playwright/test";

const SUMMARY = {
  name: "git-compose",
  section: "infra",
  owner: "PtCookie",
  description: "Compose project of Git server",
  default_branch: "main",
  last_modified: "2026-07-24T13:06:00+09:00",
  head: "abc123def456",
  branch_count: 1,
  tag_count: 1,
  clone_url: "git@git.ptcookie.net:git-compose.git",
};

const REFS = {
  branches: [{ name: "main", target: "abc123def456", committed_at: "2026-07-24T13:06:00+09:00" }],
  tags: [
    { name: "v1.0.0", target: "def456abc123", annotation: "First release", tagged_at: "2026-01-01T00:00:00+09:00" },
  ],
};

test("shows the repository summary and links to refs", async ({ page }) => {
  await page.route("**/api/v1/repos/git-compose", async (route) => {
    await route.fulfill({ json: SUMMARY });
  });

  await page.goto("/git-compose");

  await expect(page.getByRole("heading", { name: "git-compose" })).toBeVisible();
  await expect(page.getByText("Compose project of Git server")).toBeVisible();

  await page.route("**/api/v1/repos/git-compose/refs", async (route) => {
    await route.fulfill({ json: REFS });
  });
  await page.getByRole("link", { name: "Refs" }).click();

  await expect(page).toHaveURL("/git-compose/refs");
  await expect(page.getByText("v1.0.0")).toBeVisible();
});

test("shows a not-found page for unsupported sub-routes", async ({ page }) => {
  await page.goto("/git-compose/log");

  await expect(page.getByRole("heading", { name: "Page not found" })).toBeVisible();
  await page.getByRole("link", { name: "Back to repository list" }).click();
  await expect(page).toHaveURL("/");
});
