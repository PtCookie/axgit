import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "vitest-browser-react";
import { page } from "vitest/browser";

import ReadmeView from "@/components/repo/ReadmeView";
import { ApiError } from "@/lib/api/client";
import { getReadme, rawUrl } from "@/lib/api/repos";
import type { ReadmeInfo } from "@/lib/api/schemas";

vi.mock("@/lib/api/repos", () => ({
  getReadme: vi.fn(),
  // Echoes `path` so each test can tell which file a rewritten `src`/`srcset`
  // points at.
  rawUrl: vi.fn((repo: string, _ref: string | undefined, path: string) => `/api/v1/repos/${repo}/raw/HEAD/${path}`),
}));

const mockedGetReadme = vi.mocked(getReadme);

const MARKDOWN_README: ReadmeInfo = {
  path: "README.md",
  format: "markdown",
  content: "# Axgit\n\nA [docs link](./docs/x.md) and an image:\n\n![logo](images/logo.png)\n",
};

describe("ReadmeView", () => {
  beforeEach(() => {
    mockedGetReadme.mockReset();
    vi.mocked(rawUrl).mockClear();
  });

  afterEach(cleanup);

  it("renders sanitized markdown with a heading and a link", async () => {
    mockedGetReadme.mockResolvedValue(MARKDOWN_README);
    await render(<ReadmeView repo="git-compose" />);

    // `ReadmeMarkdown` is lazy-loaded (`ReadmeBody.tsx`), and browser-mode
    // test isolation means this chunk is fetched fresh on every test — under
    // CI load that fetch can outrun the default 1s retry timeout, failing
    // the assertion here and orphaning the still-in-flight dynamic import as
    // an unhandled rejection. Same class of flake as the Shiki wait below.
    await expect.element(page.getByRole("heading", { name: "Axgit", level: 1 }), { timeout: 5000 }).toBeVisible();
    await expect.element(page.getByText("README.md")).toBeVisible();
  });

  it("rewrites a relative link to the repo's blob page", async () => {
    mockedGetReadme.mockResolvedValue(MARKDOWN_README);
    await render(<ReadmeView repo="git-compose" />);

    const link = page.getByRole("link", { name: "docs link" });
    // See the timeout note above — this is the first assertion to wait on
    // the lazy-loaded `ReadmeMarkdown` chunk in this test.
    await expect.element(link, { timeout: 5000 }).toHaveAttribute("href", "/git-compose/blob/docs/x.md");
  });

  it("rewrites a relative image src through the raw endpoint", async () => {
    mockedGetReadme.mockResolvedValue(MARKDOWN_README);
    await render(<ReadmeView repo="git-compose" />);

    const img = page.getByRole("img", { name: "logo" });
    // See the timeout note above — this is the first assertion to wait on
    // the lazy-loaded `ReadmeMarkdown` chunk in this test.
    await expect
      .element(img, { timeout: 5000 })
      .toHaveAttribute("src", "/api/v1/repos/git-compose/raw/HEAD/images/logo.png");
  });

  it("renders a README's raw HTML header: centred paragraph, sized image through the raw endpoint", async () => {
    mockedGetReadme.mockResolvedValue({
      path: "README.md",
      format: "markdown",
      content: '<p align="center">\n  <img src="docs/header.svg" alt="header" width="960">\n</p>\n\n# Patina\n',
    });
    await render(<ReadmeView repo="patina" />);

    const img = page.getByRole("img", { name: "header" });
    // See the timeout note above.
    await expect
      .element(img, { timeout: 5000 })
      .toHaveAttribute("src", "/api/v1/repos/patina/raw/HEAD/docs/header.svg");
    await expect.element(img).toHaveAttribute("width", "960");
    expect(img.element().closest("p")?.getAttribute("align")).toBe("center");
  });

  it("keeps a <picture>'s <source media> and rewrites its srcset", async () => {
    mockedGetReadme.mockResolvedValue({
      path: "README.md",
      format: "markdown",
      content:
        '<picture>\n  <source media="(prefers-color-scheme: dark)" srcset="docs/dark.png">\n  <img src="docs/light.png" alt="logo">\n</picture>\n',
    });
    const { container } = await render(<ReadmeView repo="git-compose" />);

    await expect
      .element(page.getByRole("img", { name: "logo" }), { timeout: 5000 })
      .toHaveAttribute("src", "/api/v1/repos/git-compose/raw/HEAD/docs/light.png");
    const source = container.querySelector("picture > source");
    expect(source?.getAttribute("media")).toBe("(prefers-color-scheme: dark)");
    expect(source?.getAttribute("srcset")).toBe("/api/v1/repos/git-compose/raw/HEAD/docs/dark.png");
  });

  it("renders GitHub-allowed HTML such as <details>/<summary> and <kbd>", async () => {
    mockedGetReadme.mockResolvedValue({
      path: "README.md",
      format: "markdown",
      content: "<details>\n<summary>More</summary>\n\nPress <kbd>Ctrl</kbd>.\n\n</details>\n",
    });
    const { container } = await render(<ReadmeView repo="git-compose" />);

    await expect.element(page.getByText("More"), { timeout: 5000 }).toBeVisible();
    expect(container.querySelector("details > summary")?.textContent).toBe("More");
    expect(container.querySelector("kbd")?.textContent).toBe("Ctrl");
  });

  it("strips scripts, event handlers, inline styles, iframes and unsafe URL schemes from raw HTML", async () => {
    mockedGetReadme.mockResolvedValue({
      path: "README.md",
      format: "markdown",
      content: [
        "# Hostile",
        "",
        "<script>window.__pwned = true</script>",
        '<img src="x.png" alt="evil" onerror="window.__pwned = true">',
        '<a href="javascript:window.__pwned = true">click</a>',
        '<p style="position:fixed">styled</p>',
        '<iframe src="https://example.com"></iframe>',
        '<picture><source srcset="javascript:alert(1)"><img src="y.png" alt="pic"></picture>',
        "",
      ].join("\n"),
    });
    const { container } = await render(<ReadmeView repo="git-compose" />);

    await expect.element(page.getByRole("heading", { name: "Hostile", level: 1 }), { timeout: 5000 }).toBeVisible();
    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("iframe")).toBeNull();
    expect(container.querySelector("[onerror]")).toBeNull();
    expect(container.querySelector("[style]")).toBeNull();
    expect(container.querySelector('a[href^="javascript"]')).toBeNull();
    expect(container.querySelector("source")?.hasAttribute("srcset")).toBe(false);
    expect((window as { __pwned?: boolean }).__pwned).toBeUndefined();
  });

  it("highlights a fenced code block via Shiki, keyed off the fence language", async () => {
    mockedGetReadme.mockResolvedValue({
      path: "README.md",
      format: "markdown",
      content: "# Axgit\n\n```rust\nfn main() {}\n```\n",
    });
    await render(<ReadmeView repo="git-compose" />);

    // `MarkdownFence` renders plain text first, then swaps in Shiki's
    // `<span style>` tokens once highlighting resolves — wait for that swap
    // rather than just the text, since a `pre`/`code` type-identity mixup
    // (react-markdown substitutes the `code` *component*, not a `"code"`
    // tag string) previously made every fence fall back to unstyled text.
    await expect.element(page.getByText("fn")).toBeVisible();
    // `vi.waitFor`'s default 1s timeout can be too tight in CI, where the
    // Shiki highlighter's cold start (several dynamic imports plus grammar
    // loading, all going through the browser's module transform pipeline)
    // competes with other test workers for CPU — bump it well past what a
    // slow, loaded runner needs.
    await vi.waitFor(
      () => {
        const span = document.querySelector("pre.shiki-code span");
        expect(span).not.toBeNull();
        expect(span?.getAttribute("style")).toBeTruthy();
      },
      { timeout: 5000 },
    );
  });

  it("renders plain-text formats (rst/plain) as preformatted text, not markdown", async () => {
    mockedGetReadme.mockResolvedValue({ path: "README", format: "plain", content: "just text\nno markup" });
    await render(<ReadmeView repo="git-compose" />);

    await expect.element(page.getByText("just text", { exact: false })).toBeVisible();
    // Only the path label (`README`) is a heading — no markdown-derived `h1`.
    expect(page.getByRole("heading", { level: 1 }).elements().length).toBe(0);
  });

  it("renders nothing, with no error, when the repository has no README", async () => {
    mockedGetReadme.mockRejectedValue(new ApiError("path_not_found", "no readme found", 404));
    const { container } = await render(<ReadmeView repo="git-compose" />);

    await vi.waitFor(() => {
      expect(container.textContent).toBe("");
    });
    expect(page.getByRole("alert").elements().length).toBe(0);
  });

  it("shows an error message for a non-404 failure", async () => {
    mockedGetReadme.mockRejectedValue(new ApiError("internal", "boom", 500));
    await render(<ReadmeView repo="git-compose" />);

    await expect.element(page.getByRole("alert")).toMatchTextContent("boom");
  });
});
