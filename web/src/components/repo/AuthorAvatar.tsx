import { useMemo } from "react";

import { Avatar, Style } from "@dicebear/core";
import identiconStyle from "@dicebear/styles/identicon.json";

import type { CommitAuthor } from "@/lib/api/schemas";

/**
 * DiceBear 10 validates and decomposes a style definition on construction, so
 * the wrapper is built once at module scope and shared by every avatar rather
 * than rebuilt per render.
 */
const identicon = new Style(identiconStyle);

interface AuthorAvatarProps {
  author: CommitAuthor;
  className?: string;
}

/**
 * Locally generated avatar (DECISIONS.md #11) — no Gravatar/external
 * request. Seeded from `email_hash` (the raw address is never sent to the
 * frontend at all, let alone a third party).
 */
export default function AuthorAvatar({ author, className }: AuthorAvatarProps) {
  const dataUri = useMemo(() => new Avatar(identicon, { seed: author.email_hash }).toDataUri(), [author.email_hash]);

  // The author's name is rendered as adjacent text everywhere this is used
  // (log rows, commit header), so the image itself stays decorative.
  return <img src={dataUri} alt="" width={32} height={32} className={className} />;
}
