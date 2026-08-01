import { Fragment, isValidElement, useEffect, useState, type ReactNode } from "react";
import Markdown, { type Components } from "react-markdown";
import rehypeSanitize from "rehype-sanitize";
import remarkGfm from "remark-gfm";

import { ApiError } from "@/lib/api/client";
import { getReadme, rawUrl } from "@/lib/api/repos";
import type { ReadmeInfo } from "@/lib/api/schemas";
import { highlightFence, type HighlightedLine } from "@/lib/format/highlight";
import { isExternalUrl, resolveRepoPath } from "@/lib/markdown-url";
import { blobHref, treeHref } from "@/lib/repo-href";
import { repoFromPathname } from "@/lib/repo-param";
import { Skeleton } from "@/components/ui/skeleton";

type State =
  | { status: "loading" }
  | { status: "error"; error: ApiError }
  /** No README candidate found (`path_not_found`) or an unborn HEAD
   *  (`ref_not_found`) — both are a normal "nothing to show" outcome, not
   *  an error (docs/API.md's readme section). */
  | { status: "empty" }
  | { status: "data"; readme: ReadmeInfo };

interface ReadmeViewProps {
  /**
   * Omitted by the prerendered `/{repo}` shell, which is built under a
   * placeholder param (`lib/shell.ts`) — the real name is read from the URL
   * in the browser. `client:only` guarantees this default is only ever
   * evaluated there.
   */
  repo?: string;
}

/** Also rendered statically into the page shell as the island's
 *  `slot="fallback"`, so the prerendered HTML is not blank. */
export function ReadmeViewSkeleton() {
  return (
    <div className="space-y-2" aria-busy="true">
      <Skeleton className="h-5 w-1/3" />
      <Skeleton className="h-40 w-full" />
    </div>
  );
}

/** `code` component override — used for both inline code and (transiently,
 *  before `pre` intercepts it) fenced blocks. Top-level, not defined inside
 *  `ReadmeView`, so `pre`'s `child.type === InlineCode` identity check below
 *  is stable across renders. `node` (the hast element, injected by
 *  react-markdown alongside every component's regular props) must never be
 *  spread onto a real DOM element — destructured here purely to exclude it
 *  from `...props`. */
// eslint-disable-next-line @typescript-eslint/no-unused-vars
function InlineCode({ className, children, node, ...props }: JSX.IntrinsicElements["code"] & { node?: unknown }) {
  return (
    <code className={`bg-muted rounded px-1 py-0.5 font-mono text-sm ${className ?? ""}`} {...props}>
      {children}
    </code>
  );
}

/** Flattens a fenced code block's React children (plain text produced by
 *  rehype from the markdown AST) back into one string for highlighting. */
function textContent(node: ReactNode): string {
  if (typeof node === "string") return node;
  if (typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textContent).join("");
  if (isValidElement(node)) return textContent((node.props as { children?: ReactNode }).children);
  return "";
}

interface MarkdownFenceProps {
  code: string;
  /** The fence's language token (`code`'s `language-xxx` className, minus
   *  the prefix) — possibly empty for a fence with no language annotation. */
  info: string;
}

/** Renders one fenced code block with Shiki highlighting, swapped in once
 *  ready — same no-loading-flash pattern as `CodeBlock.tsx`, but without
 *  line numbers or a gutter (a README fence is just a code block). */
function MarkdownFence({ code, info }: MarkdownFenceProps) {
  const [highlighted, setHighlighted] = useState<HighlightedLine[] | null>(null);

  useEffect(() => {
    let cancelled = false;

    highlightFence(code, info).then((result) => {
      if (!cancelled) {
        setHighlighted(result);
      }
    });

    return () => {
      cancelled = true;
    };
  }, [code, info]);

  return (
    <pre className="shiki-code border-border overflow-x-auto rounded-md border p-3 font-mono text-sm">
      <code>
        {highlighted
          ? highlighted.map((lineTokens, index) => (
              // eslint-disable-next-line @eslint-react/no-array-index-key -- lines have no stable identity
              <Fragment key={index}>
                {lineTokens.map((token, tokenIndex) => (
                  // eslint-disable-next-line @eslint-react/no-array-index-key -- tokens have no stable identity
                  <span key={tokenIndex} style={token.style}>
                    {token.content}
                  </span>
                ))}
                {index < highlighted.length - 1 && "\n"}
              </Fragment>
            ))
          : code}
      </code>
    </pre>
  );
}

