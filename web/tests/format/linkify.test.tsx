import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render } from "vitest-browser-react";
import { page } from "vitest/browser";

import { linkify } from "@/lib/format/linkify";

afterEach(() => {
  cleanup();
});

describe("linkify", () => {
  it("leaves plain text unchanged", async () => {
    render(<>{linkify("fix: update readme", { repo: "git-compose" })}</>);
    await expect.element(page.getByText("fix: update readme")).toBeVisible();
  });

  it("turns a bare URL into a link", async () => {
    render(<>{linkify("see https://example.com/docs for details", { repo: "git-compose" })}</>);
    const link = page.getByRole("link", { name: "https://example.com/docs" });
    await expect.element(link).toBeVisible();
    await expect.element(link).toHaveAttribute("href", "https://example.com/docs");
    await expect.element(link).toHaveAttribute("target", "_blank");
  });

  it("turns a bare commit sha into a repo-relative link", async () => {
    render(<>{linkify("fixed by abc1234def", { repo: "git-compose" })}</>);
    const link = page.getByRole("link", { name: "abc1234def" });
    await expect.element(link).toBeVisible();
    await expect.element(link).toHaveAttribute("href", "/git-compose/commit/abc1234def");
  });

  it("does not link short hex runs below the 7-character minimum", async () => {
    render(<>{linkify("see abc123 for context", { repo: "git-compose" })}</>);
    await expect.element(page.getByText("see abc123 for context")).toBeVisible();
    expect(page.getByRole("link").elements().length).toBe(0);
  });
});
