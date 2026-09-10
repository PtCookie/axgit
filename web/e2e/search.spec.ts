import { expect, test } from "./fixtures";

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

const NO_README = { error: { code: "path_not_found", message: "no readme found" } };

const COMMIT_SHA = "abc123def456abc123def456abc123def456abc";

const SEARCH_CONTENT_RESULTS = {
  sha: COMMIT_SHA,
  type: "content",
  truncated: false,
  files: [
    {
      path: "src/main.rs",
      lines: [{ line: 2, text: 'println!("hi");' }],
    },
  ],
  commits: [],
};

const BLOB_MAIN = {
  sha: COMMIT_SHA,
  path: "src/main.rs",
  mode: "100644",
  size: 42,
  binary: false,
  too_large: false,
  content: 'fn main() {\n    println!("hi");\n}\n',
};

test("searches a repository and follows a result into the blob view", async ({ page }) => {
  await page.route("**/api/v1/repos/git-compose", async (route) => {
    await route.fulfill({ json: SUMMARY });
  });
  await page.route("**/api/v1/repos/git-compose/readme*", async (route) => {
    await route.fulfill({ status: 404, json: NO_README });
  });

  await page.goto("/git-compose");
  await page.getByRole("link", { name: "Search" }).click();

  await expect(page).toHaveURL("/git-compose/search");
  await expect(page.getByText("Enter a search query above.")).toBeVisible();

  await page.route("**/api/v1/repos/git-compose/search*", async (route) => {
    await route.fulfill({ json: SEARCH_CONTENT_RESULTS });
  });

  await page.getByRole("searchbox", { name: "Search query" }).fill("hi");
  await page.getByRole("button", { name: "Search" }).click();

  await expect(page).toHaveURL(/\/git-compose\/search\?q=hi&type=content/);
  await expect(page.getByRole("link", { name: "src/main.rs" })).toBeVisible();

  await page.route("**/api/v1/repos/git-compose/blob/HEAD/src/main.rs", async (route) => {
    await route.fulfill({ json: BLOB_MAIN });
  });
  await page.getByRole("link", { name: /2:/ }).click();

  await expect(page).toHaveURL("/git-compose/blob/src/main.rs#L2");
  await expect(page.getByText('println!("hi");')).toBeVisible();
});

test("shows a no-matches message and a truncation notice", async ({ page }) => {
  await page.route("**/api/v1/repos/git-compose", async (route) => {
    await route.fulfill({ json: SUMMARY });
  });
  await page.route("**/api/v1/repos/git-compose/readme*", async (route) => {
    await route.fulfill({ status: 404, json: NO_README });
  });
  await page.route("**/api/v1/repos/git-compose/search*", async (route) => {
    await route.fulfill({
      json: { sha: COMMIT_SHA, type: "path", truncated: true, files: [], commits: [] },
    });
  });

  await page.goto("/git-compose/search?q=nope&type=path");

  await expect(page.getByText("No matches found.")).toBeVisible();
  await expect(page.getByRole("status")).toHaveText(/partial result/);
});
