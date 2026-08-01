import type { ReactNode } from "react";

import { encodeSegment } from "@/lib/api/path";

const URL_SOURCE = String.raw`https?:\/\/[^\s<>"']+`;
const SHA_SOURCE = String.raw`\b[0-9a-f]{7,40}\b`;

// A URL or a bare commit sha, as a single alternation — URLs are tried
// first so a sha pattern can never partially match inside one.
const LINK_PATTERN = new RegExp(`(${URL_SOURCE})|(${SHA_SOURCE})`, "gi");

interface LinkifyOptions {
  /** Repository name, used to build sha links to `/{repo}/commit/{sha}`. */
  repo: string;
}

/**
 * Splits commit message text into a node list with bare URLs and commit
 * shas turned into links (DECISIONS.md #11 — regex-based, not a markdown
 * renderer). Returns React nodes rather than using
 * `dangerouslySetInnerHTML`: repository content is untrusted input.
 */
export function linkify(text: string, { repo }: LinkifyOptions): ReactNode[] {
  const nodes: ReactNode[] = [];
  let lastIndex = 0;
  let key = 0;

  LINK_PATTERN.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = LINK_PATTERN.exec(text)) !== null) {
    const [full, url, sha] = match;
    if (match.index > lastIndex) {
      nodes.push(text.slice(lastIndex, match.index));
    }

    if (url) {
      nodes.push(
        <a key={key++} href={url} rel="noopener noreferrer" target="_blank" className="underline">
          {url}
        </a>,
      );
    } else if (sha) {
      nodes.push(
        <a key={key++} href={`/${encodeSegment(repo)}/commit/${encodeSegment(sha)}`} className="underline">
          {sha}
        </a>,
      );
    }

    lastIndex = match.index + full.length;
    // A zero-length match would spin the loop forever; `LINK_PATTERN` never
    // produces one, but guard anyway since `lastIndex` is mutated above.
    if (full.length === 0) {
      LINK_PATTERN.lastIndex++;
    }
  }

  if (lastIndex < text.length) {
    nodes.push(text.slice(lastIndex));
  }

  return nodes;
}
