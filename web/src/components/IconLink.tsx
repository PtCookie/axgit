import type { Icon } from "@phosphor-icons/react";

/** An icon-only link, its accessible name carried entirely by `label`
 *  (`aria-label`) rather than visible text — used for the per-row quick
 *  links on the repository index (`RepoList.tsx`) and the tree listing
 *  (`TreeView.tsx`), where a text label per row/action would be too wide.
 *  `aria-hidden` on the icon is load-bearing: without it, some screen
 *  readers would announce the icon's own (unrelated) title in addition to
 *  `label`. Same `size-4` idiom as `RepoSummary.tsx`'s `MetaLink`. */
export default function IconLink({ href, label, Icon: IconComponent }: { href: string; label: string; Icon: Icon }) {
  return (
    <a
      href={href}
      aria-label={label}
      className="text-muted-foreground hover:text-foreground inline-flex items-center transition-colors"
    >
      <IconComponent className="size-4" aria-hidden="true" />
    </a>
  );
}
