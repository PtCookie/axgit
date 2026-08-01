import { describe, expect, it } from "vitest";

import { REPO_SHELL_PARAM, shellFor } from "@/lib/shell";
import { repoFromPathname } from "@/lib/repo-param";

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

  it("maps unmatched shapes to /404", () => {
    for (const path of ["/git-compose/log", "/git-compose/tree/src", "/a/b/c"]) {
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
