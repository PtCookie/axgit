import { useEffect, useState } from "react";

import { ApiError } from "@/lib/api/client";
import { getTree } from "@/lib/api/repos";
import type { EntryKind, TreeListing } from "@/lib/api/schemas";
import { formatMode } from "@/lib/format/mode";
import { formatSize } from "@/lib/format/size";
import { filePathFromPathname, paramFromSearch, repoFromPathname } from "@/lib/repo-param";
import { blobHref, treeHref } from "@/lib/repo-href";
import PathBreadcrumbs from "@/components/repo/PathBreadcrumbs";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

type State = { status: "loading" } | { status: "error"; error: ApiError } | { status: "data"; tree: TreeListing };

interface TreeViewProps {
  /**
   * Omitted by the prerendered `/{repo}/tree` shell, which is built under a
   * placeholder param (`lib/shell.ts`) — the real repo/path are read from
   * the URL in the browser. `client:only` guarantees this default is only
   * ever evaluated there.
   */
  repo?: string;
  /** `/`-joined path, empty for the root tree. */
  path?: string;
  ref?: string;
}

/** Also rendered statically into the page shell as the island's
 *  `slot="fallback"`, so the prerendered HTML is not blank. */
export function TreeViewSkeleton() {
  return (
    <div className="space-y-2" aria-busy="true">
      <Skeleton className="h-5 w-1/2" />
      <Skeleton className="h-8 w-full" />
      <Skeleton className="h-8 w-full" />
      <Skeleton className="h-8 w-full" />
    </div>
  );
}

function entryHref(kind: EntryKind, repo: string, path: string, ref: string | undefined): string | undefined {
  switch (kind) {
    case "tree":
      return treeHref(repo, path, ref);
    case "blob":
    case "symlink":
      return blobHref(repo, path, ref);
    case "commit":
      // A submodule gitlink has no content in this repository to link to.
      return undefined;
  }
}

const KIND_LABEL: Record<EntryKind, string> = {
  tree: "directory",
  blob: "file",
  symlink: "symlink",
  commit: "submodule",
};

export default function TreeView({ repo, path, ref: refParam }: TreeViewProps) {
  const resolvedRepo = repo ?? repoFromPathname(window.location.pathname);
  const resolvedPath = path ?? filePathFromPathname(window.location.pathname);
  const resolvedRef = refParam ?? paramFromSearch("ref", window.location.search);
  const [state, setState] = useState<State>({ status: "loading" });

  useEffect(() => {
    let cancelled = false;

    getTree(resolvedRepo, resolvedRef, resolvedPath)
      .then((tree) => {
        if (!cancelled) {
          setState({ status: "data", tree });
        }
      })
      .catch((error: unknown) => {
        if (cancelled) {
          return;
        }
        setState({
          status: "error",
          error: error instanceof ApiError ? error : new ApiError("internal", "unknown error", 0),
        });
      });

    return () => {
      cancelled = true;
    };
  }, [resolvedRepo, resolvedRef, resolvedPath]);

  if (state.status === "loading") {
    return <TreeViewSkeleton />;
  }

  if (state.status === "error") {
    const message =
      state.error.code === "path_not_found"
        ? "Path not found."
        : state.error.code === "repo_not_found"
          ? "Repository not found."
          : state.error.message;
    return (
      <p role="alert" className="text-destructive text-sm">
        Failed to load tree: {message}
      </p>
    );
  }

  const { tree } = state;
  const parentPath = resolvedPath.split("/").slice(0, -1).join("/");

  return (
    <div className="space-y-4">
      <PathBreadcrumbs repo={resolvedRepo} path={resolvedPath} ref={resolvedRef} />

      <Table>
        <TableHeader>
          <TableRow>
            <TableHead className="w-px">Mode</TableHead>
            <TableHead>Name</TableHead>
            <TableHead>Size</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {resolvedPath && (
            <TableRow>
              <TableCell colSpan={3}>
                <a className="hover:underline" href={treeHref(resolvedRepo, parentPath, resolvedRef)}>
                  ..
                </a>
              </TableCell>
            </TableRow>
          )}
          {tree.entries.length === 0 && !resolvedPath ? (
            <TableRow>
              <TableCell colSpan={3} className="text-muted-foreground">
                This repository is empty.
              </TableCell>
            </TableRow>
          ) : (
            tree.entries.map((entry) => {
              const entryPath = resolvedPath ? `${resolvedPath}/${entry.name}` : entry.name;
              const href = entryHref(entry.type, resolvedRepo, entryPath, resolvedRef);
              const label = entry.type === "tree" ? `${entry.name}/` : entry.name;
              return (
                <TableRow key={entry.name}>
                  <TableCell className="text-muted-foreground w-px font-mono" title={entry.mode}>
                    {formatMode(entry.mode)}
                  </TableCell>
                  <TableCell className="font-medium">
                    {href ? (
                      <a className="hover:underline" href={href}>
                        {label}
                      </a>
                    ) : (
                      <span title={KIND_LABEL[entry.type]}>{label}</span>
                    )}
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {entry.size === null ? "—" : formatSize(entry.size)}
                  </TableCell>
                </TableRow>
              );
            })
          )}
        </TableBody>
      </Table>
    </div>
  );
}
