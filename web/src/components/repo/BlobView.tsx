import { useEffect, useState } from "react";

import { ApiError } from "@/lib/api/client";
import { getBlob, rawUrl } from "@/lib/api/repos";
import type { BlobInfo } from "@/lib/api/schemas";
import { formatSize } from "@/lib/format/size";
import { filePathFromPathname, paramFromSearch, repoFromPathname } from "@/lib/repo-param";
import { blameHref, logHref } from "@/lib/repo-href";
import CodeBlock from "@/components/repo/CodeBlock";
import HexDump from "@/components/repo/HexDump";
import PathBreadcrumbs from "@/components/repo/PathBreadcrumbs";
import { Skeleton } from "@/components/ui/skeleton";

type State = { status: "loading" } | { status: "error"; error: ApiError } | { status: "data"; blob: BlobInfo };

interface BlobViewProps {
  /**
   * Omitted by the prerendered `/{repo}/blob` shell, which is built under a
   * placeholder param (`lib/shell.ts`) — the real repo/path are read from
   * the URL in the browser. `client:only` guarantees this default is only
   * ever evaluated there.
   */
  repo?: string;
  path?: string;
  ref?: string;
}

/** Also rendered statically into the page shell as the island's
 *  `slot="fallback"`, so the prerendered HTML is not blank. */
export function BlobViewSkeleton() {
  return (
    <div className="space-y-2" aria-busy="true">
      <Skeleton className="h-5 w-1/2" />
      <Skeleton className="h-64 w-full" />
    </div>
  );
}

/** `mode` for a symlink entry (API.md's tree/blob section). */
const SYMLINK_MODE = "120000";

export default function BlobView({ repo, path, ref: refParam }: BlobViewProps) {
  const resolvedRepo = repo ?? repoFromPathname(window.location.pathname);
  const resolvedPath = path ?? filePathFromPathname(window.location.pathname);
  const resolvedRef = refParam ?? paramFromSearch("ref", window.location.search);
  const [state, setState] = useState<State>({ status: "loading" });

  useEffect(() => {
    let cancelled = false;

    getBlob(resolvedRepo, resolvedRef, resolvedPath)
      .then((blob) => {
        if (!cancelled) {
          setState({ status: "data", blob });
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
    return <BlobViewSkeleton />;
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
        Failed to load file: {message}
      </p>
    );
  }

  const { blob } = state;
  const raw = rawUrl(resolvedRepo, resolvedRef, resolvedPath);
  const historyHref = logHref(resolvedRepo, { path: blob.path, ref: resolvedRef });

  return (
    <div className="space-y-4">
      <PathBreadcrumbs repo={resolvedRepo} path={resolvedPath} ref={resolvedRef} />

      <div className="text-muted-foreground flex flex-wrap items-center gap-x-4 gap-y-1 text-sm">
        <span>{formatSize(blob.size)}</span>
        <a className="hover:text-foreground hover:underline" href={raw}>
          Raw
        </a>
        <a className="hover:text-foreground hover:underline" href={blameHref(resolvedRepo, resolvedPath, resolvedRef)}>
          Blame
        </a>
        <a className="hover:text-foreground hover:underline" href={historyHref}>
          History
        </a>
      </div>

      {blob.mode === SYMLINK_MODE ? (
        <p className="text-sm">
          Symlink to <code className="bg-muted/50 rounded px-1 py-0.5 font-mono">{blob.content}</code>
        </p>
      ) : blob.binary ? (
        <HexDump url={raw} size={blob.size} />
      ) : blob.too_large ? (
        <p className="text-muted-foreground text-sm">
          File too large to display —{" "}
          <a className="underline" href={raw}>
            view raw
          </a>
          .
        </p>
      ) : (
        <CodeBlock content={blob.content ?? ""} path={blob.path} />
      )}
    </div>
  );
}
