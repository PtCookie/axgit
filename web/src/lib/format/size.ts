const UNITS = ["B", "KiB", "MiB", "GiB"] as const;

/** Formats a byte count as a short human-readable size (binary units, one
 *  decimal place above the base unit). */
export function formatSize(bytes: number): string {
  let value = bytes;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < UNITS.length - 1) {
    value /= 1024;
    unitIndex++;
  }
  const formatted = unitIndex === 0 ? String(value) : value.toFixed(1);
  return `${formatted} ${UNITS[unitIndex]}`;
}