export default function ReadmeView({ repo }: ReadmeViewProps) {
  const resolvedRepo = repo ?? repoFromPathname(window.location.pathname);
  const [state, setState] = useState<State>({ status: "loading" });

  useEffect(() => {
    let cancelled = false;

    getReadme(resolvedRepo)
      .then((readme) => {
        if (!cancelled) {
          setState({ status: "data", readme });
        }
      })
      .catch((error: unknown) => {
        if (cancelled) {
          return;
        }
        if (error instanceof ApiError && error.status === 404) {
          setState({ status: "empty" });
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
  }, [resolvedRepo]);

  if (state.status === "loading") {
    return <ReadmeViewSkeleton />;
  }

  if (state.status === "empty") {
    return null;
  }

  if (state.status === "error") {
    return (
      <p role="alert" className="text-destructive text-sm">
        Failed to load README: {state.error.message}
      </p>
    );
  }

  const { readme } = state;

  /** Rewrites a README-relative link into a `/{repo}/tree|blob/...` page
   *  URL — a trailing `/` means a directory reference. Absolute/external
   *  URLs and anything that resolves outside the repository root pass
   *  through unchanged (`lib/markdown-url.ts`). No `?ref=` — the readme
   *  endpoint itself defaults to HEAD, matching the rest of the summary
   *  page. */
  function rewriteHref(url: string): string {
    if (isExternalUrl(url)) return url;
    const resolved = resolveRepoPath("", url);
    if (resolved === null) return url;
    return url.endsWith("/")
      ? treeHref(resolvedRepo, resolved, undefined)
      : blobHref(resolvedRepo, resolved, undefined);
  }

  /** Rewrites a README-relative image reference into a `/raw/...` API URL.
   *  Same external/root-escape rules as `rewriteHref`. */
  function rewriteSrc(url: string): string {
    if (isExternalUrl(url)) return url;
    const resolved = resolveRepoPath("", url);
    return resolved === null ? url : rawUrl(resolvedRepo, undefined, resolved);
  }

  const components: Components = {
    h1: (props) => <h1 className="text-foreground text-xl font-semibold" {...props} />,
    h2: (props) => <h2 className="text-foreground mt-6 text-lg font-semibold" {...props} />,
    h3: (props) => <h3 className="text-foreground mt-4 text-base font-semibold" {...props} />,
    h4: (props) => <h4 className="text-foreground mt-4 text-sm font-semibold" {...props} />,
    p: (props) => <p className="text-foreground leading-relaxed" {...props} />,
    a: ({ href, ...props }) => (
      <a className="text-primary underline underline-offset-2" href={href ? rewriteHref(href) : href} {...props} />
    ),
    img: ({ src, ...props }) => (
      <img className="max-w-full" src={typeof src === "string" ? rewriteSrc(src) : src} {...props} />
    ),
    ul: (props) => <ul className="list-disc space-y-1 pl-6" {...props} />,
    ol: (props) => <ol className="list-decimal space-y-1 pl-6" {...props} />,
    blockquote: (props) => <blockquote className="border-border text-muted-foreground border-l-2 pl-4" {...props} />,
    table: (props) => <table className="border-border w-full border-collapse text-sm" {...props} />,
    th: (props) => <th className="border-border border px-2 py-1 text-left font-medium" {...props} />,
    td: (props) => <td className="border-border border px-2 py-1" {...props} />,
    code: InlineCode,
    pre: ({ children }) => {
      // Only a fenced code block is wrapped in `<pre>` (inline code isn't),
      // and rehype always nests exactly one `<code>` inside it — so this is
      // the reliable place to intercept fences for highlighting, without
      // also matching inline `code` spans. `children` is hast-util-to-jsx-
      // runtime's array of child nodes even when there's exactly one, so
      // unwrap it before checking. Its `type` is `InlineCode` itself (the
      // component reference substituted in `components.code` above, not the
      // string `"code"`) — react-markdown substitutes the component before
      // building the element, it isn't resolved until React renders it.
      const child = Array.isArray(children) ? children[0] : children;
      if (isValidElement<{ className?: string; children?: ReactNode }>(child) && child.type === InlineCode) {
        const codeClassName = child.props.className ?? "";
        const match = /language-(\S+)/.exec(codeClassName);
        const code = textContent(child.props.children).replace(/\n$/, "");
        return <MarkdownFence code={code} info={match?.[1] ?? ""} />;
      }
      return <pre>{children}</pre>;
    },
  };

  return (
    <div className="space-y-3">
      <h2 className="text-muted-foreground font-mono text-xs tracking-wide uppercase">{readme.path}</h2>
      {readme.format === "markdown" ? (
        <div className="space-y-3">
          <Markdown remarkPlugins={[remarkGfm]} rehypePlugins={[rehypeSanitize]} components={components}>
            {readme.content}
          </Markdown>
        </div>
      ) : (
        <pre className="border-border overflow-x-auto rounded-md border p-3 font-mono text-sm whitespace-pre-wrap">
          {readme.content}
        </pre>
      )}
    </div>
  );
}
