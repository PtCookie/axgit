import { Bar, BarChart, CartesianGrid, XAxis, YAxis } from "recharts";

import { type ChartConfig, ChartContainer, ChartTooltip, ChartTooltipContent } from "@/components/ui/chart";

interface StatsChartProps {
  data: { label: string; commits: number }[];
}

const chartConfig: ChartConfig = {
  commits: { label: "Commits", color: "var(--chart-1)" },
};

/** Lazy-loaded by `StatsView.tsx` — Recharts is the single biggest
 *  dependency in the app (~330 KB), and it's only ever needed on
 *  `/{repo}/stats`. Kept as its own module so the `recharts`/`ui/chart`
 *  imports never land in `StatsView`'s own (eager) island entry chunk. */
export default function StatsChart({ data }: StatsChartProps) {
  return (
    <ChartContainer config={chartConfig} className="aspect-auto h-64 w-full">
      <BarChart data={data} margin={{ left: 0, right: 0, top: 8, bottom: 0 }}>
        <CartesianGrid vertical={false} />
        <XAxis dataKey="label" tickLine={false} axisLine={false} tickMargin={8} />
        <YAxis tickLine={false} axisLine={false} width={32} allowDecimals={false} />
        <ChartTooltip cursor={false} content={<ChartTooltipContent />} />
        {/* Recharts omits the bar element entirely for a zero value — with
            no mark there's no hover/tooltip hit target for that bucket
            (`references/interaction.md`'s "the mark is the hit target"
            rule). `minPointSize` keeps a thin sliver so every bucket stays
            hoverable, a zero-commit month included. */}
        <Bar dataKey="commits" fill="var(--color-commits)" radius={[4, 4, 0, 0]} maxBarSize={24} minPointSize={2} />
      </BarChart>
    </ChartContainer>
  );
}
