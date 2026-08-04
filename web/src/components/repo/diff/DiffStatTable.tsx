import type { DiffStat } from "@/lib/api/schemas";
import { renamePathLabel, STATUS_CLASS, STATUS_LABEL } from "@/lib/format/diff-status";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

/** The uncapped diffstat table (file → status → change count), shared by the
 *  commit page and the two-revision compare page. Each row links to its file's
 *  `#diff-N` anchor in the file list below (present for up to 300 files — the
 *  diff endpoints' own cap; a stale anchor past that just no-ops). */
export default function DiffStatTable({ stat }: { stat: DiffStat }) {
  return (
    <div className="space-y-2">
      <h3 className="text-sm font-medium">
        {stat.files_changed} file{stat.files_changed === 1 ? "" : "s"} changed, +{stat.total_additions} −
        {stat.total_deletions}
      </h3>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Path</TableHead>
            <TableHead>Status</TableHead>
            <TableHead>Changes</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {stat.files.map((file, index) => (
            <TableRow key={file.path}>
              <TableCell className="font-mono">
                <a href={`#diff-${index + 1}`} className="hover:underline">
                  {renamePathLabel(file)}
                </a>
              </TableCell>
              <TableCell className={STATUS_CLASS[file.status]}>{STATUS_LABEL[file.status]}</TableCell>
              <TableCell className="text-muted-foreground font-mono">
                {file.binary ? "binary" : `+${file.additions} −${file.deletions}`}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  );
}
