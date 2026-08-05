import type { GraphRow } from "@/lib/commit-graph";

/**
 * One row of the Log tab's graph column (docs/DECISIONS.md #33). Renders
 * `row`'s lanes as an inline SVG sized to the page-wide lane count, so lanes
 * line up column-to-column across rows.
 *
 * `ROW_HEIGHT` is coupled to the `h-12` class `CommitLog` puts on each
 * `TableRow` — the graph cell has no padding of its own, so if that class
 * ever changes this constant must change with it. `CommitGraphSpacer` below
 * is exempt from that coupling — it sizes itself to whatever height its
 * container ends up with.
 */

/** Centre-to-centre spacing between lanes, in px. */
export const LANE_WIDTH = 12;
/** Must match the `h-12` row height `CommitLog` applies to each `TableRow`. */
export const ROW_HEIGHT = 48;

const MID = ROW_HEIGHT / 2;
const NODE_R = 3.5;
const MERGE_R = 4;
/** Overdraw past both edges so adjacent rows' lines meet across the
 *  table's collapsed row border, instead of leaving a 1px gap. */
const BLEED = 1;

function x(lane: number): number {
  return LANE_WIDTH / 2 + lane * LANE_WIDTH;
}

interface CommitGraphProps {
  row: GraphRow;
  /** Page-wide lane count (`CommitGraph.lanes` from `layoutCommitGraph`). */
  lanes: number;
}

export default function CommitGraph({ row, lanes }: CommitGraphProps) {
  const paths: { key: string; d: string }[] = [];

  for (const lane of row.through) {
    paths.push({ key: `t${lane}`, d: `M${x(lane)} ${-BLEED}V${ROW_HEIGHT + BLEED}` });
  }
  for (const lane of row.in) {
    paths.push({
      key: `i${lane}`,
      d:
        lane === row.lane
          ? `M${x(lane)} ${-BLEED}V${MID}`
          : `M${x(lane)} ${-BLEED}C${x(lane)} ${MID / 2},${x(row.lane)} ${MID / 2},${x(row.lane)} ${MID}`,
    });
  }
  for (const lane of row.out) {
    paths.push({
      key: `o${lane}`,
      d:
        lane === row.lane
          ? `M${x(row.lane)} ${MID}V${ROW_HEIGHT + BLEED}`
          : `M${x(row.lane)} ${MID}C${x(row.lane)} ${MID * 1.5},${x(lane)} ${MID * 1.5},${x(lane)} ${ROW_HEIGHT + BLEED}`,
    });
  }

  return (
    <svg
      data-testid="commit-graph"
      aria-hidden="true"
      focusable="false"
      width={lanes * LANE_WIDTH}
      height={ROW_HEIGHT}
      viewBox={`0 0 ${lanes * LANE_WIDTH} ${ROW_HEIGHT}`}
      className="text-muted-foreground block overflow-visible"
    >
      <g fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round">
        {paths.map((path) => (
          <path key={path.key} d={path.d} />
        ))}
      </g>
      {row.merge ? (
        <circle
          cx={x(row.lane)}
          cy={MID}
          r={MERGE_R}
          data-merge="true"
          className="fill-background text-foreground stroke-current stroke-2"
        />
      ) : (
        <circle cx={x(row.lane)} cy={MID} r={NODE_R} className="text-foreground fill-current" />
      )}
    </svg>
  );
}

/**
 * Graph segment for a commit's expanded message row (`CommitLog`'s `msg=1`
 * mode). Unlike `CommitGraph`, this row's height is whatever its content
 * needs, so it can't be laid out on the fixed `ROW_HEIGHT` grid — instead it
 * draws only the straight verticals for lanes that continue past the commit
 * row's bottom edge (`row.through` plus `row.out`, since a lane a commit's
 * own node just moved into keeps going too) as an absolutely positioned
 * overlay that stretches to fill its container. No node, no diagonals: those
 * belong to the commit row above.
 */
export function CommitGraphSpacer({ row, lanes }: CommitGraphProps) {
  const continuing = [...new Set([...row.through, ...row.out])].sort((a, b) => a - b);

  return (
    <svg
      data-testid="commit-graph-spacer"
      aria-hidden="true"
      focusable="false"
      width={lanes * LANE_WIDTH}
      preserveAspectRatio="none"
      className="text-muted-foreground absolute inset-y-0 left-0 h-full overflow-visible"
    >
      <g stroke="currentColor" strokeWidth={1.5} fill="none">
        {continuing.map((lane) => (
          <line key={lane} x1={x(lane)} y1={-BLEED} x2={x(lane)} y2="100%" />
        ))}
      </g>
    </svg>
  );
}
