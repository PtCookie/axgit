import { describe, expect, it } from "vitest";

import { parseRoute, repoUrl } from "@/lib/router";

describe("parseRoute", () => {
  it("maps the root path to the repo list", () => {
    expect(parseRoute("/")).toEqual({ name: "repos" });
    expect(parseRoute("")).toEqual({ name: "repos" });
  });

  it("maps a single segment to the repo summary route", () => {
    expect(parseRoute("/git-compose")).toEqual({ name: "repo", repo: "git-compose" });
    expect(parseRoute("/git-compose/")).toEqual({ name: "repo", repo: "git-compose" });
  });

  it("maps /{repo}/refs to the refs route", () => {
    expect(parseRoute("/git-compose/refs")).toEqual({ name: "refs", repo: "git-compose" });
  });

  it("decodes percent-encoded repo names", () => {
    expect(parseRoute("/my%20repo")).toEqual({ name: "repo", repo: "my repo" });
  });

  it("falls back to not-found for unsupported sub-routes and depths", () => {
    expect(parseRoute("/git-compose/log")).toEqual({ name: "not-found" });
    expect(parseRoute("/git-compose/tree/src")).toEqual({ name: "not-found" });
  });
});

describe("repoUrl", () => {
  it("builds the summary link by default", () => {
    expect(repoUrl("git-compose")).toBe("/git-compose");
  });

  it("builds a sub-page link", () => {
    expect(repoUrl("git-compose", "refs")).toBe("/git-compose/refs");
  });

  it("escapes special characters in the repo name", () => {
    expect(repoUrl("my repo")).toBe("/my%20repo");
  });
});
