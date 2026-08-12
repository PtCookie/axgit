import { apiFetch } from "./client";
import type { SiteInfo } from "./schemas";

/** Site-wide title/description/readme (docs/DECISIONS.md #70/#71) — not
 *  scoped to any repository, unlike every other function in `lib/api/`. */
export function getSite(): Promise<SiteInfo> {
  return apiFetch<SiteInfo>("/site");
}
