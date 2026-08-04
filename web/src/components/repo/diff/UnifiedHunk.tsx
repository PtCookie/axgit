import type { Hunk, Line } from "@/lib/api/schemas";

function DiffLineRow({ line }: { line: Line }) {
  const background =
    line.origin === "+"
      ? "bg-green-500/10 dark:bg-green-500/20"
      : line.origin === "-"
        ? "bg-red-500/10 dark:bg-red-500/20"
        : undefined;
  return (
    <tr className={background}>
      <td className="text-muted-foreground w-10 shrink-0 px-2 text-right font-mono select-none">
        {line.old_lineno ?? ""}
      </td>
      <td className="text-muted-foreground w-10 shrink-0 px-2 text-right font-mono select-none">
        {line.new_lineno ?? ""}
      </td>
      <td className="w-4 shrink-0 text-center font-mono select-none">{line.origin}</td>
      <td className="font-mono whitespace-pre">{line.content}</td>
    </tr>
  );
}

/** One hunk rendered as a 4-column unified diff table (lineno × 2, origin, content). */
export default function UnifiedHunk({ hunk }: { hunk: Hunk }) {
  return (
    <table className="w-full border-collapse text-sm">
      <tbody>
        <tr>
          <td colSpan={4} className="text-muted-foreground bg-muted/30 px-2 py-1 font-mono">
            {hunk.header}
          </td>
        </tr>
        {hunk.lines.map((line, index) => (
          // eslint-disable-next-line @eslint-react/no-array-index-key -- lines have no stable identity
          <DiffLineRow key={index} line={line} />
        ))}
      </tbody>
    </table>
  );
}
