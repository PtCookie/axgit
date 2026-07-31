import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { formatAbsoluteTime, formatRelativeTime } from "@/lib/format/time";

describe("formatRelativeTime", () => {
  beforeEach(() => {
    vi.setSystemTime(new Date("2026-07-31T00:00:00Z"));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("formats a time in the recent past", () => {
    expect(formatRelativeTime("2026-07-30T23:59:00Z")).toContain("분");
  });

  it("formats a time further in the past using a coarser unit", () => {
    expect(formatRelativeTime("2026-03-31T00:00:00Z")).toContain("개월");
  });
});

describe("formatAbsoluteTime", () => {
  it("formats an RFC 3339 timestamp deterministically", () => {
    expect(formatAbsoluteTime("2026-07-24T13:06:00+09:00")).toEqual(expect.any(String));
  });
});
