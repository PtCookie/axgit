import { describe, expect, it } from "vitest";

import { REPO_SHELL_PARAM, shellFor } from "@/lib/shell";
import { commitShaFromPathname, repoFromPathname } from "@/lib/repo-param";

// Same case table as `api/src/shell.rs`'s unit tests — the two must change
// together (see the doc comment on `shellFor`).
describe("shellFor", () => {
  it("maps the root to /", () => {
    expect(shellFor("/")).toBe("/");
  });

  it("maps repo paths to the placeholder repo shell", () => {
    for (const path of ["/git-compose", "/git-compose/", "/my%20repo"]) {
      expect(shellFor(path)).toBe(`/${REPO_SHELL_PARAM}`);
    }
  });

  it("maps repo refs paths to the placeholder refs shell", () => {
    for (const path of ["/git-compose/refs", "/git-compose/refs/"]) {
      expect(shellFor(path)).toBe(`/${REPO_SHELL_PARAM}/refs`);
    }
  });

  it("maps repo log paths to the placeholder log shell", () => {
    for (const path of ["/git-compose/log", "/git-compose/log/"]) {
      expect(shellFor(path)).toBe(`/${REPO_SHELL_PARAM}/log`);
    }
  });

  it("maps repo commit paths to the placeholder commit shell", () => {
    for (const path of ["/git-compose/commit/abc123", "/git-compose/commit/abc123/"]) {
      expect(shellFor(path)).toBe(`/${REPO_SHELL_PARAM}/commit`);
    }
  });

  it("maps unmatched shapes to /404", () => {
    for (const path of [
      "/git-compose/tree/src",
      "/git-compose/commit",
      "/git-compose/commit/abc123/extra",
      "/git-compose/stats",
      "/a/b/c",
    ]) {
      expect(shellFor(path)).toBe("/404");
    }
  });
});

describe("repoFromPathname", () => {
  it("returns an empty string at the root", () => {
    expect(repoFromPathname("/")).toBe("");
  });

  it("returns the decoded first segment", () => {
    expect(repoFromPathname("/git-compose")).toBe("git-compose");
    expect(repoFromPathname("/git-compose/refs")).toBe("git-compose");
    expect(repoFromPathname("/my%20repo")).toBe("my repo");
  });

  it("falls back to the raw segment for a malformed escape", () => {
    expect(repoFromPathname("/%zz")).toBe("%zz");
  });
});

describe("commitShaFromPathname", () => {
  it("returns an empty string when there is no sha segment", () => {
    expect(commitShaFromPathname("/")).toBe("");
    expect(commitShaFromPathname("/git-compose")).toBe("");
    expect(commitShaFromPathname("/git-compose/commit")).toBe("");
  });

  it("returns the decoded third segment", () => {
    expect(commitShaFromPathname("/git-compose/commit/abc123")).toBe("abc123");
  });

  it("falls back to the raw segment for a malformed escape", () => {
    expect(commitShaFromPathname("/git-compose/commit/%zz")).toBe("%zz");
  });
});
