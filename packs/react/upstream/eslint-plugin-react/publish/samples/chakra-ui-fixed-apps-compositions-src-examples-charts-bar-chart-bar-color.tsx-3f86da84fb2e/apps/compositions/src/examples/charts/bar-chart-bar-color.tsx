"use client"

import { Chart, type UseChartReturn, useChart } from "@chakra-ui/charts"
import {
  Bar,
  BarChart,
  CartesianGrid,
  Rectangle,
  type RectangleProps,
  XAxis,
  YAxis,
} from "recharts"

interface Allocation {
  allocation: number
  type: string
  color: string
}

interface ColoredBarProps extends RectangleProps {
  chart: UseChartReturn<Allocation>
  payload?: Allocation
}

const ColoredBar = ({ chart, ...props }: ColoredBarProps) => (
  <Rectangle {...props} fill={chart.color(props.payload!.color)} />
)

export const BarChartBarColor = () => {
  const chart = useChart({
    data: [
      { allocation: 60, type: "Stock", color: "red.solid" },
      { allocation: 45, type: "Crypto", color: "blue.solid" },
      { allocation: 12, type: "ETF", color: "green.solid" },
      { allocation: 4, type: "Cash", color: "yellow.solid" },
    ],
  })

  return (
    <Chart.Root maxH="sm" chart={chart}>
      <BarChart data={chart.data} responsive>
        <CartesianGrid stroke={chart.color("border.muted")} vertical={false} />
        <XAxis axisLine={false} tickLine={false} dataKey={chart.key("type")} />
        <YAxis
          axisLine={false}
          tickLine={false}
          domain={[0, 100]}
          tickFormatter={(value) => `${value}%`}
        />
        <Bar
          isAnimationActive={false}
          dataKey={chart.key("allocation")}
          shape={<ColoredBar chart={chart} />}
        />
      </BarChart>
    </Chart.Root>
  )
}
