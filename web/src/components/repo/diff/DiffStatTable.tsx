import type { DiffStat, DiffStatFile } from "@/lib/api/schemas";
import { renamePathLabel, STATUS_CLASS, STATUS_LABEL } from "@/lib/format/diff-status";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

interface DiffStatTableProps {
  stat: DiffStat;
  /**
   * Builds each row's link. Defaults to the `#diff-N` anchor in the file
   * list below (present for up to 300 files — the diff endpoints' own cap;
   * a stale anchor past that just no-ops). `view=stat` passes a per-file
   * diff href instead, since there's no file list on the page to jump to.
   */
  hrefFor?: (file: DiffStatFile, index: number) => string;
}

const defaultHrefFor = (_file: DiffStatFile, index: number) => `#diff-${index + 1}`;

/** The uncapped diffstat table (file → status → change count), shared by the
 *  commit page and the two-revision compare page. */
export default function DiffStatTable({ stat, hrefFor = defaultHrefFor }: DiffStatTableProps) {
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
                <a href={hrefFor(file, index)} className="hover:underline">
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
