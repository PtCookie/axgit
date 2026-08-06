import { describe, expect, it } from "vitest";

import { REPO_SHELL_PARAM, shellFor } from "@/lib/shell";
import { commitShaFromPathname, filePathFromPathname, repoFromPathname } from "@/lib/repo-param";

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

  it("maps repo search paths to the placeholder search shell", () => {
    for (const path of ["/git-compose/search", "/git-compose/search/"]) {
      expect(shellFor(path)).toBe(`/${REPO_SHELL_PARAM}/search`);
    }
  });

  it("maps repo commit paths to the placeholder commit shell", () => {
    for (const path of ["/git-compose/commit/abc123", "/git-compose/commit/abc123/"]) {
      expect(shellFor(path)).toBe(`/${REPO_SHELL_PARAM}/commit`);
    }
  });

  it("maps repo tree paths to the placeholder tree shell", () => {
    for (const path of [
      "/git-compose/tree",
      "/git-compose/tree/",
      "/git-compose/tree/src",
      "/git-compose/tree/src/lib",
    ]) {
      expect(shellFor(path)).toBe(`/${REPO_SHELL_PARAM}/tree`);
    }
  });

  it("maps repo blob paths to the placeholder blob shell", () => {
    for (const path of [
      "/git-compose/blob/src/main.rs",
      "/git-compose/blob/src/main.rs/",
      "/git-compose/blob/README.md",
    ]) {
      expect(shellFor(path)).toBe(`/${REPO_SHELL_PARAM}/blob`);
    }
  });

  it("maps repo blame paths to the placeholder blame shell", () => {
    for (const path of [
      "/git-compose/blame/src/main.rs",
      "/git-compose/blame/src/main.rs/",
      "/git-compose/blame/README.md",
    ]) {
      expect(shellFor(path)).toBe(`/${REPO_SHELL_PARAM}/blame`);
    }
  });

  it("maps repo tag paths to the placeholder tag shell", () => {
    for (const path of ["/git-compose/tag/v1.0.0", "/git-compose/tag/v1.0.0/", "/git-compose/tag/release/1.0"]) {
      expect(shellFor(path)).toBe(`/${REPO_SHELL_PARAM}/tag`);
    }
  });

  it("maps repo stats paths to the placeholder stats shell", () => {
    for (const path of ["/git-compose/stats", "/git-compose/stats/"]) {
      expect(shellFor(path)).toBe(`/${REPO_SHELL_PARAM}/stats`);
    }
  });

  it("maps repo diff paths to the placeholder diff shell", () => {
    for (const path of ["/git-compose/diff", "/git-compose/diff/"]) {
      expect(shellFor(path)).toBe(`/${REPO_SHELL_PARAM}/diff`);
    }
  });

  it("maps unmatched shapes to /404", () => {
    for (const path of [
      "/git-compose/blob",
      "/git-compose/blob/",
      "/git-compose/blame",
      "/git-compose/blame/",
      "/git-compose/commit",
      "/git-compose/commit/abc123/extra",
      "/git-compose/stats/extra",
      "/git-compose/diff/extra",
      "/git-compose/tag",
      "/git-compose/tag/",
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

describe("filePathFromPathname", () => {
  it("returns an empty string when there is no path segment", () => {
    expect(filePathFromPathname("/")).toBe("");
    expect(filePathFromPathname("/git-compose")).toBe("");
    expect(filePathFromPathname("/git-compose/tree")).toBe("");
    expect(filePathFromPathname("/git-compose/tree/")).toBe("");
  });

  it("returns the decoded remainder joined by /", () => {
    expect(filePathFromPathname("/git-compose/tree/src")).toBe("src");
    expect(filePathFromPathname("/git-compose/tree/src/lib")).toBe("src/lib");
    expect(filePathFromPathname("/git-compose/blob/src/main.rs")).toBe("src/main.rs");
  });

  it("decodes each segment independently", () => {
    expect(filePathFromPathname("/git-compose/blob/my%20dir/file.rs")).toBe("my dir/file.rs");
  });

  it("falls back to the raw segment for a malformed escape", () => {
    expect(filePathFromPathname("/git-compose/blob/%zz")).toBe("%zz");
  });
});
