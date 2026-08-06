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

const TAG_SHA = "abc123def456abc123def456abc123def456abc";

const REFS = {
  branches: [{ name: "main", target: TAG_SHA, committed_at: "2026-07-24T13:06:00+09:00" }],
  remote_branches: [],
  tags: [{ name: "v1.0.0", target: TAG_SHA, annotation: "First release", tagged_at: "2026-01-01T00:00:00+09:00" }],
};

const TAG_DETAIL = {
  name: "v1.0.0",
  tag_object: "tagobject0000000000000000000000000000000",
  object: { sha: TAG_SHA, type: "commit" },
  target: TAG_SHA,
  message: "Release v1.0.0\n\nFirst stable cut.",
  tagger: { name: "PtCookie", email_hash: "deadbeef" },
  tagged_at: "2026-01-01T00:00:00+09:00",
};

// Exercises the route/shell mapping and <ClientRouter /> navigation end to
// end — the component test (`TagView.test.tsx`) already covers rendering in
// isolation, but only e2e proves `shellFor`/`shell_for`'s tag arm and the
// refs page's link actually connect.
test("clicking a tag name on the refs page navigates to its tag detail page", async ({ page }) => {
  await page.route("**/api/v1/repos/git-compose", async (route) => {
    await route.fulfill({ json: SUMMARY });
  });
  await page.route("**/api/v1/repos/git-compose/refs", async (route) => {
    await route.fulfill({ json: REFS });
  });
  await page.route("**/api/v1/repos/git-compose/tags/v1.0.0", async (route) => {
    await route.fulfill({ json: TAG_DETAIL });
  });

  await page.goto("/git-compose/refs");

  await page.getByRole("link", { name: "v1.0.0", exact: true }).click();

  await expect(page).toHaveURL("/git-compose/tag/v1.0.0");
  await expect(page.getByRole("heading", { name: "v1.0.0" })).toBeVisible();
  await expect(page.getByText("PtCookie")).toBeVisible();
  await expect(page.getByText(/Release v1\.0\.0/)).toBeVisible();
  await expect(page.getByText(/First stable cut\./)).toBeVisible();
});
