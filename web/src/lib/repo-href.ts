import { encodePath, encodeSegment } from "@/lib/api/path";

/** Appends `?ref=` when a non-default ref is selected — shared by every
 *  href builder below (`?ref=`-only ref selection, DECISIONS.md #18). */
function refQuery(ref: string | undefined): string {
  return ref ? `?ref=${encodeSegment(ref)}` : "";
}

/** Builds a `/{repo}/tree/{path}` href, shared with `TreeView` (the parent-
 *  directory row uses the same link shape as a breadcrumb ancestor) and
 *  `PathBreadcrumbs`. */
export function treeHref(repo: string, path: string, ref: string | undefined): string {
  return `/${encodeSegment(repo)}/tree${path ? `/${encodePath(path)}` : ""}${refQuery(ref)}`;
}

/** Builds a `/{repo}/blob/{path}` href. `path` must be non-empty — there's
 *  nothing to show for `/{repo}/blob` itself. */
export function blobHref(repo: string, path: string, ref: string | undefined): string {
  return `/${encodeSegment(repo)}/blob/${encodePath(path)}${refQuery(ref)}`;
}

/** Builds a `/{repo}/blame/{path}` href. Same non-empty-`path` rule as
 *  `blobHref`. */
export function blameHref(repo: string, path: string, ref: string | undefined): string {
  return `/${encodeSegment(repo)}/blame/${encodePath(path)}${refQuery(ref)}`;
}
