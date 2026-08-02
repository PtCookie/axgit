import { describe, expect, it } from "vitest";

import { formatMode } from "@/lib/format/mode";

describe("formatMode", () => {
  it("formats a regular file", () => {
    expect(formatMode("100644")).toEqual("-rw-r--r--");
  });

  it("formats an executable file", () => {
    expect(formatMode("100755")).toEqual("-rwxr-xr-x");
  });

  it("formats a directory with no permission bits", () => {
    expect(formatMode("040000")).toEqual("d---------");
  });

  it("formats a symlink with no permission bits", () => {
    expect(formatMode("120000")).toEqual("l---------");
  });

  it("formats a submodule gitlink with no permission bits", () => {
    expect(formatMode("160000")).toEqual("m---------");
  });

  it("falls back to the raw input for a malformed mode", () => {
    expect(formatMode("not-octal")).toEqual("not-octal");
  });
});
