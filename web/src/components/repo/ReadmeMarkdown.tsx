import { Fragment, isValidElement, useEffect, useMemo, useState, type JSX, type ReactNode } from "react";
import Markdown, { type Components } from "react-markdown";
import rehypeSanitize from "rehype-sanitize";
import remarkGfm from "remark-gfm";

import { highlightFence, type HighlightedLine } from "@/lib/format/highlight";
import { isExternalUrl, resolveRepoPath } from "@/lib/markdown-url";
import { blobHref, treeHref } from "@/lib/repo-href";
import { rawUrl } from "@/lib/api/repos";

interface ReadmeMarkdownProps {
  /** Omitted for a readme with no single owning repository (the site-level
   *  readme, `SiteIntro.tsx`) — relative links/images are then left
   *  untouched (resolved by the browser against the current page) rather
   *  than rewritten into a nonsensical `/{repo}/...` href. */
  repo?: string;
  content: string;
}

/** `code` component override — used for both inline code and (transiently,
 *  before `pre` intercepts it) fenced blocks. Top-level, not defined inside
 *  `ReadmeMarkdown`, so `pre`'s `child.type === InlineCode` identity check
 *  below is stable across renders. `node` (the hast element, injected by
 *  react-markdown alongside every component's regular props) must never be
 *  spread onto a real DOM element — destructured here purely to exclude it
 *  from `...props`.
 *
 *  Must stay in this module, next to the `pre` override that checks its
 *  identity — see docs/DECISIONS.md #21: react-markdown substitutes the
 *  `code` *component reference* as the element's `type`, not the string
 *  `"code"`, so splitting these two across files would silently break the
 *  fence-detection check with no type error to catch it. */
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

/** The markdown-rendering half of `ReadmeView` — lazy-loaded, since
 *  react-markdown/remark-gfm/rehype-sanitize (~135 KB) are only needed for a
 *  repository whose README is actually `format: "markdown"`. `ReadmeView`
 *  keeps the fetch/state machine and the `rst`/`plain` `<pre>` branch, which
 *  need none of this. */
export default function ReadmeMarkdown({ repo, content }: ReadmeMarkdownProps) {
  // Rebuilt only when `repo` changes — the overrides below are inline arrow
  // functions, so without this every `ReadmeMarkdown` re-render would
  // unmount/remount the whole markdown tree (and restart `MarkdownFence`'s
  // highlight effect on every visible fence). `InlineCode`'s identity check
  // in `pre` below is unaffected either way, since that reference is
  // module-scope.
  const components = useMemo<Components>(() => {
    /** Rewrites a README-relative link into a `/{repo}/tree|blob/...` page
     *  URL — a trailing `/` means a directory reference. Absolute/external
     *  URLs and anything that resolves outside the repository root pass
     *  through unchanged (`lib/markdown-url.ts`). No `?ref=` — the readme
     *  endpoint itself defaults to HEAD, matching the rest of the summary
     *  page. */
    function rewriteHref(url: string): string {
      if (repo === undefined || isExternalUrl(url)) return url;
      const resolved = resolveRepoPath("", url);
      if (resolved === null) return url;
      return url.endsWith("/") ? treeHref(repo, resolved, undefined) : blobHref(repo, resolved, undefined);
    }

    /** Rewrites a README-relative image reference into a `/raw/...` API URL.
     *  Same external/root-escape/no-`repo` rules as `rewriteHref`. */
    function rewriteSrc(url: string): string {
      if (repo === undefined || isExternalUrl(url)) return url;
      const resolved = resolveRepoPath("", url);
      return resolved === null ? url : rawUrl(repo, undefined, resolved);
    }

    return {
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
        // and rehype always nests exactly one `<code>` inside it — so this
        // is the reliable place to intercept fences for highlighting,
        // without also matching inline `code` spans. `children` is
        // hast-util-to-jsx-runtime's array of child nodes even when there's
        // exactly one, so unwrap it before checking. Its `type` is
        // `InlineCode` itself (the component reference substituted in
        // `components.code` above, not the string `"code"`) —
        // react-markdown substitutes the component before building the
        // element, it isn't resolved until React renders it.
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
  }, [repo]);

  return (
    <div className="space-y-3">
      <Markdown remarkPlugins={[remarkGfm]} rehypePlugins={[rehypeSanitize]} components={components}>
        {content}
      </Markdown>
    </div>
  );
}
