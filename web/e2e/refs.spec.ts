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

const TAG_SHA = "abc123def456abc123def456abc123def456abc";
const BLOB_TAG_SHA = "deadbeef00000000000000000000000000000000";

const REFS = {
  branches: [{ name: "main", target: TAG_SHA, committed_at: "2026-07-24T13:06:00+09:00" }],
  remote_branches: [],
  tags: [
    {
      name: "v1.0.0",
      object: { sha: TAG_SHA, type: "commit" },
      target: TAG_SHA,
      annotation: "First release",
      tagged_at: "2026-01-01T00:00:00+09:00",
    },
    {
      name: "blob-tag",
      object: { sha: BLOB_TAG_SHA, type: "blob" },
      target: null,
      annotation: null,
      tagged_at: null,
    },
  ],
};

const OBJECT_DETAIL = {
  sha: BLOB_TAG_SHA,
  type: "blob",
  tree: null,
  blob: { size: 5, binary: false, too_large: false, content: "hello" },
  tag: null,
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

// Proves `shellFor`/`shell_for`'s object arm and the refs page's non-commit
// tag link actually connect end to end — closing the "Tags and refs"
// cgit-parity gap (docs/DECISIONS.md #53): a tag whose target isn't a
// commit used to render as inert text, with nowhere to go.
test("clicking a non-commit tag's object link on the refs page navigates to the object page", async ({ page }) => {
  await page.route("**/api/v1/repos/git-compose", async (route) => {
    await route.fulfill({ json: SUMMARY });
  });
  await page.route("**/api/v1/repos/git-compose/refs", async (route) => {
    await route.fulfill({ json: REFS });
  });
  await page.route(`**/api/v1/repos/git-compose/objects/${BLOB_TAG_SHA}`, async (route) => {
    await route.fulfill({ json: OBJECT_DETAIL });
  });

  await page.goto("/git-compose/refs");

  await page.getByRole("link", { name: BLOB_TAG_SHA.slice(0, 12) }).click();

  await expect(page).toHaveURL(`/git-compose/object/${BLOB_TAG_SHA}`);
  await expect(page.getByText(BLOB_TAG_SHA)).toBeVisible();
  await expect(page.getByText("hello")).toBeVisible();
});
