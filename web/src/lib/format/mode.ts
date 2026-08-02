const TYPE_CHARS: Record<number, string> = {
  0o040000: "d", // directory (tree)
  0o120000: "l", // symlink
  0o160000: "m", // submodule (gitlink)
};

const RWX_TRIPLETS = ["---", "--x", "-w-", "-wx", "r--", "r-x", "rw-", "rwx"] as const;

/** Formats a 6-digit octal git file mode (e.g. `"100644"`) as a symbolic
 *  `ls -l`-style string (e.g. `"-rw-r--r--"`), the way cgit does. Trees,
 *  symlinks, and submodule gitlinks carry no permission bits in git, so
 *  their permission triplets render as `---------`. Falls back to the raw
 *  input if it isn't a valid octal mode. */
export function formatMode(mode: string): string {
  const bits = Number.parseInt(mode, 8);
  if (Number.isNaN(bits)) {
    return mode;
  }
  const typeChar = TYPE_CHARS[bits & 0o170000] ?? "-";
  const owner = RWX_TRIPLETS[(bits >> 6) & 0o7];
  const group = RWX_TRIPLETS[(bits >> 3) & 0o7];
  const other = RWX_TRIPLETS[bits & 0o7];
  return `${typeChar}${owner}${group}${other}`;
}
