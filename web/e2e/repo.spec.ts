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

const COMMIT_SHA = "abc123def456abc123def456abc123def456abc";

const COMMITS_PAGE = {
  commits: [
    {
      sha: COMMIT_SHA,
      summary: "fix: update the readme",
      author: { name: "Ada Lovelace", email_hash: "deadbeef" },
      authored_at: "2026-07-24T13:06:00+09:00",
      parents: [],
    },
  ],
  next_cursor: null,
};

const COMMIT_DETAIL = {
  sha: COMMIT_SHA,
  summary: "fix: update the readme",
  message: "fix: update the readme\n",
  author: { name: "Ada Lovelace", email_hash: "deadbeef" },
  committer: { name: "Ada Lovelace", email_hash: "deadbeef" },
  authored_at: "2026-07-24T13:06:00+09:00",
  committed_at: "2026-07-24T13:06:00+09:00",
  parents: [],
  diffstat: {
    files: [{ path: "README.md", old_path: null, status: "modified", additions: 2, deletions: 1, binary: false }],
    files_changed: 1,
    total_additions: 2,
    total_deletions: 1,
  },
};

const COMMIT_DIFF = {
  sha: COMMIT_SHA,
  parent: null,
  truncated: false,
  files: [
    {
      path: "README.md",
      old_path: null,
      status: "modified",
      additions: 2,
      deletions: 1,
      binary: false,
      truncated: false,
      hunks: [
        {
          header: "@@ -1,1 +1,2 @@",
          old_start: 1,
          old_lines: 1,
          new_start: 1,
          new_lines: 2,
          lines: [
            { origin: "-", content: "# Old", old_lineno: 1, new_lineno: null },
            { origin: "+", content: "# New", old_lineno: null, new_lineno: 1 },
            { origin: "+", content: "More text", old_lineno: null, new_lineno: 2 },
          ],
        },
      ],
    },
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

test("navigates from the log to a commit's detail", async ({ page }) => {
  await page.route("**/api/v1/repos/git-compose", async (route) => {
    await route.fulfill({ json: SUMMARY });
  });
  await page.route("**/api/v1/repos/git-compose/commits", async (route) => {
    await route.fulfill({ json: COMMITS_PAGE });
  });

  await page.goto("/git-compose");
  await page.getByRole("link", { name: "Log" }).click();

  await expect(page).toHaveURL("/git-compose/log");
  await expect(page.getByText("fix: update the readme")).toBeVisible();

  await page.route(`**/api/v1/repos/git-compose/commits/${COMMIT_SHA}`, async (route) => {
    await route.fulfill({ json: COMMIT_DETAIL });
  });
  await page.route(`**/api/v1/repos/git-compose/commits/${COMMIT_SHA}/diff`, async (route) => {
    await route.fulfill({ json: COMMIT_DIFF });
  });
  await page.getByRole("link", { name: "fix: update the readme" }).click();

  await expect(page).toHaveURL(`/git-compose/commit/${COMMIT_SHA}`);
  await expect(page.getByRole("heading", { name: "fix: update the readme" })).toBeVisible();
  await expect(page.getByText("More text")).toBeVisible();
});

const TREE_ROOT = {
  sha: COMMIT_SHA,
  path: "",
  entries: [
    { name: "src", type: "tree", mode: "040000", size: null },
    { name: "README.md", type: "blob", mode: "100644", size: 16 },
  ],
};

const TREE_SRC = {
  sha: COMMIT_SHA,
  path: "src",
  entries: [{ name: "main.rs", type: "blob", mode: "100644", size: 42 }],
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

const BLAME_MAIN = {
  sha: COMMIT_SHA,
  path: "src/main.rs",
  binary: false,
  too_large: false,
  lines: 3,
  ranges: [
    {
      start_line: 1,
      line_count: 3,
      sha: COMMIT_SHA,
      summary: "fix: update the readme",
      author: { name: "Ada Lovelace", email_hash: "deadbeef" },
      authored_at: "2026-07-24T13:06:00+09:00",
    },
  ],
};

test("navigates from the tree into a subdirectory and a file", async ({ page }) => {
  await page.route("**/api/v1/repos/git-compose", async (route) => {
    await route.fulfill({ json: SUMMARY });
  });
  await page.route("**/api/v1/repos/git-compose/tree/HEAD", async (route) => {
    await route.fulfill({ json: TREE_ROOT });
  });

  await page.goto("/git-compose");
  await page.getByRole("link", { name: "Tree" }).click();

  await expect(page).toHaveURL("/git-compose/tree");
  await expect(page.getByRole("link", { name: "src/" })).toBeVisible();

  await page.route("**/api/v1/repos/git-compose/tree/HEAD/src", async (route) => {
    await route.fulfill({ json: TREE_SRC });
  });
  await page.getByRole("link", { name: "src/" }).click();

  await expect(page).toHaveURL("/git-compose/tree/src");
  await expect(page.getByRole("link", { name: "main.rs" })).toBeVisible();

  await page.route("**/api/v1/repos/git-compose/blob/HEAD/src/main.rs", async (route) => {
    await route.fulfill({ json: BLOB_MAIN });
  });
  await page.getByRole("link", { name: "main.rs" }).click();

  await expect(page).toHaveURL("/git-compose/blob/src/main.rs");
  await expect(page.getByText('println!("hi");')).toBeVisible();

  await page.route("**/api/v1/repos/git-compose/blame/HEAD/src/main.rs", async (route) => {
    await route.fulfill({ json: BLAME_MAIN });
  });
  await page.getByRole("link", { name: "Blame" }).click();

  await expect(page).toHaveURL("/git-compose/blame/src/main.rs");
  await expect(page.getByText('println!("hi");')).toBeVisible();
  await expect(page.getByText("Ada Lovelace")).toBeVisible();
});

test("shows a not-found page for unsupported sub-routes", async ({ page }) => {
  await page.goto("/git-compose/stats");

  await expect(page.getByRole("heading", { name: "Page not found" })).toBeVisible();
  await page.getByRole("link", { name: "Back to repository list" }).click();
  await expect(page).toHaveURL("/");
});
