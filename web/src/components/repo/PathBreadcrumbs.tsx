import { encodePath, encodeSegment } from "@/lib/api/path";

interface PathBreadcrumbsProps {
  repo: string;
  /** `/`-joined path, empty for the tree root. */
  path: string;
  ref?: string;
}

/** Builds a `/{repo}/tree/{path}` href, shared with `TreeView` (the parent-
 *  directory row uses the same link shape as a breadcrumb ancestor). */
export function treeHref(repo: string, path: string, ref: string | undefined) {
  const query = ref ? `?ref=${encodeSegment(ref)}` : "";
  return `/${encodeSegment(repo)}/tree${path ? `/${encodePath(path)}` : ""}${query}`;
}

/**
 * `repo / a / b / file.rs`-style breadcrumb, shared by `TreeView` and
 * `BlobView`. Every ancestor links back into the tree at that depth; the
 * last segment (the page's own directory or file) is plain text — matching
 * the "current location isn't a link" convention `RepoNav.astro` already
 * uses for its active tab.
 */
export default function PathBreadcrumbs({ repo, path, ref }: PathBreadcrumbsProps) {
  const segments = path ? path.split("/") : [];

  return (
    <nav aria-label="Path" className="text-muted-foreground flex flex-wrap items-center gap-1 text-sm break-all">
      <a className="hover:text-foreground hover:underline" href={treeHref(repo, "", ref)}>
        {repo}
      </a>
      {segments.map((segment, index) => {
        const isLast = index === segments.length - 1;
        const ancestorPath = segments.slice(0, index + 1).join("/");
        return (
          <span key={ancestorPath} className="flex items-center gap-1">
            <span aria-hidden="true">/</span>
            {isLast ? (
              <span className="text-foreground font-medium">{segment}</span>
            ) : (
              <a className="hover:text-foreground hover:underline" href={treeHref(repo, ancestorPath, ref)}>
                {segment}
              </a>
            )}
          </span>
        );
      })}
    </nav>
  );
}
