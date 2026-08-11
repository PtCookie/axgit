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

const README = {
  path: "README.md",
  format: "markdown",
  // The heading text deliberately differs from the repo name ("git-compose")
  // — that text is already used by the page's own `<h1>` (`RepoNav.astro`),
  // and re-using it here would make `getByRole("heading")` ambiguous.
  content: "# Getting started\n\nSee the [docs](./docs/setup.md) for setup.\n\n```sh\necho hello\n```\n",
};

const NO_README = { error: { code: "path_not_found", message: "no readme found" } };

const REFS = {
  branches: [{ name: "main", target: "abc123def456", committed_at: "2026-07-24T13:06:00+09:00" }],
  remote_branches: [],
  tags: [
    {
      name: "v1.0.0",
      object: { sha: "def456abc123", type: "commit" },
      target: "def456abc123",
      annotation: "First release",
      tagged_at: "2026-01-01T00:00:00+09:00",
    },
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
  note: null,
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

test("shows the repository summary, README, and links to refs", async ({ page }) => {
  await page.route("**/api/v1/repos/git-compose", async (route) => {
    await route.fulfill({ json: SUMMARY });
  });
  await page.route("**/api/v1/repos/git-compose/readme*", async (route) => {
    await route.fulfill({ json: README });
  });

  await page.goto("/git-compose");

  await expect(page.getByRole("heading", { name: "git-compose" })).toBeVisible();
  await expect(page.getByText("Compose project of Git server")).toBeVisible();
  await expect(page.getByRole("link", { name: "tar.gz" })).toHaveAttribute(
    "href",
    "/api/v1/repos/git-compose/archive/HEAD.tar.gz",
  );
  await expect(page.getByRole("link", { name: "tar.zst" })).toHaveAttribute(
    "href",
    "/api/v1/repos/git-compose/archive/HEAD.tar.zst",
  );
  await expect(page.getByRole("link", { name: "Atom" })).toHaveAttribute("href", "/api/v1/repos/git-compose/feed.atom");

  // The README is the wide left column; its relative link is rewritten to
  // the repository's blob page and its code fence is highlighted.
  await expect(page.getByRole("heading", { name: "Getting started", level: 1 })).toBeVisible();
  const docsLink = page.getByRole("link", { name: "docs" });
  await expect(docsLink).toHaveAttribute("href", "/git-compose/blob/docs/setup.md");
  await expect(page.getByText("echo hello")).toBeVisible();

  // The details block is a right-hand sidebar at `lg` and up (all three
  // desktop projects share a 1280px viewport), while the DOM order stays
  // details-then-README
  // — `flex-row-reverse` in `pages/[repo]/index.astro`. Compared by
  // bounding box, since that ordering is purely a CSS outcome and nothing in
  // the markup would catch a regression to a single column.
  const details = page.getByRole("complementary", { name: "Repository details" });
  const detailsBox = await details.boundingBox();
  const readmeBox = await page.getByRole("heading", { name: "Getting started", level: 1 }).boundingBox();
  if (!detailsBox || !readmeBox) {
    throw new Error("expected both the details sidebar and the README heading to have a bounding box");
  }
  expect(detailsBox.x).toBeGreaterThan(readmeBox.x);

  await expect(page.getByRole("link", { name: "1 branch" })).toHaveAttribute("href", "/git-compose/refs");

  await page.route("**/api/v1/repos/git-compose/refs", async (route) => {
    await route.fulfill({ json: REFS });
  });
  await page.getByRole("link", { name: "Refs" }).click();

  await expect(page).toHaveURL("/git-compose/refs");
  // Now a link to the tag page (this commit) — `exact: true` disambiguates
  // from "Compare v1.0.0 with main" and "Download v1.0.0 as tar.gz", both of
  // which contain this string too.
  await expect(page.getByRole("link", { name: "v1.0.0", exact: true })).toBeVisible();
});

test("navigates from the log to a commit's detail, showing its ref badge on both pages", async ({ page }) => {
  await page.route("**/api/v1/repos/git-compose", async (route) => {
    await route.fulfill({ json: SUMMARY });
  });
  await page.route("**/api/v1/repos/git-compose/readme*", async (route) => {
    await route.fulfill({ status: 404, json: NO_README });
  });
  await page.route("**/api/v1/repos/git-compose/commits", async (route) => {
    await route.fulfill({ json: COMMITS_PAGE });
  });
  // The branch tip matches the log's one commit, so its ref badge should
  // show up on both the log row and the commit detail page below.
  await page.route("**/api/v1/repos/git-compose/refs", async (route) => {
    await route.fulfill({
      json: { branches: [{ name: "main", target: COMMIT_SHA, committed_at: null }], remote_branches: [], tags: [] },
    });
  });

  await page.goto("/git-compose");
  await page.getByRole("link", { name: "Log" }).click();

  await expect(page).toHaveURL("/git-compose/log");
  await expect(page.getByText("fix: update the readme")).toBeVisible();
  await expect(page.getByRole("link", { name: "main" })).toHaveAttribute("href", "/git-compose/log?ref=main");

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
  await expect(page.getByRole("link", { name: "main" })).toHaveAttribute("href", "/git-compose/log?ref=main");
  await expect(page.getByRole("link", { name: "tar.gz" })).toHaveAttribute(
    "href",
    `/api/v1/repos/git-compose/archive/${COMMIT_SHA}.tar.gz`,
  );
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
  await page.route("**/api/v1/repos/git-compose/readme*", async (route) => {
    await route.fulfill({ status: 404, json: NO_README });
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
  await expect(page.getByRole("link", { name: "main.rs", exact: true })).toBeVisible();
  await expect(page.getByRole("link", { name: "Raw for main.rs" })).toHaveAttribute(
    "href",
    "/api/v1/repos/git-compose/raw/HEAD/src/main.rs",
  );
  await expect(page.getByRole("link", { name: "Blame for main.rs" })).toHaveAttribute(
    "href",
    "/git-compose/blame/src/main.rs",
  );

  await page.route("**/api/v1/repos/git-compose/blob/HEAD/src/main.rs", async (route) => {
    await route.fulfill({ json: BLOB_MAIN });
  });
  await page.getByRole("link", { name: "main.rs", exact: true }).click();

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
  // `/{repo}/blob` with no path segment is a structurally unmatched shape
  // (`lib/shell.ts::shellFor`) — there's nothing to show for a bare blob URL.
  await page.goto("/git-compose/blob");

  await expect(page.getByRole("heading", { name: "Page not found" })).toBeVisible();
  await page.getByRole("link", { name: "Back to repository list" }).click();
  await expect(page).toHaveURL("/");
});

// A link click navigating via `<ClientRouter />` (docs/DECISIONS.md #24) never
// tears down the JS realm — `window` survives. A silently downgraded full
// reload (e.g. the dev shell-fallback middleware not recognizing the
// router's fetch, or a non-2xx response) would still land on the same URL
// and pass every other assertion in this file, so this marker is the only
// signal that a test is actually exercising client-side routing.
test("tab navigation is client-side, not a full page reload", async ({ page }) => {
  await page.route("**/api/v1/repos/git-compose", async (route) => {
    await route.fulfill({ json: SUMMARY });
  });
  await page.route("**/api/v1/repos/git-compose/readme*", async (route) => {
    await route.fulfill({ status: 404, json: NO_README });
  });
  await page.route("**/api/v1/repos/git-compose/refs", async (route) => {
    await route.fulfill({ json: REFS });
  });

  await page.goto("/git-compose");
  await page.evaluate(() => {
    (window as unknown as { __navMarker?: number }).__navMarker = 1;
  });

  await page.getByRole("link", { name: "Refs" }).click();

  await expect(page).toHaveURL("/git-compose/refs");
  // Now a link to the tag page (this commit) — `exact: true` disambiguates
  // from "Compare v1.0.0 with main" and "Download v1.0.0 as tar.gz", both of
  // which contain this string too.
  await expect(page.getByRole("link", { name: "v1.0.0", exact: true })).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as { __navMarker?: number }).__navMarker)).toBe(1);
});
