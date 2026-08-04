import type { DiffStatus } from "@/lib/api/schemas";

export const STATUS_LABEL: Record<DiffStatus, string> = {
  added: "added",
  deleted: "deleted",
  modified: "modified",
  renamed: "renamed",
  copied: "copied",
  typechange: "typechange",
};

export const STATUS_CLASS: Record<DiffStatus, string> = {
  added: "text-green-600 dark:text-green-400",
  deleted: "text-destructive",
  modified: "text-amber-600 dark:text-amber-400",
  renamed: "text-muted-foreground",
  copied: "text-muted-foreground",
  typechange: "text-muted-foreground",
};

/** `old → new` when renamed/copied and the paths actually differ; otherwise just `path`. */
export function renamePathLabel(file: { path: string; old_path: string | null }): string {
  return file.old_path && file.old_path !== file.path ? `${file.old_path} → ${file.path}` : file.path;
}
