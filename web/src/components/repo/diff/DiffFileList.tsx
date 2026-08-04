import type { FileDiff } from "@/lib/api/schemas";
import type { DiffViewMode } from "@/lib/diff-options";
import DiffFile from "@/components/repo/diff/DiffFile";

interface DiffFileListProps {
  /** `true` when files beyond the 300-file cap were omitted (shared by the
   *  per-commit diff and the two-revision diff — both cap the same way). */
  truncated: boolean;
  files: FileDiff[];
  view: DiffViewMode;
}

/** The full list of per-file diffs for a commit or a two-revision comparison. */
export default function DiffFileList({ truncated, files, view }: DiffFileListProps) {
  return (
    <div className="space-y-3">
      {truncated && (
        <p className="text-muted-foreground text-sm">
          Some files were omitted (300 files max) — see the table above for the full file list.
        </p>
      )}
      {files.map((file, index) => (
        <DiffFile key={file.path} file={file} index={index} view={view} />
      ))}
    </div>
  );
}
